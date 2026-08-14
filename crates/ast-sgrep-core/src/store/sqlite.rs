use super::embed_support::{
    embed_cache_cap, embed_chunks, evict_embed_cache, init_cache_seq, insert_embed_cache_entries,
    read_sym_loc, requested_model_identity, structure_fingerprint, touch_embed_cache_entries,
    EmbeddedChunk, EmbeddedChunks,
};
use super::sql::configure_connection_with;
use super::sql::{
    append_lang_filter, calls_matching, count_star, delete_file_children, delete_file_lines,
    emb_vec, lang_and_clause, like_terms_filter, optional_row, query_cached_map, query_limit_map,
    query_map_rows, read_legacy_emb, read_sem_row, where_clause, CLEAR_ALL_SQL, SCHEMA_DDL,
};
use super::try_index_db_path;
use crate::{IndexStatus, Result};
use ast_sgrep_lang::PatternNode;
use rusqlite::types::{Type, ValueRef};
use rusqlite::{params, Connection, ToSql};
#[cfg(test)]
use std::cell::Cell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
#[cfg(test)]
thread_local! {
    /// Test-only inject for d2a1.2: force restore_synchronous to fail so
    /// callers prove commit/rollback surfaces the error (no `let _ =`).
    static FORCE_RESTORE_SYNC_FAILURE: Cell<bool> = const { Cell::new(false) };
    /// Force COMMIT to fail before it reaches SQLite so tests can verify that
    /// transaction cleanup does not depend on a successful commit.
    static FORCE_COMMIT_FAILURE: Cell<bool> = const { Cell::new(false) };
    /// Fail after write pragmas are admitted but before BEGIN so cleanup of a
    /// partially admitted FastUnsafe batch can be asserted deterministically.
    static FORCE_BEGIN_FAILURE: Cell<bool> = const { Cell::new(false) };
}
// 6 = symbols_name_lower. 7 = semantic-layout-v2 wipe. 8 = unstemmed code FTS.
// 9 = repository lexicon.
// Never reuse a SCHEMA_VERSION for two different migrations.
const SCHEMA_VERSION: i64 = 9;
const IMPORT_SELECT: &str =
    "SELECT f.path, f.language, i.module_path, i.line_no FROM imports i JOIN files f ON f.id = i.file_id";
const SYM_LOC: &str = "SELECT f.path, s.name, f.language, s.line_start, s.line_end FROM symbols s JOIN files f ON f.id = s.file_id";
pub type IndexedLineRow = (Arc<str>, u32, String, Option<Arc<str>>);
pub type ImportQueryRow = (String, Option<String>, String, u32);
pub type CallRow = (String, u32, String, String);

/// INTEGER byte offsets must fit `i64` on write (pb2w). Wrapping `as i64`
/// would store a negative and later wrap to `usize::MAX` on a 64-bit host.
fn sql_i64_from_byte_offset(value: usize) -> rusqlite::Result<i64> {
    i64::try_from(value).map_err(|_| {
        rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("byte offset {value} exceeds SQLite INTEGER storage"),
        )))
    })
}

/// Reject negative / out-of-range INTEGER byte offsets on read (pb2w).
fn sql_usize_from_byte_offset(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<usize> {
    let raw: i64 = row.get(idx)?;
    usize::try_from(raw).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(idx, raw))
}
pub struct PatternNodeRow {
    pub path: String,
    pub language: Option<String>,
    pub line_start: u32,
    pub line_end: u32,
    pub excerpt: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticChunkStats {
    pub count: usize,
    pub max_id: i64,
    pub dim: usize,
}
#[derive(Debug, Clone)]
pub struct SymbolRow {
    pub name: String,
    pub kind: String,
    pub line_start: u32,
    pub line_end: u32,
    pub byte_start: usize,
    pub byte_end: usize,
}
#[derive(Debug, Clone)]
pub struct CallerRow {
    pub caller: String,
    pub callee: String,
    pub line_no: u32,
    pub byte_start: usize,
    pub byte_end: usize,
}
#[derive(Debug, Clone)]
pub struct ImportRow {
    pub module_path: String,
    pub line_no: u32,
}
#[derive(Debug, Clone)]
pub struct SymbolLocationRow {
    pub path: String,
    pub name: String,
    pub language: Option<String>,
    pub line_start: u32,
    pub line_end: u32,
}
pub struct UpsertFileInput<'a> {
    pub rel_path: &'a str,
    pub language: Option<&'a str>,
    pub mtime_secs: i64,
    pub mtime_nanos: u32,
    pub content_hash: &'a str,
    pub lines: &'a [(u32, String)],
    pub eol: &'a str,
    pub symbols: &'a [SymbolRow],
    pub callers: &'a [CallerRow],
    pub imports: &'a [ImportRow],
    pub pattern_nodes: &'a [PatternNode],
    pub semantic_chunks: &'a [crate::semantic_chunk::SemanticChunkInput],
    pub embed_semantic: bool,
    pub embed_backend: ast_sgrep_embed::EmbedPreference,
}
pub struct RefreshLinesInput<'a> {
    pub file_id: i64,
    pub language: Option<&'a str>,
    pub mtime_secs: i64,
    pub mtime_nanos: u32,
    pub content_hash: &'a str,
    pub lines: &'a [(u32, String)],
    pub eol: &'a str,
    pub rel_path: &'a str,
}
pub struct IndexStore {
    conn: Connection,
    root: std::path::PathBuf,
    db_path: std::path::PathBuf,
    file_tx_depth: std::cell::Cell<u32>,
    file_tx_owns: std::cell::Cell<bool>,
    file_tx_poisoned: std::cell::Cell<bool>,
    bulk_tx_active: std::cell::Cell<bool>,
    bulk_tx_owns: std::cell::Cell<bool>,
    cache_seq: std::cell::Cell<i64>,
    /// Write-durability profile for this connection (0obi).
    durability: crate::store::Durability,
}
impl IndexStore {
    pub fn open(root: &Path, index_path: Option<&Path>) -> Result<Self> {
        Self::open_with_durability(root, index_path, crate::store::Durability::from_env())
    }

    /// Open with an explicit durability profile (0obi).
    pub fn open_with_durability(
        root: &Path,
        index_path: Option<&Path>,
        durability: crate::store::Durability,
    ) -> Result<Self> {
        let db_path = try_index_db_path(root, index_path).map_err(|e| {
            crate::StoreError::Other(format!(
                "failed to resolve index path for root {}: {e}",
                root.display()
            ))
        })?;
        if let Some(p) = db_path.parent() {
            std::fs::create_dir_all(p).map_err(|e| {
                crate::StoreError::Other(format!(
                    "failed to create index directory {} (root {}): {e}",
                    p.display(),
                    root.display()
                ))
            })?;
        }
        // Preserve rusqlite's error code so explicit reindex can distinguish a
        // corrupt/non-database file from permission, locking, and IO failures.
        let conn = Connection::open(&db_path)?;
        configure_connection_with(&conn, durability)?;
        let store = Self {
            conn,
            root: root.to_path_buf(),
            db_path,
            file_tx_depth: std::cell::Cell::new(0),
            file_tx_owns: std::cell::Cell::new(false),
            file_tx_poisoned: std::cell::Cell::new(false),
            bulk_tx_active: std::cell::Cell::new(false),
            bulk_tx_owns: std::cell::Cell::new(false),
            cache_seq: std::cell::Cell::new(0),
            durability,
        };
        store.init_schema()?;
        init_cache_seq(&store.conn, &store.cache_seq)?;
        Ok(store)
    }
    fn init_schema(&self) -> Result<()> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(crate::StoreError::Other(format!(
                "index schema version {version} is newer than supported version {SCHEMA_VERSION}; refusing to modify it"
            )));
        }
        if version >= SCHEMA_VERSION {
            // Probe core tables even when user_version is current (a639).
            let core: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('files','lines','meta','symbols')",
                [],
                |r| r.get(0),
            )?;
            if core >= 4 {
                return Ok(());
            }
            // Corrupt/partial schema with current user_version — rebuild.
        }
        // A legacy semantic sidecar is only a derived acceleration structure.
        // Remove it before migration; if the DB transaction then rolls back,
        // searches safely fall back rather than observing stale ANN contents.
        if version < 7 {
            crate::semantic_ivf::invalidate_semantic_ivf(&self.db_path)?;
        }
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let migration = (|| -> Result<()> {
            self.conn.execute_batch(SCHEMA_DDL)?;
            if version < 3 {
                self.conn.execute_batch(
                    "INSERT INTO lines_trigram(rowid, content) SELECT rowid, content FROM lines;",
                )?;
            }
            // Schema 8 (vvpk): backfill the unstemmed code field for older indexes.
            if version < 8 {
                self.conn.execute_batch(
                    "DELETE FROM lines_code_fts;
                 INSERT INTO lines_code_fts(rowid, content, file_id, line_no)
                   SELECT rowid, content, file_id, line_no FROM lines;",
                )?;
            }
            // Schema 6 (main): idx_symbols_name_lower arrives via SCHEMA_DDL above.
            // Schema 7: force re-embed under parent-mapped child layout (e2hc.6).
            if version < 7 {
                self.conn.execute_batch(
                    "DELETE FROM semantic_chunks;
                 DELETE FROM embeddings;
                 DELETE FROM embed_cache;
                 DELETE FROM meta WHERE key LIKE 'body:%' OR key LIKE 'struct:%'
                   OR key IN ('embed_backend', 'embed_model', 'embed_dim');
                 UPDATE files SET content_hash = 'semantic-layout-v2:' || content_hash
                   WHERE content_hash NOT LIKE 'semantic-layout-v2:%';",
                )?;
            }
            self.conn
                .execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
            Ok(())
        })();
        match migration {
            Ok(()) => self.conn.execute_batch("COMMIT")?,
            Err(error) => {
                if let Err(rollback_error) = self.conn.execute_batch("ROLLBACK") {
                    return Err(crate::StoreError::Other(format!(
                        "schema migration failed ({error}); rollback also failed: {rollback_error}"
                    )));
                }
                return Err(error);
            }
        }
        Ok(())
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
    pub fn connection(&self) -> &Connection {
        // Sealed for first-party use: external crates should prefer typed store
        // APIs (status/query helpers). Direct connection access remains for
        // in-tree search passes and integration tests that need prepared SQL
        // beyond the public facade (l115). Do not open a second connection to
        // the same db_path from agent surfaces.
        &self.conn
    }
    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.prepare_cached( "INSERT INTO meta(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )?.execute(params![key, value])?;
        Ok(())
    }
    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        optional_row(
            &self.conn,
            "SELECT value FROM meta WHERE key = ?1",
            &[&key],
            |r| r.get(0),
        )
    }
    pub fn delete_meta(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM meta WHERE key = ?1", params![key])?;
        Ok(())
    }
    /// Monotonic generation for searchable-index mutations on every connection.
    /// Monotonic index generation (d3l5).
    ///
    /// Every indexing transaction bumps `index_data_version`, so it is already
    /// the generation counter this engine needs; exposing it under the honest
    /// name avoids maintaining a second counter that could drift from the
    /// first.
    pub fn index_generation(&self) -> Result<i64> {
        self.index_data_version()
    }

    /// Schema version this database was built with (d3l5).
    pub fn schema_version(&self) -> i64 {
        SCHEMA_VERSION
    }

    /// Highest indexed file mtime, as a worktree revision proxy (d3l5).
    ///
    /// This is what the index believes about the worktree, so a response can
    /// state the source state it actually read rather than the current one.
    pub fn worktree_revision(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(MAX(mtime_secs), 0) FROM files",
            [],
            |row| row.get::<_, i64>(0),
        )?)
    }

    /// Count exact-name definitions by file in one query. The total count is
    /// returned alongside the map so caller resolution avoids per-hit SQL.
    pub(crate) fn symbol_name_candidate_counts(
        &self,
        name: &str,
    ) -> Result<(HashMap<String, usize>, usize)> {
        let mut statement = self.conn.prepare_cached(
            "SELECT f.path, COUNT(*) FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE s.name = ?1 GROUP BY f.path",
        )?;
        let rows = statement.query_map(params![name], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
        })?;
        let mut by_file = HashMap::new();
        let mut total = 0usize;
        for row in rows {
            let (file, count) = row?;
            total = total.saturating_add(count);
            by_file.insert(file, count);
        }
        Ok((by_file, total))
    }

    /// Visit bounded symbol context without buffering the whole repository.
    pub fn for_each_symbol_context(&self, mut visit: impl FnMut(&str, &str)) -> Result<()> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT s.name, COALESCE(GROUP_CONCAT(SUBSTR(l.content, 1, 1600), ' '), '')
             FROM symbols s
             LEFT JOIN lines l
               ON l.file_id = s.file_id
              AND l.line_no BETWEEN MAX(1, s.line_start - 2) AND s.line_start + 2
             GROUP BY s.id
             ORDER BY s.id
             LIMIT 100000",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (name, context) = row?;
            visit(&name, &context);
        }
        Ok(())
    }

    /// Remove learned associations as part of the caller's active transaction.
    /// The dirty marker suppresses repeated generation bumps in bulk indexing.
    pub(crate) fn invalidate_lexicon(&self) -> Result<()> {
        if self.get_meta("lexicon_dirty")?.as_deref() == Some("1") {
            return Ok(());
        }
        self.conn.execute("DELETE FROM lexicon", [])?;
        self.bump_meta_u64("lexicon_data_version", 1)?;
        self.set_meta("lexicon_dirty", "1")
    }

    pub(crate) fn lexicon_is_dirty(&self) -> Result<bool> {
        Ok(self.get_meta("lexicon_dirty")?.as_deref() == Some("1"))
    }

    /// Replace the repository lexicon in one transaction (ufk7).
    pub fn replace_lexicon(&self, associations: &[crate::lexicon::Association]) -> Result<()> {
        if associations.len() > crate::lexicon::MAX_PAIRS {
            return Err(crate::StoreError::Other(format!(
                "lexicon exceeds maximum of {} associations",
                crate::lexicon::MAX_PAIRS
            )));
        }
        if associations.iter().any(|association| {
            association.term.chars().count() > crate::lexicon::MAX_TERM_CHARS
                || association.related.chars().count() > crate::lexicon::MAX_TERM_CHARS
        }) {
            return Err(crate::StoreError::Other(format!(
                "lexicon term exceeds maximum of {} characters",
                crate::lexicon::MAX_TERM_CHARS
            )));
        }
        if associations
            .iter()
            .any(|association| !association.ppmi.is_finite())
        {
            return Err(crate::StoreError::Other(
                "lexicon contains a non-finite score".into(),
            ));
        }
        self.with_file_tx(|| {
            self.conn.execute("DELETE FROM lexicon", [])?;
            {
                let mut stmt = self.conn.prepare_cached(
                    "INSERT OR REPLACE INTO lexicon(term, related, ppmi, support) VALUES(?1,?2,?3,?4)",
                )?;
                for association in associations {
                    stmt.execute(params![
                        association.term,
                        association.related,
                        association.ppmi,
                        association.support
                    ])?;
                }
            }
            self.delete_meta("lexicon_dirty")?;
            self.bump_meta_u64("lexicon_data_version", 1)
        })
    }

    /// Read a bounded lexicon (ufk7). Ordered so loads are deterministic.
    pub fn all_lexicon_rows(&self) -> Result<Vec<crate::lexicon::Association>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT SUBSTR(term, 1, ?1), SUBSTR(related, 1, ?1), ppmi, support,
                    LENGTH(term), LENGTH(related)
             FROM lexicon
             ORDER BY term, related
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            params![
                crate::lexicon::MAX_TERM_CHARS as i64,
                crate::lexicon::MAX_PAIRS as i64 + 1
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )?;
        let mut out = Vec::with_capacity(crate::lexicon::MAX_PAIRS.min(1024));
        for row in rows {
            let (term, related, ppmi, support, term_chars, related_chars) = row?;
            if term_chars > crate::lexicon::MAX_TERM_CHARS as i64
                || related_chars > crate::lexicon::MAX_TERM_CHARS as i64
            {
                return Err(crate::StoreError::Other(format!(
                    "stored lexicon term exceeds maximum of {} characters",
                    crate::lexicon::MAX_TERM_CHARS
                )));
            }
            if !ppmi.is_finite() {
                return Err(crate::StoreError::Other(
                    "stored lexicon contains a non-finite score".into(),
                ));
            }
            let support = u32::try_from(support).map_err(|_| {
                crate::StoreError::Other("stored lexicon support is out of range".into())
            })?;
            out.push(crate::lexicon::Association {
                term,
                related,
                ppmi,
                support,
            });
        }
        if out.len() > crate::lexicon::MAX_PAIRS {
            return Err(crate::StoreError::Other(format!(
                "stored lexicon exceeds maximum of {} associations",
                crate::lexicon::MAX_PAIRS
            )));
        }
        Ok(out)
    }

    pub fn index_data_version(&self) -> Result<i64> {
        Ok(self
            .get_meta("index_data_version")?
            .and_then(|value| value.parse().ok())
            .unwrap_or(0))
    }

    /// Indexed-content and lexicon generations used by long-lived search caches.
    pub(crate) fn search_data_versions(&self) -> Result<(i64, i64)> {
        Ok(self.conn.query_row(
            "SELECT
               COALESCE(MAX(CASE WHEN key = 'index_data_version' THEN CAST(value AS INTEGER) END), 0),
               COALESCE(MAX(CASE WHEN key = 'lexicon_data_version' THEN CAST(value AS INTEGER) END), 0)
             FROM meta
             WHERE key IN ('index_data_version', 'lexicon_data_version')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
    }
    /// True when the index was built with the legacy `"semantic"` embed backend.
    /// Search refuses this meta; indexing must rewrite every chunk before
    /// promoting to `"semantic-v2"` (semantic_v1_rewrite contract).
    pub fn needs_semantic_v1_rewrite(&self) -> Result<bool> {
        Ok(self.get_meta("embed_backend")?.as_deref() == Some("semantic"))
    }
    /// Start a proven-complete semantic rewrite inside the caller's bulk
    /// transaction. Old vectors are removed before the first upsert so that
    /// its resolved provider/model can establish the store-wide identity;
    /// every later upsert must match it or the bulk transaction rolls back.
    pub(crate) fn reset_semantic_index_for_rewrite(&self) -> Result<()> {
        self.conn.execute("DELETE FROM semantic_chunks", [])?;
        self.conn.execute("DELETE FROM embeddings", [])?;
        for key in [
            "embed_backend",
            "embed_backend_pref",
            "embed_model",
            "embed_dim",
        ] {
            self.delete_meta(key)?;
        }
        crate::semantic_ann::mark_semantic_ivf_stale(self)?;
        self.bump_semantic_data_version()
    }
    fn bump_index_data_version(&self) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES('index_data_version', '1')              ON CONFLICT(key) DO UPDATE SET value =              CAST(COALESCE(meta.value, '0') AS INTEGER) + 1",
            [],
        )?;
        Ok(())
    }
    /// Monotonic counter bumped on every semantic_chunks mutation (insert or delete).
    /// Used by SemanticCache and the IVF fingerprint to detect delete+re-add
    /// collisions where max_id is reused but chunk content/vectors differ.
    /// See bead ast-sgrep-44a4 (F-02).
    pub fn semantic_data_version(&self) -> Result<i64> {
        Ok(self
            .get_meta("semantic_data_version")?
            .and_then(|s| s.parse().ok())
            .unwrap_or(0))
    }
    pub fn bump_semantic_data_version(&self) -> Result<()> {
        let v = self.semantic_data_version()?.saturating_add(1);
        self.set_meta("semantic_data_version", &v.to_string())
    }
    pub fn file_id(&self, rel_path: &str) -> Result<Option<i64>> {
        optional_row(
            &self.conn,
            "SELECT id FROM files WHERE path = ?1",
            &[&rel_path],
            |r| r.get(0),
        )
    }
    /// File-tx stays OFF until bulk commit (no re-NORMAL after each file).
    /// Nested begins only increment depth; only the owning outermost end commits
    /// or rolls back (bead ast-sgrep-j97d.37er).
    /// Active write-durability profile (0obi).
    pub fn durability(&self) -> crate::store::Durability {
        self.durability
    }

    pub fn begin_file_tx(&self) -> Result<()> {
        let depth = self.file_tx_depth.get();
        if depth == 0 {
            self.file_tx_poisoned.set(false);
            if self.conn.is_autocommit() {
                // 0obi: was unconditionally OFF; now the profile decides.
                self.begin_owned_transaction(&format!(
                    "PRAGMA synchronous = {}",
                    self.durability.write_pragma()
                ))?;
                self.file_tx_owns.set(true);
            } else {
                // Bulk (or other) transaction owns the write set.
                self.file_tx_owns.set(false);
            }
        }
        self.file_tx_depth.set(depth + 1);
        Ok(())
    }
    pub fn commit_file_tx(&self) -> Result<()> {
        self.end_file_tx(true)
    }
    pub fn rollback_file_tx(&self) -> Result<()> {
        self.end_file_tx(false)
    }
    fn restore_synchronous(&self) -> Result<()> {
        #[cfg(test)]
        if FORCE_RESTORE_SYNC_FAILURE.with(|c| c.get()) {
            return Err(crate::StoreError::Other(
                "restore_synchronous forced failure (test inject)".into(),
            ));
        }
        self.conn.execute_batch(&format!(
            "PRAGMA synchronous = {}; PRAGMA cache_size = -16384",
            self.durability.steady_pragma()
        ))?;
        Ok(())
    }
    /// Apply write-mode pragmas and acquire a transaction as one admission.
    /// If acquisition fails after (for example) FastUnsafe selected
    /// `synchronous=OFF`, restore the steady profile before returning.
    fn begin_owned_transaction(&self, setup: &str) -> Result<()> {
        let start = (|| -> Result<()> {
            self.conn.execute_batch(setup)?;
            #[cfg(test)]
            if FORCE_BEGIN_FAILURE.with(|c| c.get()) {
                return Err(crate::StoreError::Other(
                    "BEGIN forced failure (test inject)".into(),
                ));
            }
            self.conn.execute_batch("BEGIN IMMEDIATE")?;
            Ok(())
        })();
        let Err(start_error) = start else {
            return Ok(());
        };

        if !self.conn.is_autocommit() {
            if let Err(rollback_error) = self.conn.execute_batch("ROLLBACK") {
                return Err(crate::StoreError::Other(format!(
                    "transaction admission failed ({start_error}); cleanup ROLLBACK failed: {rollback_error}"
                )));
            }
        }
        // Fail visible if cleanup itself cannot restore a safe steady profile.
        self.restore_synchronous()?;
        Err(start_error)
    }
    fn execute_transaction_end(&self, sql: &str) -> Result<()> {
        #[cfg(test)]
        if sql == "COMMIT" && FORCE_COMMIT_FAILURE.with(|c| c.get()) {
            return Err(crate::StoreError::Other(
                "COMMIT forced failure (test inject)".into(),
            ));
        }
        self.conn.execute_batch(sql)?;
        Ok(())
    }
    /// End a transaction owned by this store and restore its steady-state
    /// connection settings. A failed COMMIT is followed by a best-effort
    /// ROLLBACK before any error is returned, so callers never inherit an open
    /// transaction merely because commit failed.
    fn finish_owned_transaction(&self, commit: bool) -> Result<()> {
        let mut tx_error = self
            .execute_transaction_end(if commit { "COMMIT" } else { "ROLLBACK" })
            .err();

        if commit && tx_error.is_some() && !self.conn.is_autocommit() {
            let commit_error = tx_error.take().expect("checked above");
            tx_error = match self.execute_transaction_end("ROLLBACK") {
                Ok(()) => Some(commit_error),
                Err(rollback_error) => Some(crate::StoreError::Other(format!(
                    "COMMIT failed: {commit_error}; cleanup ROLLBACK failed: {rollback_error}"
                ))),
            };
        }

        if !self.conn.is_autocommit() {
            let cleanup_error = crate::StoreError::Other(
                "transaction cleanup failed: SQLite transaction remains active".into(),
            );
            return Err(match tx_error {
                Some(error) => crate::StoreError::Other(format!("{error}; {cleanup_error}")),
                None => cleanup_error,
            });
        }

        // Preserve the existing fail-visible policy: a stuck write-batch mode
        // is operationally more dangerous than the transaction error that led
        // to cleanup, so restoration errors take precedence.
        self.restore_synchronous()?;
        match tx_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    fn end_file_tx(&self, commit: bool) -> Result<()> {
        let depth = self.file_tx_depth.get();
        if depth == 0 {
            return Ok(());
        }
        if !commit {
            self.file_tx_poisoned.set(true);
        }
        if depth > 1 {
            // Nested end: never COMMIT/ROLLBACK the outer transaction here.
            self.file_tx_depth.set(depth - 1);
            return Ok(());
        }
        let owns = self.file_tx_owns.get();
        let poisoned = self.file_tx_poisoned.get();
        let transaction_result = owns.then(|| self.finish_owned_transaction(commit && !poisoned));
        // Clear bookkeeping before propagating any transaction/restore error so
        // a failed end cannot leave stale state that confuses the next begin.
        self.file_tx_depth.set(0);
        self.file_tx_owns.set(false);
        self.file_tx_poisoned.set(false);
        if let Some(result) = transaction_result {
            result?;
        }
        if poisoned && commit {
            return Err(crate::StoreError::Other(
                "file_tx commit refused: nested file_tx rolled back".into(),
            ));
        }
        Ok(())
    }
    pub(crate) fn with_file_tx<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.begin_file_tx()?;
        match f() {
            Ok(v) => {
                // Nested rollback poisons the write set. Do not return Ok(v) after
                // rolling back: callers would treat a failed nested write as success
                // (e.g. file_id that no longer exists). Match commit_file_tx refusal.
                if self.file_tx_poisoned.get() {
                    self.rollback_file_tx()?;
                    return Err(crate::StoreError::Other(
                        "file_tx commit refused: nested file_tx rolled back".into(),
                    ));
                }
                self.commit_file_tx()?;
                Ok(v)
            }
            Err(e) => {
                self.rollback_file_tx()?;
                Err(e)
            }
        }
    }
    fn meta_u64(&self, key: &str) -> Result<u64> {
        Ok(self
            .get_meta(key)?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0))
    }
    fn bump_meta_u64(&self, key: &str, delta: usize) -> Result<()> {
        if delta == 0 {
            return Ok(());
        }
        let total = self.meta_u64(key)?.saturating_add(delta as u64);
        self.set_meta(key, &total.to_string())
    }
    pub fn begin_bulk_tx(&self) -> Result<()> {
        if self.bulk_tx_active.get() {
            return Ok(());
        }
        if self.conn.is_autocommit() {
            // 0obi: was unconditionally OFF; now the profile decides.
            self.begin_owned_transaction(&format!(
                "PRAGMA temp_store = MEMORY; PRAGMA cache_size = -131072; PRAGMA mmap_size = 536870912; \
                 PRAGMA synchronous = {}",
                self.durability.write_pragma()
            ))?;
            self.bulk_tx_owns.set(true);
        } else {
            self.bulk_tx_owns.set(false);
        }
        self.bulk_tx_active.set(true);
        Ok(())
    }
    pub fn commit_bulk_tx(&self) -> Result<()> {
        self.end_bulk_tx(true)
    }
    pub fn rollback_bulk_tx(&self) -> Result<()> {
        self.end_bulk_tx(false)
    }
    /// Finish a bulk write started with [`begin_bulk_tx`].
    ///
    /// On `Ok`, commits. On `Err`, rolls back and **prefers** the rollback/restore
    /// error so a stuck FastUnsafe `synchronous=OFF` mode cannot hide behind the
    /// original write failure (d2a1.2 residual / pass9: no `let _ = rollback`).
    pub fn apply_bulk_write_result(&self, write_result: Result<()>) -> Result<()> {
        match write_result {
            Ok(()) => self.commit_bulk_tx(),
            Err(e) => match self.rollback_bulk_tx() {
                Ok(()) => Err(e),
                Err(rb) => Err(rb),
            },
        }
    }
    fn end_bulk_tx(&self, commit: bool) -> Result<()> {
        if !self.bulk_tx_active.get() {
            return Ok(());
        }
        let owns = self.bulk_tx_owns.get();
        let transaction_result = owns.then(|| self.finish_owned_transaction(commit));
        self.bulk_tx_active.set(false);
        self.bulk_tx_owns.set(false);
        self.file_tx_depth.set(0);
        self.file_tx_owns.set(false);
        self.file_tx_poisoned.set(false);
        match transaction_result {
            Some(result) => result,
            None => Ok(()),
        }
    }
    pub fn clear_all_data(&self) -> Result<()> {
        // Bump both generations inside the same file_tx as the wipe so a crash
        // between COMMIT and a post-tx bump cannot leave empty tables with a
        // stale semantic_data_version (pass3).
        self.with_file_tx(|| {
            self.conn.execute_batch(CLEAR_ALL_SQL)?;
            self.bump_index_data_version()?;
            self.bump_semantic_data_version()?;
            self.bump_meta_u64("lexicon_data_version", 1)?;
            self.set_meta("lexicon_dirty", "1")?;
            crate::semantic_ann::mark_semantic_ivf_stale(self)
        })?;
        let _ = self.conn.execute_batch("VACUUM");
        Ok(())
    }
    pub fn upsert_file(&self, input: UpsertFileInput<'_>) -> Result<i64> {
        let requested_identity = input
            .embed_semantic
            .then(|| requested_model_identity(input.embed_backend));
        let struct_fp = structure_fingerprint(
            input.symbols,
            input.callers,
            input.imports,
            input.pattern_nodes,
            input.semantic_chunks,
            requested_identity.as_deref(),
        );
        let struct_key = format!("struct:{}", input.rel_path);
        let embed_identity_matches = !input.embed_semantic
            || self.get_meta("embed_model")?.as_deref() == requested_identity.as_deref();
        if let Some(file_id) = self.file_id(input.rel_path)? {
            if embed_identity_matches
                && self.get_meta(&struct_key)?.as_deref() == Some(struct_fp.as_str())
            {
                return self.with_file_tx(|| {
                    self.refresh_lines_only(RefreshLinesInput {
                        file_id,
                        language: input.language,
                        mtime_secs: input.mtime_secs,
                        mtime_nanos: input.mtime_nanos,
                        content_hash: input.content_hash,
                        lines: input.lines,
                        eol: input.eol,
                        rel_path: input.rel_path,
                    })
                });
            }
        }
        let emb = embed_chunks(
            &self.conn,
            input.semantic_chunks,
            input.embed_semantic,
            input.embed_backend,
        )?;
        let (cache_hits, cache_misses) = (
            emb.cache_hits.len(),
            emb.chunks.len().saturating_sub(emb.cache_hits.len()),
        );
        self.with_file_tx(|| {
            let id = self.upsert_file_inner(input, &emb.chunks, &struct_key, &struct_fp)?;
            self.persist_embed_cache_side_effects(&emb, cache_hits, cache_misses)?;
            Ok(id)
        })
    }
    fn persist_embed_cache_side_effects(
        &self,
        emb: &EmbeddedChunks,
        cache_hits: usize,
        cache_misses: usize,
    ) -> Result<()> {
        if !emb.cache_entries.is_empty() {
            if let Err(e) =
                insert_embed_cache_entries(&self.conn, &self.cache_seq, &emb.cache_entries)
            {
                eprintln!("[asgrep] warning: failed to write embedding cache: {e}");
            }
            if let Err(e) = evict_embed_cache(&self.conn, embed_cache_cap()) {
                eprintln!("[asgrep] warning: failed to evict embedding cache: {e}");
            }
        } else if !emb.cache_hits.is_empty() {
            let hits: Vec<_> = emb
                .cache_hits
                .iter()
                .map(|h| (h.chunk_hash.clone(), h.model_id.clone()))
                .collect();
            if let Err(e) = touch_embed_cache_entries(&self.conn, &self.cache_seq, &hits) {
                eprintln!("[asgrep] warning: failed to touch embedding cache: {e}");
            }
        }
        self.bump_meta_u64("embed_cache_hits", cache_hits)?;
        self.bump_meta_u64("embed_cache_misses", cache_misses)?;
        Ok(())
    }
    /// Lines/FTS only when structure fingerprint matches (append / truncate / full rewrite).
    pub fn refresh_lines_only(&self, input: RefreshLinesInput<'_>) -> Result<i64> {
        let RefreshLinesInput {
            file_id,
            language: lang,
            mtime_secs,
            mtime_nanos,
            content_hash: hash,
            lines,
            eol,
            rel_path,
        } = input;
        let existing: Vec<(u32, String)> = {
            let mut stmt = self.conn.prepare_cached(
                "SELECT line_no, content FROM lines WHERE file_id = ?1 ORDER BY line_no",
            )?;
            let rows = stmt.query_map(params![file_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let common = existing
            .iter()
            .zip(lines.iter())
            .take_while(|(a, b)| a.1 == b.1)
            .count();
        if common == existing.len() && lines.len() >= existing.len() {
            // Append trailing lines — must keep lines_fts AND lines_trigram in sync
            // (literal BMH path uses trigram when indexed lines ≥ 1000).
            let extra = &lines[common..];
            if !extra.is_empty() {
                self.insert_lines(file_id, extra)?;
            }
        } else if common == lines.len() && existing.len() > lines.len() {
            // Truncate trailing lines: drop FTS + content=trigram rowids before lines.
            delete_file_lines(&self.conn, file_id, Some(lines.len() as u32 + 1))?;
        } else {
            delete_file_lines(&self.conn, file_id, None)?;
            self.insert_lines(file_id, lines)?;
        }
        self.conn
            .prepare_cached(
                "UPDATE files SET language=?1, mtime_secs=?2, mtime_nanos=?3, content_hash=?4 WHERE id=?5",
            )?
            .execute(params![lang, mtime_secs, mtime_nanos, hash, file_id])?;
        self.set_meta(&format!("eol:{rel_path}"), eol)?;
        self.bump_index_data_version()?;
        self.invalidate_lexicon()?;
        Ok(file_id)
    }
    fn upsert_file_inner(
        &self,
        input: UpsertFileInput<'_>,
        emb: &[EmbeddedChunk],
        struct_key: &str,
        struct_fp: &str,
    ) -> Result<i64> {
        let file_id = self.upsert_file_row(
            input.rel_path,
            input.language,
            input.mtime_secs,
            input.mtime_nanos,
            input.content_hash,
        )?;
        self.insert_lines(file_id, input.lines)?;
        self.set_meta(&format!("eol:{}", input.rel_path), input.eol)?;
        let symbol_ids = self.insert_symbols(file_id, input.symbols)?;
        self.insert_semantic_chunks(
            file_id,
            input.symbols,
            &symbol_ids,
            input.semantic_chunks,
            emb,
            input.embed_backend,
        )?;
        self.insert_callers(file_id, input.callers)?;
        self.insert_imports(file_id, input.imports)?;
        self.insert_pattern_nodes(file_id, input.pattern_nodes)?;
        self.set_meta(struct_key, struct_fp)?;
        crate::semantic_ann::mark_semantic_ivf_stale(self)?;
        self.bump_index_data_version()?;
        self.invalidate_lexicon()?;
        Ok(file_id)
    }
    fn upsert_file_row(
        &self,
        path: &str,
        lang: Option<&str>,
        mtime_secs: i64,
        mtime_nanos: u32,
        hash: &str,
    ) -> Result<i64> {
        if let Some(id) = self.file_id(path)? {
            delete_file_children(&self.conn, id)?;
            self.conn.prepare_cached(
                "UPDATE files SET language=?1, mtime_secs=?2, mtime_nanos=?3, content_hash=?4 WHERE id=?5",
            )?.execute(params![lang, mtime_secs, mtime_nanos, hash, id])?;
            return Ok(id);
        }
        self.conn.prepare_cached( "INSERT INTO files(path, language, mtime_secs, mtime_nanos, content_hash) VALUES(?1,?2,?3,?4,?5)",
        )?.execute(params![path, lang, mtime_secs, mtime_nanos, hash])?;
        Ok(self.conn.last_insert_rowid())
    }
    /// Prepare a cached INSERT and bind each row. SQL text must remain byte-identical.
    fn insert_each<T>(
        &self,
        sql: &'static str,
        rows: &[T],
        mut bind: impl FnMut(&mut rusqlite::CachedStatement<'_>, &T) -> rusqlite::Result<()>,
    ) -> Result<()> {
        let mut st = self.conn.prepare_cached(sql)?;
        for row in rows {
            bind(&mut st, row)?;
        }
        Ok(())
    }

    fn insert_lines(&self, file_id: i64, lines: &[(u32, String)]) -> Result<()> {
        let mut ls = self
            .conn
            .prepare_cached("INSERT INTO lines(file_id, line_no, content) VALUES(?1,?2,?3)")?;
        let mut fts = self.conn.prepare_cached(
            "INSERT INTO lines_fts(rowid, content, file_id, line_no) VALUES(?1,?2,?3,?4)",
        )?;
        // vvpk: the same line also lands in the unstemmed code field.
        let mut code_fts = self.conn.prepare_cached(
            "INSERT INTO lines_code_fts(rowid, content, file_id, line_no) VALUES(?1,?2,?3,?4)",
        )?;
        let mut tri = self
            .conn
            .prepare_cached("INSERT INTO lines_trigram(rowid, content) VALUES(?1,?2)")?;
        for (no, content) in lines {
            ls.execute(params![file_id, no, content])?;
            let rid = self.conn.last_insert_rowid();
            fts.execute(params![rid, content, file_id, no])?;
            code_fts.execute(params![rid, content, file_id, no])?;
            tri.execute(params![rid, content])?;
        }
        Ok(())
    }

    fn insert_symbols(&self, file_id: i64, symbols: &[SymbolRow]) -> Result<Vec<i64>> {
        let mut ids = Vec::with_capacity(symbols.len());
        let mut st = self.conn.prepare_cached(
            "INSERT INTO symbols(file_id, name, kind, line_start, line_end, byte_start, byte_end) VALUES(?1,?2,?3,?4,?5,?6,?7)", )?;
        for s in symbols {
            st.execute(params![
                file_id,
                s.name,
                s.kind,
                s.line_start,
                s.line_end,
                sql_i64_from_byte_offset(s.byte_start)?,
                sql_i64_from_byte_offset(s.byte_end)?
            ])?;
            ids.push(self.conn.last_insert_rowid());
        }
        Ok(ids)
    }
    fn insert_semantic_chunks(
        &self,
        file_id: i64,
        symbols: &[SymbolRow],
        symbol_ids: &[i64],
        chunks: &[crate::semantic_chunk::SemanticChunkInput],
        emb: &[EmbeddedChunk],
        preference: ast_sgrep_embed::EmbedPreference,
    ) -> Result<()> {
        if emb.is_empty() {
            // A re-upsert of an EXISTING file reaches here AFTER upsert_file_row's
            // delete_file_children already removed its old semantic_chunks. Bump so
            // SemanticCache + IVF fingerprint detect the mutation (bead ast-sgrep-44a4).
            // Benign over-bump when the file never had chunks (new file, no chunks):
            // an extra cache miss, never a stale hit.
            self.bump_semantic_data_version()?;
            return Ok(());
        }
        if emb.len() != chunks.len() {
            return Err(crate::StoreError::Other(format!(
                "embedding result count {} does not match semantic child count {}",
                emb.len(),
                chunks.len()
            )));
        }
        let first = &emb[0];
        if emb
            .iter()
            .any(|entry| entry.backend != first.backend || entry.dim != first.dim)
        {
            return Err(crate::StoreError::Other(
                "embedding provider returned mixed backend or dimension identities".into(),
            ));
        }
        let model = ast_sgrep_embed::configured_backend_model_id(first.backend, first.dim)
            .ok_or_else(|| {
                crate::StoreError::Other(format!(
                    "resolved {:?} embedding backend has no configured model identity",
                    first.backend
                ))
            })?;
        let (siblings, stored_backend, stored_model, stored_dim): (
            bool,
            Option<String>,
            Option<String>,
            Option<String>,
        ) = self.conn.query_row(
            "SELECT
               EXISTS(SELECT 1 FROM semantic_chunks WHERE file_id != ?1),
               (SELECT value FROM meta WHERE key = 'embed_backend'),
               (SELECT value FROM meta WHERE key = 'embed_model'),
               (SELECT value FROM meta WHERE key = 'embed_dim')",
            params![file_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        if siblings {
            let backend_matches = stored_backend.as_deref().is_some_and(|stored| {
                ast_sgrep_embed::EmbedBackendKind::parse(stored) == Some(first.backend)
            });
            if !backend_matches
                || stored_dim.as_deref().and_then(|dim| dim.parse().ok()) != Some(first.dim)
                || stored_model.as_deref() != Some(model.as_str())
            {
                return Err(crate::StoreError::Other(format!(
                    "resolved embedding identity {:?}/{model}/{} does not match existing repository vectors; run `asgrep reindex`",
                    first.backend, first.dim
                )));
            }
        }
        let name_to_id: HashMap<String, i64> = symbols
            .iter()
            .zip(symbol_ids)
            .map(|(s, id)| (format!("{}:{}", s.name, s.line_start), *id))
            .collect();
        let mut st = self.conn.prepare_cached(
            "INSERT INTO semantic_chunks(file_id, symbol_id, chunk_kind, line_start, line_end, symbol_name, text, vector) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)", )?;
        for (c, e) in chunks.iter().zip(emb.iter()) {
            let sid = name_to_id
                .get(&format!("{}:{}", c.symbol_name, c.line_start))
                .copied();
            st.execute(params![
                file_id,
                sid,
                if c.kind == "file" { "file" } else { "symbol" },
                c.line_start,
                c.line_end,
                c.symbol_name,
                c.excerpt,
                e.vector_bytes
            ])?;
        }
        self.persist_embed_metadata(Some(first.dim), Some(first.backend), preference)
    }
    fn persist_embed_metadata(
        &self,
        dim: Option<usize>,
        kind: Option<ast_sgrep_embed::EmbedBackendKind>,
        preference: ast_sgrep_embed::EmbedPreference,
    ) -> Result<()> {
        if let Some(k) = kind {
            // embed_backend always stores the RESOLVED kind (search loads
            // vectors by it). The build-time preference lives separately so
            // identity checks can distinguish an Auto build (reopen no-ops
            // under Auto) from an explicit one (28vo exactness) — parity.
            // e2hc.13: a partial (single-file) update must not promote a
            // legacy v1 store (meta == "semantic") to v2 while sibling chunks
            // remain v1 — only a full index_all rewrite may.
            if self.get_meta("embed_backend")?.as_deref() != Some("semantic") {
                self.set_meta("embed_backend", k.as_meta_str())?;
                if preference == ast_sgrep_embed::EmbedPreference::Auto {
                    self.set_meta("embed_backend_pref", "auto")?;
                } else {
                    self.delete_meta("embed_backend_pref")?;
                }
            }
            let model = dim.and_then(|dim| ast_sgrep_embed::configured_backend_model_id(k, dim));
            if let Some(model) = model {
                self.set_meta("embed_model", &model)?;
            } else {
                self.delete_meta("embed_model")?;
            }
        }
        if let Some(d) = dim {
            self.set_meta("embed_dim", &d.to_string())?;
        }
        // Bump semantic_data_version on every chunk insertion so SemanticCache and
        // the IVF fingerprint detect delete+re-add collisions (bead ast-sgrep-44a4).
        self.bump_semantic_data_version()?;
        Ok(())
    }
    fn insert_callers(&self, file_id: i64, callers: &[CallerRow]) -> Result<()> {
        self.insert_each(
            "INSERT INTO callers(file_id, caller, callee, line_no, byte_start, byte_end) VALUES(?1,?2,?3,?4,?5,?6)",
            callers,
            |st, c| {
                st.execute(params![
                    file_id,
                    c.caller,
                    c.callee,
                    c.line_no,
                    sql_i64_from_byte_offset(c.byte_start)?,
                    sql_i64_from_byte_offset(c.byte_end)?
                ])?;
                Ok(())
            },
        )
    }
    fn insert_pattern_nodes(&self, file_id: i64, nodes: &[PatternNode]) -> Result<()> {
        self.insert_each(
            "INSERT INTO pattern_nodes(file_id, signature, line_start, line_end, excerpt) VALUES(?1,?2,?3,?4,?5)",
            nodes,
            |st, n| {
                st.execute(params![
                    file_id,
                    n.signature,
                    n.line_start,
                    n.line_end,
                    n.excerpt
                ])?;
                Ok(())
            },
        )
    }
    fn insert_imports(&self, file_id: i64, imports: &[ImportRow]) -> Result<()> {
        self.insert_each(
            "INSERT INTO imports(file_id, module_path, line_no) VALUES(?1,?2,?3)",
            imports,
            |st, i| {
                st.execute(params![file_id, i.module_path, i.line_no])?;
                Ok(())
            },
        )
    }
    pub fn remove_file(&self, rel_path: &str) -> Result<()> {
        self.with_file_tx(|| {
            let Some(id) = self.file_id(rel_path)? else {
                return Ok(());
            };
            delete_file_children(&self.conn, id)?;
            self.conn
                .execute("DELETE FROM files WHERE id = ?1", params![id])?;
            self.delete_meta(&format!("eol:{rel_path}"))?;
            self.delete_meta(&format!("body:{rel_path}"))?;
            self.delete_meta(&format!("struct:{rel_path}"))?;
            crate::semantic_ann::mark_semantic_ivf_stale(self)?;
            self.bump_index_data_version()?;
            self.bump_semantic_data_version()?;
            self.invalidate_lexicon()?;
            Ok(())
        })
    }
    pub fn file_hash(&self, rel_path: &str) -> Result<Option<String>> {
        optional_row(
            &self.conn,
            "SELECT content_hash FROM files WHERE path = ?1",
            &[&rel_path],
            |r| r.get(0),
        )
    }
    pub fn all_file_paths(&self) -> Result<Vec<String>> {
        query_cached_map(
            &self.conn,
            "SELECT path FROM files ORDER BY path",
            [],
            |r| r.get(0),
        )
    }
    pub(crate) fn has_file_with_prefix(&self, prefix: &str) -> Result<bool> {
        let pattern = format!("{}*", super::sql::escape_glob_literal(prefix));
        Ok(self
            .conn
            .prepare_cached("SELECT 1 FROM files WHERE path GLOB ?1 LIMIT 1")?
            .exists(params![pattern])?)
    }
    pub(crate) fn remove_files_with_prefix(&self, prefix: &str) -> Result<usize> {
        let pattern = format!("{}*", super::sql::escape_glob_literal(prefix));
        let paths: Vec<String> = query_cached_map(
            &self.conn,
            "SELECT path FROM files WHERE path GLOB ?1 ORDER BY path",
            params![pattern],
            |row| row.get(0),
        )?;
        if paths.is_empty() {
            return Ok(0);
        }
        self.with_file_tx(|| {
            for path in &paths {
                self.remove_file(path)?;
            }
            Ok(paths.len())
        })
    }
    pub fn status(&self) -> Result<IndexStatus> {
        let (fc, lc, sc, cc, ic, sec): (usize, usize, usize, usize, usize, usize) = self.conn.query_row(
            "SELECT (SELECT COUNT(*) FROM files),(SELECT COUNT(*) FROM lines),(SELECT COUNT(*) FROM symbols),\
             (SELECT COUNT(*) FROM callers),(SELECT COUNT(*) FROM imports),(SELECT COUNT(*) FROM semantic_chunks)",
            [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)), )?;
        Ok(IndexStatus {
            root: self.root.display().to_string(),
            index_path: self.db_path.display().to_string(),
            file_count: fc,
            line_count: lc,
            symbol_count: sc,
            caller_count: cc,
            import_count: ic,
            semantic_chunk_count: sec,
            embed_backend: self.get_meta("embed_backend")?,
            embed_dim: self.get_meta("embed_dim")?.and_then(|d| d.parse().ok()),
            embed_cache_entries: count_star(&self.conn, "embed_cache")?,
            embed_cache_capacity: embed_cache_cap(),
            embed_cache_hits: self.meta_u64("embed_cache_hits")?,
            embed_cache_misses: self.meta_u64("embed_cache_misses")?,
            semantic_ivf_present: crate::semantic_ivf::semantic_ivf_path(&self.db_path).exists(),
            durability: self.durability.as_str().to_owned(),
            writer_generation: crate::store::read_writer_generation(
                &self.root,
                Some(&self.db_path),
            ),
        })
    }
    pub fn indexed_line_count(&self) -> Result<usize> {
        count_star(&self.conn, "lines")
    }
    /// True when indexed lines ≥ threshold (LIMIT probe; avoids full COUNT).
    pub fn indexed_line_count_at_least(&self, threshold: usize) -> Result<bool> {
        super::sql::at_least_rows(&self.conn, "lines", threshold)
    }
    pub fn all_indexed_lines(&self) -> Result<Vec<IndexedLineRow>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT f.path, l.line_no, l.content, f.language FROM lines l JOIN files f ON f.id = l.file_id ORDER BY f.path, l.line_no")?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        let mut last_path: Option<Arc<str>> = None;
        let mut last_lang: Option<Arc<str>> = None;
        while let Some(row) = rows.next()? {
            let path = row.get_ref(0)?.as_str().map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(e))
            })?;
            if last_path.as_deref() != Some(path) {
                last_path = Some(Arc::from(path));
                last_lang = match row.get_ref(3)? {
                    ValueRef::Null => None,
                    v => Some(Arc::from(v.as_str().map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(e))
                    })?)),
                };
            }
            out.push((
                Arc::clone(last_path.as_ref().expect("path")),
                row.get(1)?,
                row.get(2)?,
                last_lang.clone(),
            ));
        }
        Ok(out)
    }
    pub(crate) fn indexed_excerpt_in_range(
        &self,
        path: &str,
        line_start: u32,
        line_end: u32,
    ) -> Result<String> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT CAST(substr(CAST(l.content AS BLOB), 1, ?4) AS BLOB),
                    length(CAST(l.content AS BLOB))
             FROM lines l JOIN files f ON f.id = l.file_id
             WHERE f.path = ?1 AND l.line_no >= ?2 AND l.line_no <= ?3
             ORDER BY l.line_no",
        )?;
        let max = crate::limits::MAX_SEARCH_HIT_EXCERPT_BYTES;
        let prefix_limit = max.saturating_add(1);
        let mut rows = stmt.query(params![path, line_start, line_end, prefix_limit])?;
        let mut excerpt = String::new();
        while let Some(row) = rows.next()? {
            let prefix = row.get::<_, Vec<u8>>(0)?;
            let line_bytes = row.get::<_, i64>(1)?;
            let line_truncated = usize::try_from(line_bytes)
                .map(|len| len > prefix.len())
                .unwrap_or(true);
            let valid_len = match std::str::from_utf8(&prefix) {
                Ok(_) => prefix.len(),
                Err(error) if line_truncated && error.error_len().is_none() => error.valid_up_to(),
                Err(error) => {
                    return Err(rusqlite::Error::FromSqlConversionFailure(
                        0,
                        Type::Text,
                        Box::new(error),
                    )
                    .into())
                }
            };
            let line = std::str::from_utf8(&prefix[..valid_len]).expect("validated prefix");
            let separator = usize::from(!excerpt.is_empty());
            if excerpt
                .len()
                .saturating_add(separator)
                .saturating_add(line.len())
                <= max
                && !line_truncated
            {
                if separator == 1 {
                    excerpt.push('\n');
                }
                excerpt.push_str(line);
                continue;
            }
            const MARKER: &str = "…";
            let content_limit = max.saturating_sub(MARKER.len());
            if excerpt.len() > content_limit {
                let mut end = content_limit;
                while end > 0 && !excerpt.is_char_boundary(end) {
                    end -= 1;
                }
                excerpt.truncate(end);
            }
            let mut allowance = content_limit.saturating_sub(excerpt.len());
            if separator == 1 && allowance > 0 {
                excerpt.push('\n');
                allowance -= 1;
            }
            let mut end = allowance.min(line.len());
            while end > 0 && !line.is_char_boundary(end) {
                end -= 1;
            }
            excerpt.push_str(&line[..end]);
            excerpt.push_str(MARKER);
            break;
        }
        Ok(excerpt)
    }
    pub fn semantic_chunk_max_id(&self) -> Result<Option<i64>> {
        optional_row(
            &self.conn,
            "SELECT MAX(id) FROM semantic_chunks",
            &[],
            |r| r.get::<_, Option<i64>>(0),
        )
        .map(Option::flatten)
    }
    pub fn semantic_chunk_stats(&self, lang: Option<&str>) -> Result<SemanticChunkStats> {
        let max_id = self.semantic_chunk_max_id()?.unwrap_or(0);
        let (count, dim): (usize, usize) = if let Some(l) = lang {
            self.conn.query_row(
                "SELECT COUNT(*), COALESCE(MAX(length(sc.vector)/4),0) FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id WHERE f.language=?1",
                params![l], |r| Ok((r.get(0)?, r.get(1)?)), )?
        } else {
            self.conn.query_row(
                "SELECT COUNT(*), COALESCE(MAX(length(vector)/4),0) FROM semantic_chunks",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?
        };
        Ok(SemanticChunkStats { count, max_id, dim })
    }
    pub fn semantic_chunk_ids(&self, lang: Option<&str>) -> Result<Vec<i64>> {
        let (sql, l) = if lang.is_some() {
            ("SELECT sc.id FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id WHERE f.language=?1 ORDER BY sc.id", lang)
        } else {
            ("SELECT id FROM semantic_chunks ORDER BY id", None)
        };
        query_map_rows(&self.conn, sql, l, |r| r.get(0))
    }
    pub fn semantic_chunks_by_ids(
        &self,
        ids: &[i64],
    ) -> Result<Vec<(i64, ast_sgrep_embed::SemanticChunkRow)>> {
        let mut out = Vec::with_capacity(ids.len());
        for batch in ids.chunks(500) {
            let ph = std::iter::repeat_n("?", batch.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT sc.id, f.path, sc.line_start, sc.line_end, sc.symbol_name, sc.text, sc.vector \
                 FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id WHERE sc.id IN ({ph})"
            );
            let mut stmt = self.conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(batch.iter()), |r| {
                let id: i64 = r.get(0)?;
                // Fail closed on corrupt blobs (parity with read_sem_row / emb_vec).
                // unwrap_or_default() previously dropped corrupt IVF candidates as
                // empty vectors, silently skewing ANN re-rank results (pass3).
                let row = (
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    r.get(5)?,
                    emb_vec(r, 6)?,
                );
                Ok((id, row))
            })?;
            out.extend(rows.collect::<std::result::Result<Vec<_>, _>>()?);
        }
        Ok(out)
    }
    pub fn all_semantic_chunks(
        &self,
        lang: Option<&str>,
    ) -> Result<Vec<ast_sgrep_embed::SemanticChunkRow>> {
        let sql = format!(
            "SELECT f.path, sc.line_start, sc.line_end, sc.symbol_name, sc.text, sc.vector \
             FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id WHERE 1=1{} ORDER BY sc.id",
            lang_and_clause(lang)
        );
        query_map_rows(&self.conn, &sql, lang, read_sem_row)
    }
    /// Walk sorted file paths and extend with per-path query results (stable path order).
    fn map_sorted_files<T>(
        files: &std::collections::HashSet<String>,
        mut query: impl FnMut(&str) -> Result<Vec<T>>,
    ) -> Result<Vec<T>> {
        let mut paths = files.iter().collect::<Vec<_>>();
        paths.sort_unstable();
        let mut out = Vec::new();
        for path in paths {
            out.extend(query(path)?);
        }
        Ok(out)
    }

    pub(crate) fn semantic_chunks_for_files(
        &self,
        files: &std::collections::HashSet<String>,
        lang: Option<&str>,
    ) -> Result<Vec<ast_sgrep_embed::SemanticChunkRow>> {
        Self::map_sorted_files(files, |path| match lang {
            Some(language) => query_cached_map(
                &self.conn,
                "SELECT f.path, sc.line_start, sc.line_end, sc.symbol_name, sc.text, sc.vector \
                 FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id \
                 WHERE f.path=?1 AND f.language=?2 ORDER BY sc.id",
                params![path, language],
                read_sem_row,
            ),
            None => query_cached_map(
                &self.conn,
                "SELECT f.path, sc.line_start, sc.line_end, sc.symbol_name, sc.text, sc.vector \
                 FROM semantic_chunks sc JOIN files f ON f.id=sc.file_id \
                 WHERE f.path=?1 ORDER BY sc.id",
                params![path],
                read_sem_row,
            ),
        })
    }

    pub(crate) fn legacy_embeddings_for_files(
        &self,
        files: &std::collections::HashSet<String>,
        lang: Option<&str>,
    ) -> Result<Vec<ast_sgrep_embed::SemanticChunkRow>> {
        Self::map_sorted_files(files, |path| match lang {
            Some(language) => query_cached_map(
                &self.conn,
                "SELECT f.path, l.line_no, l.content, sc.symbol_name, e.vector \
                 FROM embeddings e JOIN lines l ON l.file_id=e.file_id AND l.line_no=e.line_no \
                 JOIN files f ON f.id=e.file_id \
                 LEFT JOIN semantic_chunks sc ON sc.file_id=f.id AND sc.line_start=l.line_no \
                 WHERE f.path=?1 AND f.language=?2 ORDER BY l.line_no LIMIT 5000",
                params![path, language],
                read_legacy_emb,
            ),
            None => query_cached_map(
                &self.conn,
                "SELECT f.path, l.line_no, l.content, sc.symbol_name, e.vector \
                 FROM embeddings e JOIN lines l ON l.file_id=e.file_id AND l.line_no=e.line_no \
                 JOIN files f ON f.id=e.file_id \
                 LEFT JOIN semantic_chunks sc ON sc.file_id=f.id AND sc.line_start=l.line_no \
                 WHERE f.path=?1 ORDER BY l.line_no LIMIT 5000",
                params![path],
                read_legacy_emb,
            ),
        })
    }

    pub fn symbols_in_file(&self, rel_path: &str) -> Result<Vec<SymbolRow>> {
        query_cached_map(
            &self.conn,
            "SELECT s.name, s.kind, s.line_start, s.line_end, s.byte_start, s.byte_end \
             FROM symbols s JOIN files f ON f.id=s.file_id WHERE f.path=?1 ORDER BY s.line_start",
            params![rel_path],
            |r| {
                Ok(SymbolRow {
                    name: r.get(0)?,
                    kind: r.get(1)?,
                    line_start: r.get(2)?,
                    line_end: r.get(3)?,
                    byte_start: sql_usize_from_byte_offset(r, 4)?,
                    byte_end: sql_usize_from_byte_offset(r, 5)?,
                })
            },
        )
    }
    pub fn incoming_calls(&self, callee: &str) -> Result<Vec<CallRow>> {
        calls_matching(&self.conn, "callee", callee)
    }
    pub fn outgoing_calls(&self, caller: &str) -> Result<Vec<CallRow>> {
        calls_matching(&self.conn, "caller", caller)
    }
    pub fn symbol_at_line(&self, path: &str, line: u32) -> Result<Option<SymbolLocationRow>> {
        optional_row(
            &self.conn, &format!("{SYM_LOC} WHERE f.path=?1 AND s.line_start<=?2 AND s.line_end>=?2 ORDER BY (s.line_end-s.line_start), s.line_start DESC, s.name LIMIT 1"),
            &[&path as &dyn ToSql, &line as &dyn ToSql], read_sym_loc,
        )
    }
    pub fn first_symbol_in_file(&self, path: &str) -> Result<Option<SymbolLocationRow>> {
        optional_row(
            &self.conn,
            &format!("{SYM_LOC} WHERE f.path=?1 ORDER BY s.line_start, s.line_end, s.name LIMIT 1"),
            &[&path],
            read_sym_loc,
        )
    }
    pub fn symbols_named(&self, name: &str, limit: usize) -> Result<Vec<SymbolLocationRow>> {
        query_cached_map(
            &self.conn,
            &format!(
                "{SYM_LOC} WHERE lower(s.name)=lower(?1) ORDER BY f.path, s.line_start, s.line_end LIMIT ?2"
            ),
            params![name, limit as i64],
            read_sym_loc,
        )
    }
    pub fn imports_from_file(&self, path: &str) -> Result<Vec<ImportRow>> {
        query_cached_map(
            &self.conn,
            "SELECT i.module_path, i.line_no FROM imports i JOIN files f ON f.id=i.file_id \
             WHERE f.path=?1 ORDER BY i.line_no, i.module_path",
            params![path],
            |r| {
                Ok(ImportRow {
                    module_path: r.get(0)?,
                    line_no: r.get(1)?,
                })
            },
        )
    }
    pub fn resolve_module_path(&self, from_file: &str, module: &str) -> Result<Vec<String>> {
        let lang = self.file_language(from_file)?;
        let cands =
            super::module_resolve::collect_module_candidates(from_file, module, lang.as_deref());
        let mut out = Vec::new();
        for c in cands {
            if self.file_exists(&c)? {
                out.push(c);
            }
        }
        Ok(out)
    }

    pub(crate) fn file_language(&self, path: &str) -> Result<Option<String>> {
        optional_row(
            &self.conn,
            "SELECT language FROM files WHERE path=?1",
            &[&path],
            |r| r.get(0),
        )
    }
    pub fn pattern_node_count(&self) -> Result<usize> {
        count_star(&self.conn, "pattern_nodes")
    }
    pub fn pattern_nodes_matching(
        &self,
        signature: &str,
        lang: Option<&str>,
    ) -> Result<Vec<PatternNodeRow>> {
        let mut sql = String::from(
            "SELECT f.path, f.language, n.line_start, n.line_end, n.excerpt FROM pattern_nodes n JOIN files f ON f.id=n.file_id WHERE n.signature=?1",
        );
        if lang.is_some() {
            sql.push_str(" AND f.language=?2");
        }
        sql.push_str(" ORDER BY f.path, n.line_start");
        let map = |r: &rusqlite::Row<'_>| {
            Ok(PatternNodeRow {
                path: r.get(0)?,
                language: r.get(1)?,
                line_start: r.get(2)?,
                line_end: r.get(3)?,
                excerpt: r.get(4)?,
            })
        };
        match lang {
            Some(l) => query_cached_map(&self.conn, &sql, params![signature, l], map),
            None => query_cached_map(&self.conn, &sql, params![signature], map),
        }
    }
    pub fn file_text(&self, path: &str) -> Result<Option<String>> {
        let lines = self.file_lines(path)?;
        if lines.is_empty() {
            return Ok(None);
        }
        let sep = match self.get_meta(&format!("eol:{path}"))? {
            Some(v) if v == "crlf" => "\r\n",
            _ => "\n",
        };
        Ok(Some(
            lines
                .iter()
                .map(|(_, c)| c.as_str())
                .collect::<Vec<_>>()
                .join(sep),
        ))
    }
    pub fn file_lines(&self, path: &str) -> Result<Vec<(u32, String)>> {
        query_cached_map( &self.conn, "SELECT l.line_no, l.content FROM lines l JOIN files f ON f.id=l.file_id WHERE f.path=?1 ORDER BY l.line_no",
            params![path], |r| Ok((r.get(0)?, r.get(1)?)), )
    }
    pub fn line_content(&self, path: &str, line: u32) -> Result<Option<String>> {
        optional_row(
            &self.conn, "SELECT l.content FROM lines l JOIN files f ON f.id=l.file_id WHERE f.path=?1 AND l.line_no=?2",
            &[&path as &dyn ToSql, &line as &dyn ToSql], |r| r.get(0),
        )
    }
    pub fn query_imports(
        &self,
        module: Option<&str>,
        lang: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ImportQueryRow>> {
        let map = |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?));
        if module.is_none_or(|m| m.is_empty()) {
            let mut parts = Vec::new();
            let mut bind = Vec::new();
            append_lang_filter(&mut parts, &mut bind, lang);
            let w = where_clause(&parts);
            return query_limit_map(
                &self.conn,
                &format!("{IMPORT_SELECT}{w} LIMIT ?{}", bind.len() + 1),
                bind,
                limit,
                map,
            );
        }
        let m = module.unwrap().to_string();
        let (w, bind) = like_terms_filter("i.module_path", &[m], lang);
        query_limit_map(
            &self.conn,
            &format!("{IMPORT_SELECT}{w} LIMIT ?{}", bind.len() + 1),
            bind,
            limit,
            map,
        )
    }
    pub fn all_legacy_embeddings(
        &self,
        lang: Option<&str>,
    ) -> Result<Vec<ast_sgrep_embed::SemanticChunkRow>> {
        let sql = format!(
            "SELECT f.path, l.line_no, l.content, sc.symbol_name, e.vector FROM embeddings e \
             JOIN lines l ON l.file_id=e.file_id AND l.line_no=e.line_no JOIN files f ON f.id=e.file_id \
             LEFT JOIN semantic_chunks sc ON sc.file_id=f.id AND sc.line_start=l.line_no WHERE 1=1{} LIMIT 5000",
            lang_and_clause(lang)
        );
        query_map_rows(&self.conn, &sql, lang, read_legacy_emb)
    }
    pub fn file_exists(&self, path: &str) -> Result<bool> {
        Ok(self
            .conn
            .prepare_cached("SELECT 1 FROM files WHERE path=?1")?
            .exists(params![path])?)
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/core/store__sqlite__restore_synchronous_tests.rs"]
mod restore_synchronous_tests;

#[cfg(test)]
#[path = "../../../../tests/unit/core/store__sqlite__pass3_deep_core_tests.rs"]
mod pass3_deep_core_tests;

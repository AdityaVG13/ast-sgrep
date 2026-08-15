use super::embed_support::init_cache_seq;
use super::sql::configure_connection_with;
use super::sql::{optional_row, CLEAR_ALL_SQL, SCHEMA_DDL};
use super::try_index_db_path;
use crate::Result;
use ast_sgrep_lang::PatternNode;
use rusqlite::{params, Connection};
#[cfg(test)]
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
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
// 9 = repository lexicon. 10 = per-field semantic vectors (name/docs/body/graph).
// 11 = scip_facts overlay (kgvi.2). 12 = tests/examples semantic vector.
// Never reuse a SCHEMA_VERSION for two migrations.
const SCHEMA_VERSION: i64 = 12;
const IMPORT_SELECT: &str =
    "SELECT f.path, f.language, i.module_path, i.line_no FROM imports i JOIN files f ON f.id = i.file_id";
const SYM_LOC: &str = "SELECT f.path, s.name, f.language, s.line_start, s.line_end FROM symbols s JOIN files f ON f.id = s.file_id";
pub type IndexedLineRow = (Arc<str>, u32, String, Option<Arc<str>>);
pub type ImportQueryRow = (String, Option<String>, String, u32);
pub type CallRow = (String, u32, String, String);

#[derive(Debug, Clone)]
pub(crate) struct CallEvidenceRow {
    pub(crate) file: String,
    pub(crate) line: u32,
    pub(crate) caller: String,
    pub(crate) callee: String,
    pub(crate) scip_exact: bool,
}

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

fn ensure_semantic_field_vector_columns(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(semantic_chunks)")?;
    let existing = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for column in [
        "vector_name",
        "vector_docs",
        "vector_body",
        "vector_graph",
        "vector_tests_examples",
    ] {
        if !existing.iter().any(|name| name == column) {
            conn.execute(
                &format!("ALTER TABLE semantic_chunks ADD COLUMN {column} BLOB"),
                [],
            )?;
        }
    }
    Ok(())
}

fn ensure_scip_facts_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS scip_facts (
            file_id INTEGER NOT NULL, line_no INTEGER NOT NULL, name TEXT NOT NULL,
            is_def INTEGER NOT NULL, PRIMARY KEY (file_id, line_no, name, is_def),
            FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE);
         CREATE INDEX IF NOT EXISTS idx_scip_facts_file_id ON scip_facts(file_id);",
    )?;
    Ok(())
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
mod queries;
mod writes;
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
        if version < 12 {
            crate::semantic_ivf::invalidate_semantic_ivf(&self.db_path)?;
        }
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let migration = (|| -> Result<()> {
            self.conn.execute_batch(SCHEMA_DDL)?;
            if version < 12 {
                ensure_semantic_field_vector_columns(&self.conn)?;
            }
            if version < 11 {
                ensure_scip_facts_table(&self.conn)?;
            }
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
                   OR key IN ('embed_backend', 'embed_model', 'embed_dim');",
                )?;
            }
            // Schema 10 changed chunk rendering and added per-field vectors.
            // All semantic state is derived, so rebuild it rather than mixing
            // legacy primary vectors or cache metadata with the new layout.
            if version < 10 {
                self.conn.execute_batch(
                    "DELETE FROM semantic_chunks;
                     DELETE FROM embeddings;
                     DELETE FROM embed_cache;
                     DELETE FROM meta WHERE key LIKE 'body:%' OR key LIKE 'struct:%'
                       OR key IN ('embed_backend', 'embed_model', 'embed_dim');
                     UPDATE files SET content_hash = 'semantic-layout-v3:' || content_hash
                       WHERE content_hash NOT LIKE 'semantic-layout-v3:%';",
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

    /// Persist SCIP defs/refs that match existing symbols/callers (kgvi.2).
    /// Unmatched occurrences are skipped; no new graph edges are invented.
    pub fn apply_scip(
        &self,
        index: &crate::scip::ScipIndex,
    ) -> Result<crate::scip::ScipApplyStats> {
        self.with_file_tx(|| self.apply_scip_inner(index))
    }

    fn apply_scip_inner(
        &self,
        index: &crate::scip::ScipIndex,
    ) -> Result<crate::scip::ScipApplyStats> {
        self.conn.execute("DELETE FROM scip_facts", [])?;
        let mut files = HashMap::new();
        {
            let mut stmt = self.conn.prepare("SELECT id, path FROM files")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (id, path) = row?;
                files.insert(crate::scip::normalize_scip_path(&path), id);
            }
        }
        let mut symbols = Vec::new();
        {
            let mut stmt = self
                .conn
                .prepare("SELECT file_id, line_start, line_end, name FROM symbols")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            for row in rows {
                symbols.push(row?);
            }
        }
        let mut callers = HashSet::new();
        {
            let mut stmt = self
                .conn
                .prepare("SELECT file_id, line_no, callee FROM callers")?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                callers.insert(row?);
            }
        }
        let mut stats = crate::scip::ScipApplyStats::default();
        let mut insert = self.conn.prepare(
            "INSERT OR IGNORE INTO scip_facts(file_id, line_no, name, is_def) VALUES(?1, ?2, ?3, ?4)",
        )?;
        for doc in &index.documents {
            let Some(&file_id) = files.get(&crate::scip::normalize_scip_path(&doc.relative_path))
            else {
                stats.skipped += doc.occurrences.len();
                continue;
            };
            for occ in &doc.occurrences {
                let Some(name) = crate::scip::scip_symbol_ident(&occ.symbol) else {
                    stats.skipped += 1;
                    continue;
                };
                let Some(line) = occ.start_line_1based() else {
                    stats.skipped += 1;
                    continue;
                };
                let fact_line = if occ.is_definition() {
                    symbols.iter().find_map(|(fid, start, end, n)| {
                        (*fid == file_id && n == &name && line >= *start && line <= *end)
                            .then_some(*start)
                    })
                } else {
                    callers
                        .contains(&(file_id, line, name.clone()))
                        .then_some(line)
                };
                let Some(fact_line) = fact_line else {
                    stats.skipped += 1;
                    continue;
                };
                insert.execute(params![
                    file_id,
                    i64::from(fact_line),
                    name,
                    i64::from(u8::from(occ.is_definition()))
                ])?;
                if occ.is_definition() {
                    stats.defs_upgraded += 1;
                } else {
                    stats.refs_upgraded += 1;
                }
            }
        }
        Ok(stats)
    }

    pub fn scip_fact_set(&self, is_def: bool) -> Result<HashSet<(String, u32, String)>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT f.path, s.line_no, s.name FROM scip_facts s JOIN files f ON f.id = s.file_id WHERE s.is_def = ?1",
        )?;
        let rows = stmt.query_map(params![i64::from(u8::from(is_def))], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        rows.collect::<std::result::Result<HashSet<_>, _>>()
            .map_err(Into::into)
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
}

#[cfg(test)]
#[path = "../../../../../tests/unit/core/store__sqlite__restore_synchronous_tests.rs"]
mod restore_synchronous_tests;

#[cfg(test)]
#[path = "../../../../../tests/unit/core/store__sqlite__pass3_deep_core_tests.rs"]
mod pass3_deep_core_tests;

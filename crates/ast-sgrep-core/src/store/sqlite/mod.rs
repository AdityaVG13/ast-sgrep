use super::sql::{optional_row, CLEAR_ALL_SQL, SCHEMA_DDL};
use crate::Result;
use ast_sgrep_lang::PatternNode;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::sync::Arc;
// 6 = symbols_name_lower. 7 = semantic-layout-v2 wipe. 8 = unstemmed code FTS.
// 9 = repository lexicon. 10 = per-field semantic vectors (name/docs/body/graph).
// 11 = scip_facts overlay (kgvi.2). 12 = tests/examples semantic vector.
// 13 = callers lower() expression indexes (gauntlet-r11: calls_matching full-scan fix).
// 14 = pattern_nodes (file_id, signature) composite for cascade structural seeks.
// 15 = the php `::` call-signature re-key (`call:bar` → `call:Foo::bar`)
//      shipped on 14 without a bump; 15 discards pre-rekey `pattern_nodes`
//      signature rows and forces re-extraction (those rows are served
//      AUTHORITATIVELY for exact `call:` patterns, so a pre-rekey index
//      kept resurrecting wrong answers under the default search path).
//      Never reuse a SCHEMA_VERSION for two migrations.
// 16 = additive `files.depth_truncated` flag from the extraction depth
//      budget. No fast-path defeat: rows written before 16 came from an
//      unbounded (complete) walk, so DEFAULT 0 is truthful, and unchanged
//      files keep their complete rows. Changed/NEW files carry the flag
//      from the budgeted extraction; the cached pattern lane refuses to
//      serve flagged files as authoritative and walks them natively.
// 17 = whole-body parent chunk beside extracted child fragments. Chunk
//      derivation is not part of the mtime/content-hash fast paths, so
//      pre-17 fragment-only parents would never be rebuilt on unchanged
//      files; 17 wipes derived semantic state and re-keys content hashes
//      (same treatment as the 10 re-render). Never reuse a version.
pub const INDEX_SCHEMA_VERSION: i64 = 18;
const SCHEMA_VERSION: i64 = INDEX_SCHEMA_VERSION;
const IMPORT_SELECT: &str =
    "SELECT f.path, f.language, i.module_path, i.line_no FROM imports i JOIN files f ON f.id = i.file_id";
const SYM_LOC: &str = "SELECT f.path, s.name, f.language, s.line_start, s.line_end FROM symbols s JOIN files f ON f.id = s.file_id";
pub type IndexedLineRow = (Arc<str>, u32, String, Option<Arc<str>>);
pub type ImportQueryRow = (String, Option<String>, String, u32);
pub type CallRow = (String, u32, String, String);

/// Nearest ancestor (never the root itself) holding the default-layout index,
/// searched only up to and including the git work tree root: a parent that
/// merely happens to hold an index (a home directory, a shared scratch tree) is
/// not this project's checkout and must not capture a new project. The home
/// directory and anything above it never capture, git or no git — a git-less
/// tree (benchmark corpus, tarball extract) below an indexed home is still
/// its own project.
/// Silent when an env override moves indexes elsewhere.
fn enclosing_index_db(root: &Path) -> Option<PathBuf> {
    if std::env::var_os("ASGREP_INDEX_PATH").is_some()
        || std::env::var_os("ASGREP_USE_CACHE").is_some()
    {
        return None;
    }
    let start = if root.is_file() { root.parent()? } else { root };
    let work_tree = start.ancestors().find(|dir| dir.join(".git").exists());
    let home = std::env::var_os("HOME").map(PathBuf::from);
    start
        .ancestors()
        .skip(1)
        .take_while(|dir| work_tree.is_none_or(|tree| dir.starts_with(tree)))
        .take_while(|dir| home.as_deref().is_none_or(|h| !h.starts_with(dir)))
        .map(|dir| dir.join(super::INDEX_DIR).join(super::INDEX_DB))
        .find(|candidate| candidate.is_file())
}

/// No index file at this root. Say which file is missing and, when the root sits
/// inside an indexed checkout, name that index: a nested directory answering
/// "index is empty" reads as a broken index instead of a missing one. Keeps the
/// documented `index is empty` fail-closed phrase (docs/validation/negative-ledgers.md).
fn missing_index_error(root: &Path, db_path: &Path) -> crate::StoreError {
    if let Some(enclosing) = enclosing_index_db(root) {
        return crate::StoreError::EmptyIndex(format!(
            "index is empty for {}: no index file at {}; this checkout is indexed at {} -- run the command at that root, pass --index-path for a separate index, or rerun with --auto-index to index this checkout in place",
            root.display(),
            db_path.display(),
            enclosing.display()
        ));
    }
    crate::StoreError::EmptyIndex(format!(
        "index is empty for {}: no index file at {}; run: asgrep index {} --json, or rerun with --auto-index to index this checkout in place",
        root.display(),
        db_path.display(),
        root.display()
    ))
}

/// Writable open with the default layout inside an indexed checkout: refuse to
/// create a second `.asgrep` under the tree. The caller either works at the
/// checkout root or asks for a deliberate separate index path.
fn second_index_error(root: &Path, db_path: &Path, enclosing: &Path) -> crate::StoreError {
    let enclosing_root = enclosing
        .parent()
        .and_then(|dir| dir.parent())
        .unwrap_or(enclosing);
    crate::StoreError::Other(format!(
        "refusing to create a second index at {} (root {}): the enclosing checkout is already indexed at {} -- run: asgrep index {} --json, or pass --index-path for a separate index",
        db_path.display(),
        root.display(),
        enclosing.display(),
        enclosing_root.display()
    ))
}

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

/// Add `files.depth_truncated` when missing (idempotent: fresh dbs already
/// have it via SCHEMA_DDL; pre-16 dbs get the DEFAULT-0 column here).
fn ensure_depth_truncated_column(conn: &Connection) -> Result<()> {
    let existing: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(files)")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    if !existing.iter().any(|name| name == "depth_truncated") {
        conn.execute(
            "ALTER TABLE files ADD COLUMN depth_truncated INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
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

#[derive(Debug, Clone)]
pub(crate) struct FileIdentity {
    pub mtime_secs: i64,
    pub mtime_nanos: u32,
    pub content_hash: String,
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
    /// Extraction hit the depth budget; rows for this file are incomplete
    /// and the cached pattern lane must not serve them as authoritative.
    pub depth_truncated: bool,
    pub semantic_chunks: &'a [crate::semantic_chunk::SemanticChunkInput],
    pub embed_semantic: bool,
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
    /// Write-durability profile for this connection.
    durability: crate::store::Durability,
    /// Packed indexed lines for unique-query literal. Cleared on generation bump.
    line_corpus: std::cell::RefCell<Option<std::sync::Arc<crate::store::line_corpus::LineCorpus>>>,
    /// Opened read-bundle mapping. This is index materialization, not a
    /// query-keyed memo: it is reopened whenever the store generation moves.
    bundle: std::cell::RefCell<Option<std::sync::Arc<crate::store::read_bundle::ReadBundle>>>,
    /// Identity of the file backing `bundle`; a publish replaces the inode, so
    /// a long-lived reader cannot keep serving the old mapping by generation
    /// alone.
    bundle_file_stamp: std::cell::Cell<Option<(u64, u64)>>,
    line_corpus_disabled: std::cell::Cell<bool>,
    /// Opened names sidecar (folded idf vocabularies). Generation-gated by
    /// the reader; cleared implicitly by reopen on generation change.
    names_sidecar: std::cell::RefCell<Option<std::sync::Arc<crate::names_sidecar::NamesSidecar>>>,
    /// Parsed `semantic.ivf` mmap, keyed by ANN fingerprint. Dropped when the
    /// index generation moves so a rebuilt sidecar cannot ride the old mapping.
    lazy_ivf: std::cell::RefCell<
        Option<(
            [u8; 32],
            std::sync::Arc<crate::semantic_ivf::LazySemanticIvf>,
        )>,
    >,
    read_only: bool,
}
mod graph_reads;
mod lexicon_store;
mod open;
mod queries;
mod scip_overlay;
mod transactions;
mod writes;
pub use queries::fold_file_stat_identity;
impl IndexStore {
    fn init_schema(&self) -> Result<()> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(crate::StoreError::schema_newer_than_binary(
                version,
                SCHEMA_VERSION,
            ));
        }
        if self.read_only {
            return self.init_schema_readonly(version);
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
        let drops_fts_shadows = version < 18;
        let migration = (|| -> Result<()> {
            self.conn.execute_batch(SCHEMA_DDL)?;
            if version < 12 {
                ensure_semantic_field_vector_columns(&self.conn)?;
            }
            if version < 11 {
                ensure_scip_facts_table(&self.conn)?;
            }
            if version < 13 {
                // gauntlet-r11: backfill the lower() expression indexes for
                // existing indexes. SCHEMA_DDL above already carries them via
                // IF NOT EXISTS, but only a version bump guarantees the DDL
                // re-runs on stores that never re-open through a rebuild.
            }
            if version < 15 {
                // The php `::` re-key changed the meaning of `call:`
                // signature rows without bumping the
                // schema version, so a pre-rekey index still holds `call:bar`
                // rows the current binary would serve AUTHORITATIVELY for
                // `bar($$$A)` patterns. Signature rows have no row-level
                // migration path (their semantics are binary-defined), so
                // discard them and defeat both unchanged-file fast paths —
                // the content-hash prefix defeats the hash fast path and
                // dropping `mtime_identity_root` defeats the mtime fast path
                // (same mechanism as the schema-10 semantic re-render) — so
                // the next indexing pass re-extracts every file under the
                // current keying. Read-only opens cannot migrate; they refuse
                // loudly in `init_schema_readonly` instead.
                //
                // The hash prefix + mtime wipe defeat only TWO of the THREE
                // skip layers. The per-file structure
                // fingerprints (`body:<rel>` written by
                // `commit_prepared_files`, `struct:<rel>` written by
                // `upsert_file_material`) route a fingerprint-unchanged file
                // to `refresh_lines_only`, which rewrites the PLAIN content
                // hash WITHOUT reparsing — so a surviving fingerprint
                // re-legitimizes the hash on the very next pass and
                // `pattern_nodes` stays permanently empty (the loud gate's
                // own `asgrep reindex` remedy never rebuilt the rows; ident/
                // decl search silently answered `[]` and the codemod
                // honesty gate flipped to a silent ok:true zero-edit).
                // Discard both fingerprints here (same mechanism as the
                // schema-7/10 blocks) so the next pass takes the full
                // extract+upsert path and rebuilds the rows.
                self.conn.execute_batch(
                    "DELETE FROM pattern_nodes;
                     DELETE FROM meta WHERE key = 'mtime_identity_root';
                     DELETE FROM meta WHERE key LIKE 'body:%' OR key LIKE 'struct:%';
                     UPDATE files SET content_hash = 'schema15-rekey:' || content_hash
                       WHERE content_hash NOT LIKE 'schema15-rekey:%';",
                )?;
            }
            if version < 16 {
                // The extraction depth budget needs a per-file truncation
                // flag. Purely additive — pre-16 rows were
                // produced by the unbounded (complete) walk, so DEFAULT 0 is
                // truthful and no unchanged-file fast path needs defeating;
                // only re-extracted files can ever carry the flag. Guarded:
                // fresh (version-0) dbs just ran SCHEMA_DDL, which already
                // created the column.
                ensure_depth_truncated_column(&self.conn)?;
            }
            if version < 3 {
                self.conn.execute_batch(
                    "INSERT INTO lines_trigram(rowid, content) SELECT rowid, content FROM lines;",
                )?;
            }
            // Schema 8: backfill the unstemmed code field for older indexes.
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
            // Schema 17: chunking now always writes a whole-body parent
            // chunk beside extracted child fragments. Pre-17 stores can
            // represent a function by fragments alone, and neither the
            // mtime shortcut nor the content-hash path can observe a
            // chunk-derivation change, so rebuild derived semantic state
            // and defeat both fast paths (hash re-key + identity drop).
            if version < 17 {
                self.conn.execute_batch(
                    "DELETE FROM semantic_chunks;
                     DELETE FROM embeddings;
                     DELETE FROM embed_cache;
                     DELETE FROM meta WHERE key LIKE 'body:%' OR key LIKE 'struct:%'
                       OR key IN ('embed_backend', 'embed_model', 'embed_dim', 'mtime_identity_root');
                     UPDATE files SET content_hash = 'parent-chunk-v17:' || content_hash
                       WHERE content_hash NOT LIKE 'parent-chunk-v17:%';",
                )?;
            }
            // Schema 18: the two line analyzers become FTS5 external-content
            // tables bound to `lines`. Postings and ranking are identical
            // (columns read through the content table by rowid); the
            // duplicated `_content` shadow stores (~39 MB at repo scale)
            // disappear. Migration drops the legacy tables, recreates them
            // from the new DDL, and rebuilds postings from `lines` in-db --
            // no source re-read. Insert binds the same content value as the
            // `lines` row, so supplied-text tokenization cannot diverge.
            if version < 18 {
                self.conn.execute_batch(
                    "DROP TABLE IF EXISTS lines_fts; DROP TABLE IF EXISTS lines_code_fts;",
                )?;
                self.conn.execute_batch(SCHEMA_DDL)?;
                self.conn.execute_batch(
                    "INSERT INTO lines_fts(lines_fts) VALUES('rebuild');\
                     INSERT INTO lines_code_fts(lines_code_fts) VALUES('rebuild');\
                     DROP INDEX IF EXISTS idx_pattern_nodes_file;",
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
        if drops_fts_shadows {
            // DROP frees pages to the freelist only; return the file space
            // once for the schema-18 shadow-content reclaim.
            let _ = self.conn.execute_batch("VACUUM");
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
    /// Open (or reuse) the published read-bundle mapping. The handle is an index
    /// materialization, not a query-keyed memo; a generation mismatch reopens
    /// it. Explicit `bundle` fails closed when absent/stale; `auto` falls back
    /// to SQL only when the file does not exist.
    pub(crate) fn read_bundle(
        &self,
        generation: Option<i64>,
    ) -> Result<Option<std::sync::Arc<crate::store::read_bundle::ReadBundle>>> {
        if !crate::store::read_bundle::bundle_enabled()
            || crate::semantic_ivf::is_in_memory_db(&self.db_path)
        {
            return Ok(None);
        }
        let bundle_path = crate::store::read_bundle::path_for(&self.db_path);
        let stamp = crate::store::read_bundle::file_stamp(&bundle_path);
        if let Some(cached) = self.bundle.borrow().as_ref() {
            if stamp.is_some()
                && self.bundle_file_stamp.get() == stamp
                && generation.is_none_or(|generation| cached.generation() == generation)
            {
                return Ok(Some(std::sync::Arc::clone(cached)));
            }
        }
        let path = crate::store::read_bundle::readpath();
        let Some(bundle) = crate::store::read_bundle::ReadBundle::open(&self.db_path)? else {
            if path == crate::store::read_bundle::ReadPath::Bundle {
                return Err(crate::StoreError::Other(
                    "ASGREP_READPATH=bundle but the read bundle is missing; run `asgrep index`".into(),
                )
                .into());
            }
            return Ok(None);
        };
        if let Some(generation) = generation {
            if bundle.generation() != generation {
                return Err(crate::StoreError::Other(format!(
                    "read bundle generation {} != store {generation}; reindex required",
                    bundle.generation()
                ))
                .into());
            }
        }
        let bundle = std::sync::Arc::new(bundle);
        *self.bundle.borrow_mut() = Some(std::sync::Arc::clone(&bundle));
        self.bundle_file_stamp.set(stamp);
        Ok(Some(bundle))
    }
    pub(crate) fn line_corpus(
        &self,
    ) -> Result<Option<std::sync::Arc<crate::store::line_corpus::LineCorpus>>> {
        if self.line_corpus_disabled.get() || crate::env_flag::memo_off("corpus") {
            return Ok(None);
        }
        if crate::store::read_bundle::bundle_enabled() {
            if let Some(bundle) = self.read_bundle(None)? {
                let generation = bundle.generation();
                if let Some(cached) = self.line_corpus.borrow().as_ref() {
                    if cached.index_data_version == generation {
                        return Ok(Some(std::sync::Arc::clone(cached)));
                    }
                }
                let arc =
                    crate::store::line_corpus::LineCorpus::from_bundle(bundle, generation, 0)?;
                *self.line_corpus.borrow_mut() = Some(std::sync::Arc::clone(&arc));
                return Ok(Some(arc));
            }
            // Auto with no bundle: use the SQL materialization below.
        }
        let index_v = self.index_data_version()?;
        let Ok(pragma_v) = self
            .conn
            .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))
        else {
            return Ok(None);
        };
        if let Some(cached) = self.line_corpus.borrow().as_ref() {
            if cached.index_data_version == index_v && cached.pragma_data_version == pragma_v {
                return Ok(Some(std::sync::Arc::clone(cached)));
            }
        }
        match crate::store::line_corpus::LineCorpus::load(&self.conn, index_v, pragma_v)? {
            Some(arc) => {
                *self.line_corpus.borrow_mut() = Some(std::sync::Arc::clone(&arc));
                Ok(Some(arc))
            }
            None => {
                self.line_corpus_disabled.set(true);
                self.line_corpus.borrow_mut().take();
                Ok(None)
            }
        }
    }
    /// Resident RAM corpus without loading: `None` when no warmed corpus is
    /// cached (or it went stale). One-shot queries use this so a single
    /// `literal:` does not pay the full `lines` table materialization plus
    /// token-index build (flamegraph 2026-09-20: ~95% of one-shot wall under
    /// `literal_pass` -> `line_corpus`, 3/3 captures); they fall back to the
    /// trigram/SQL path instead. Warm sessions (`warm_search_path`,
    /// `asgrep serve`) load once and keep the memchr scan. A stale entry is
    /// reported as `None`, not reloaded: the next explicit warm revalidates.
    pub(crate) fn line_corpus_if_cached(
        &self,
    ) -> Result<Option<std::sync::Arc<crate::store::line_corpus::LineCorpus>>> {
        if self.line_corpus_disabled.get() || crate::env_flag::memo_off("corpus") {
            return Ok(None);
        }
        if crate::store::read_bundle::bundle_enabled() {
            let cached = match self.line_corpus.borrow().as_ref() {
                Some(cached) => std::sync::Arc::clone(cached),
                None => return Ok(None),
            };
            if let Some(bundle) = self.read_bundle(None)? {
                if cached.index_data_version == bundle.generation() {
                    return Ok(Some(cached));
                }
            }
            return Ok(None);
        }
        let cached = match self.line_corpus.borrow().as_ref() {
            Some(cached) => std::sync::Arc::clone(cached),
            None => return Ok(None),
        };
        let index_v = self.index_data_version()?;
        let Ok(pragma_v) = self
            .conn
            .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))
        else {
            return Ok(None);
        };
        if cached.index_data_version == index_v && cached.pragma_data_version == pragma_v {
            return Ok(Some(cached));
        }
        Ok(None)
    }
    /// True when a fresh RAM corpus is resident (no load as a side effect).
    /// Lets callers and health checks observe whether the session paid the
    /// one-time corpus materialization without paying it themselves.
    pub fn has_resident_line_corpus(&self) -> bool {
        self.line_corpus_if_cached().ok().flatten().is_some()
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

    fn init_schema_readonly(&self, version: i64) -> Result<()> {
        if version < SCHEMA_VERSION {
            // A below-current stamp means the on-disk rows may predate a
            // schema migration (14→15: the php `::` call re-key; 15→16: the
            // additive per-file depth-truncation flag the cached-lane
            // refusal keys on). A read-only open cannot
            // migrate, and the exact `call:`/`decl:` lane serves rows
            // AUTHORITATIVELY, so refuse with a rebuild instruction instead of
            // silently answering from stale rows. A writable open (asgrep
            // index / reindex) migrates in place; doctor's peek reports the
            // same mismatch with the same recovery.
            return Err(crate::StoreError::Other(format!(
                "index schema version {version} is older than supported version {SCHEMA_VERSION}; its rows predate a schema migration and must not be served as authoritative. Rebuild with: asgrep reindex {} --json",
                self.root.display()
            )));
        }
        let core: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('files','lines','meta','symbols')",
            [],
            |r| r.get(0),
        )?;
        if core < 4 {
            return Err(crate::StoreError::EmptyIndex(format!(
                "index is empty for {}; run: asgrep index {} --json",
                self.root.display(),
                self.root.display()
            )));
        }
        Ok(())
    }

    /// Schema version this binary writes (d3l5). On-disk value: `on_disk_schema_version`.
    pub fn schema_version(&self) -> i64 {
        SCHEMA_VERSION
    }

    pub fn on_disk_schema_version(&self) -> Result<i64> {
        self.conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(Into::into)
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Checkpoint WAL into the main db. No-op for read-only connections.
    pub fn checkpoint_wal(&self) -> Result<()> {
        if self.read_only {
            return Ok(());
        }
        super::sql::checkpoint_wal(&self.conn)
    }

    /// Borrow the names-sidecar cache slot (used by
    /// `names_sidecar::sidecar_dfs_for_store`).
    pub(crate) fn names_sidecar_slot(
        &self,
    ) -> std::cell::RefMut<'_, Option<std::sync::Arc<crate::names_sidecar::NamesSidecar>>> {
        self.names_sidecar.borrow_mut()
    }

    /// Sticky `semantic.ivf` mapping for a fingerprint. Mill serve reuses the
    /// same generation, so the mmap parse must not run on every query.
    pub(crate) fn cached_lazy_ivf(
        &self,
        fingerprint: [u8; 32],
    ) -> Result<Option<std::sync::Arc<crate::semantic_ivf::LazySemanticIvf>>> {
        {
            let slot = self.lazy_ivf.borrow();
            if let Some((fp, ivf)) = slot.as_ref() {
                if *fp == fingerprint {
                    return Ok(Some(std::sync::Arc::clone(ivf)));
                }
            }
        }
        let path = crate::semantic_ivf::semantic_ivf_path(&self.db_path);
        let loaded = crate::semantic_ivf::load_semantic_ivf_index(&path, fingerprint)?;
        *self.lazy_ivf.borrow_mut() =
            loaded.as_ref().map(|ivf| (fingerprint, std::sync::Arc::clone(ivf)));
        Ok(loaded)
    }

    pub fn index_data_version(&self) -> Result<i64> {
        // The meta row is authoritative for both same-connection and foreign
        // commits. No memo: the bundle header or one meta read is enough.
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
    /// True when the index still stores the unversioned `"semantic"` backend.
    /// Search refuses that meta; indexing must rewrite every chunk before
    /// promoting to the current `"semantic-v2"` identity.
    pub fn needs_legacy_semantic_rewrite(&self) -> Result<bool> {
        Ok(self.get_meta("embed_backend")?.as_deref() == Some("semantic"))
    }
    /// Start a proven-complete semantic rewrite inside the caller's bulk
    /// transaction. Old vectors are removed before the first upsert so that
    /// its resolved provider/model can establish the store-wide identity;
    /// every later upsert must match it or the bulk transaction rolls back.
    /// Embed-cache rows for any other model are dead weight after the
    /// rewrite (the cache is keyed by model id) and are purged here.
    pub(crate) fn reset_semantic_index_for_rewrite(&self, keep_model_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM semantic_chunks", [])?;
        self.conn.execute("DELETE FROM embeddings", [])?;
        self.conn.execute(
            "DELETE FROM embed_cache WHERE model_id != ?1",
            rusqlite::params![keep_model_id],
        )?;
        for key in [
            "embed_backend",
            "embed_backend_pref",
            "embed_model",
            "embed_dim",
        ] {
            self.delete_meta(key)?;
        }
        crate::semantic_ann::drop_semantic_ivf(self)?;
        self.bump_semantic_data_version()
    }
    fn bump_index_data_version(&self) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES('index_data_version', '1')              ON CONFLICT(key) DO UPDATE SET value =              CAST(COALESCE(meta.value, '0') AS INTEGER) + 1",
            [],
        )?;
        self.line_corpus.borrow_mut().take();
        self.bundle.borrow_mut().take();
        self.bundle_file_stamp.set(None);
        self.lazy_ivf.borrow_mut().take();
        self.line_corpus_disabled.set(false);
        Ok(())
    }
    /// Monotonic counter bumped on every semantic_chunks mutation (insert or delete).
    /// Used by SemanticCache and the IVF fingerprint to detect delete+re-add
    /// collisions where max_id is reused but chunk content/vectors differ.
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
    /// or rolls back.
    /// Active write-durability profile.
    pub fn durability(&self) -> crate::store::Durability {
        self.durability
    }

    /// Reclaim legacy `embed_cache` rows left by pre-gate indexes. With the
    /// static cache disabled nothing repopulates the table, so a full
    /// rebuild should return the space. VACUUM only when rows actually
    /// existed so fresh indexes never pay the rewrite.
    pub fn purge_embed_cache_if_populated(&self) -> Result<()> {
        let deleted =
            self.with_file_tx(|| Ok(self.conn.execute("DELETE FROM embed_cache", [])?))?;
        if deleted > 0 {
            let _ = self.conn.execute_batch("VACUUM");
        }
        Ok(())
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
            crate::semantic_ann::drop_semantic_ivf(self)
        })?;
        let _ = self.conn.execute_batch("VACUUM");
        Ok(())
    }
}

impl Drop for IndexStore {
    fn drop(&mut self) {
        if !self.read_only {
            let _ = super::sql::checkpoint_wal_on_close(&self.conn);
        }
    }
}
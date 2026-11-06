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
mod schema;
mod semantic_reads;
mod source_reads;
mod transactions;
mod writes;
pub use queries::fold_file_stat_identity;
impl IndexStore {
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
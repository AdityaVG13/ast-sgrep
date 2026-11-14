use super::sql::{optional_row, CLEAR_ALL_SQL, SCHEMA_DDL};
use crate::Result;
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
//   19: `lines_trigram` becomes a no-bundle accelerator. A bundle-backed
//      index answers literal and regex queries from the line corpus, so the
//      indexer stops populating the table and 19 empties it, returning the
//      ~1GB it costs on a mid-size repo. Purely a space reclaim: the table
//      survives (empty) so the incremental delete statements stay valid, and
//      the no-bundle control arm still builds and reads it.
//   20: primary and per-field `semantic_chunks` vectors are f16, matching
//      the precision already served by the read bundle. Migration quantizes
//      saved vectors in place; no source reindex or re-embedding is needed.
pub const INDEX_SCHEMA_VERSION: i64 = 20;
const SCHEMA_VERSION: i64 = INDEX_SCHEMA_VERSION;
const IMPORT_SELECT: &str =
    "SELECT f.path, f.language, i.module_path, i.line_no FROM imports i JOIN files f ON f.id = i.file_id";
const SYM_LOC: &str = "SELECT f.path, s.name, f.language, s.line_start, s.line_end FROM symbols s JOIN files f ON f.id = s.file_id";
pub type IndexedLineRow = (Arc<str>, u32, String, Option<Arc<str>>);
pub type ImportQueryRow = (String, Option<String>, String, u32);
pub type CallRow = (String, u32, String, String);

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
    /// Whether line writes also populate the `lines_trigram` FTS5 index. The
    /// trigram index is a no-bundle accelerator; a bundle-backed index answers
    /// literal and regex queries from the line corpus, so the indexer turns
    /// this off and the table stays empty. Defaults to on so a store opened
    /// outside the indexer keeps the historical behavior.
    build_trigram_index: std::cell::Cell<bool>,
    /// Cached answer to "does `lines_trigram` carry rows". The trigram arms
    /// are only correct against a populated table, so every one of them asks
    /// first; the table cannot gain rows under a live store.
    trigram_populated: std::cell::Cell<Option<bool>>,
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
mod generations;
mod lexicon_store;
mod open;
mod pattern_reads;
mod queries;
mod rows;
mod scip_overlay;
mod schema;
mod semantic_reads;
mod source_reads;
mod transactions;
mod writes;
pub use queries::fold_file_stat_identity;
pub use rows::{
    CallerRow, ImportRow, PatternNodeRow, RefreshLinesInput, SemanticChunkStats, SymbolLocationRow,
    SymbolRow, UpsertFileInput,
};
pub(crate) use rows::{
    CallEvidenceRow, FileIdentity, sql_i64_from_byte_offset, sql_usize_from_byte_offset,
};
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
    ///
    /// A store-ahead-of-bundle mismatch gets a bounded grace instead of
    /// failing closed: a daemon publish bumps `index_data_version` per
    /// file-tx but swaps the bundle file once at flush, so a reader arriving
    /// between the two sees a torn pair that converges on its own when the
    /// swap lands. A store that stays stable while the bundle stays stale is
    /// genuinely stale and still fails closed below.
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
                match self.await_bundle_publish(generation) {
                    Some(settled) => {
                        // The swap replaced the file: re-stamp, or the memo
                        // would pin the old inode and force a reopen per call.
                        let stamp = crate::store::read_bundle::file_stamp(
                            &crate::store::read_bundle::path_for(&self.db_path),
                        );
                        return Ok(Some(self.memoize_bundle(settled, stamp)));
                    }
                    None => {
                        return Err(crate::StoreError::Other(format!(
                            "read bundle generation {} != store {generation}; reindex required",
                            bundle.generation()
                        ))
                        .into());
                    }
                }
            }
        }
        Ok(Some(self.memoize_bundle(bundle, stamp)))
    }

    /// Memoize an opened bundle and return the shared handle. Single home for
    /// the memo store so the publish-grace path and the fast path cannot drift.
    fn memoize_bundle(
        &self,
        bundle: crate::store::read_bundle::ReadBundle,
        stamp: Option<(u64, u64)>,
    ) -> std::sync::Arc<crate::store::read_bundle::ReadBundle> {
        let bundle = std::sync::Arc::new(bundle);
        *self.bundle.borrow_mut() = Some(std::sync::Arc::clone(&bundle));
        self.bundle_file_stamp.set(stamp);
        bundle
    }

    /// Bounded grace for a store-ahead-of-bundle read: re-open the bundle
    /// while the store version is stable at the requested generation, so an
    /// in-flight daemon publish can land its swap. Returns the settled bundle
    /// when its header reaches `generation`; returns `None` when the store
    /// moved on (the caller's snapshot is superseded — its own fence owns the
    /// retry) or the budget expires with the bundle still stale (genuinely
    /// stale, or a crashed writer: fail closed as before).
    fn await_bundle_publish(
        &self,
        generation: i64,
    ) -> Option<crate::store::read_bundle::ReadBundle> {
        const GRACE_POLLS: usize = 200;
        const GRACE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(25);
        let mut polls = 0;
        loop {
            // The store moved past the caller's snapshot: superseded, not
            // settleable here. A crashed-writer read error also exits here
            // (fail closed, no grace on a broken connection).
            if self.index_data_version().ok() != Some(generation) {
                return None;
            }
            match crate::store::read_bundle::ReadBundle::open(&self.db_path).ok()? {
                Some(next) if next.generation() == generation => return Some(next),
                Some(_) => {}
                None => return None,
            }
            polls += 1;
            if polls >= GRACE_POLLS {
                return None;
            }
            std::thread::sleep(GRACE_INTERVAL);
        }
    }
    /// Turn `lines_trigram` population on or off for this store's line
    /// writes. The indexer calls this once at construction; see
    /// [`crate::store::read_bundle::bundle_armed`].
    pub(crate) fn set_build_trigram_index(&self, build: bool) {
        self.build_trigram_index.set(build);
    }
    /// Whether line writes populate `lines_trigram`.
    pub(crate) fn builds_trigram_index(&self) -> bool {
        self.build_trigram_index.get()
    }
    /// Whether `lines_trigram` carries rows. Every trigram arm is only
    /// correct against a populated table — an empty one answers `MATCH` with
    /// zero rows rather than falling back — so each asks here first and takes
    /// the full-scan arm when the answer is no. A bundle-backed index never
    /// populates it, and the table cannot gain rows under a live store, so
    /// the probe runs once.
    ///
    /// The probe reads the FTS5 `_docsize` shadow table, not `lines_trigram`
    /// itself: the table is external-content (`content = 'lines'`), so a
    /// plain `SELECT` there answers from `lines` and reports rows even when
    /// the index holds none. `_docsize` carries one row per indexed document
    /// and is empty exactly when the index is.
    pub(crate) fn trigram_index_populated(&self) -> bool {
        if let Some(cached) = self.trigram_populated.get() {
            return cached;
        }
        let populated = self
            .conn
            .query_row("SELECT 1 FROM lines_trigram_docsize LIMIT 1", [], |_| {
                Ok(())
            })
            .is_ok();
        self.trigram_populated.set(Some(populated));
        populated
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
    /// Corpus for a one-shot query: the warmed cache when it is fresh, else a
    /// bundle-backed corpus when the read bundle exists.
    ///
    /// The cached-only gate above exists to keep a one-shot off the SQL `lines`
    /// materialization. The bundle arm is a different cost class: it is
    /// mmap-backed and copies only the file table, so a one-shot can hold it
    /// for ~1MB and answer the cascade's literal prefilter with the memchr scan
    /// instead of a trigram doclist walk. Measured on a 4.5k-file corpus, the
    /// prefilter drops from 88-103ms (SQL arm) to 1-5ms (bundle arm) with
    /// identical hits. Without a bundle this is exactly
    /// [`Self::line_corpus_if_cached`].
    pub(crate) fn line_corpus_for_query(
        &self,
    ) -> Result<Option<std::sync::Arc<crate::store::line_corpus::LineCorpus>>> {
        if let Some(cached) = self.line_corpus_if_cached()? {
            return Ok(Some(cached));
        }
        if self.line_corpus_disabled.get()
            || crate::env_flag::memo_off("corpus")
            || !crate::store::read_bundle::bundle_enabled()
        {
            return Ok(None);
        }
        let Some(bundle) = self.read_bundle(None)? else {
            return Ok(None);
        };
        let generation = bundle.generation();
        let arc = crate::store::line_corpus::LineCorpus::from_bundle(bundle, generation, 0)?;
        *self.line_corpus.borrow_mut() = Some(std::sync::Arc::clone(&arc));
        Ok(Some(arc))
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

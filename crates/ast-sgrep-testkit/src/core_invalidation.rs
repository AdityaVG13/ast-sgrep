//! Core invalidation fixtures: hermetic corpus+index, projectors, battery.
//!
//! # Contract
//!
//! - One canonical copy of the file-local helpers triplicated across the
//!   `tests/core/invalidation_*` suites (`delta_matrix`, `refresh`, `drills`)
//!   plus the hermetic [`Indexer`] triple shared with `staleness`.
//! - Every open uses an explicit `index_path` outside the corpus root with
//!   `use_tantivy: false` and `embed_semantic: false`, so no test depends on
//!   ambient `ASGREP_*` routing. See [`hermetic_indexer`].
//! - Projectors pin served structure (def files, literal spots, caller files,
//!   structural battery tuples), never scores, excerpts, or message text.
//!   [`InvalidationFixture::stored_counts`] is the strictest of the suite
//!   copies: the full 5-tuple (files, lines, symbols, callers, imports).
//! - Helpers panic (never `Result`) on IO/index failure, matching suite
//!   convention: a broken fixture is a test failure, not a fallible op.

use ast_sgrep_core::index::WatchUpdateStats;
use ast_sgrep_core::{
    HitKind, IndexOptions, IndexStats, IndexStore, Indexer, SearchOptions, Searcher,
};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// INTENT: the hermetic [`Indexer`] triple every core invalidation test opens
/// with — explicit `index_path`, no tantivy, no embed. Built on
/// [`hermetic_index_options`], so the four suite copies share one triple and
/// core flag additions touch one constructor.
pub fn hermetic_indexer(root: &Path, db: &Path) -> Indexer {
    Indexer::new(hermetic_index_options(root, db)).unwrap()
}

/// INTENT: hermetic [`IndexOptions`] — explicit `index_path`, no tantivy, no
/// embed. The single source behind [`hermetic_indexer`] and every hermetic
/// core build leg (see [`crate::core_index_options`],
/// [`crate::build_and_quiet`]). Callers add `force_reindex` / `use_tantivy`
/// via struct update. Pure constructor.
pub fn hermetic_index_options(root: &Path, db: &Path) -> IndexOptions {
    IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        use_tantivy: false,
        embed_semantic: false,
        ..IndexOptions::default()
    }
}

/// INTENT: hermetic [`SearchOptions`] — explicit `index_path`, caller limit,
/// no tantivy, no embed. The single source behind
/// [`InvalidationFixture::searcher`] and every lexical search leg. Callers
/// add `use_embed` / `use_tantivy` via struct update when the facet under
/// test varies them (see [`crate::core_search_hit_keys`],
/// [`crate::search_parity_key`]). Pure constructor. Delta vs
/// [`crate::searcher_at_root`]: that opens the default state layout
/// (`index_path: None`) and deliberately does NOT route through here.
pub fn hermetic_search_options(root: &Path, db: &Path, limit: usize) -> SearchOptions {
    SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        limit,
        use_embed: false,
        use_tantivy: false,
        ..SearchOptions::default()
    }
}

/// One battery hit: (query, kind, file, line_start, line_end, symbol, caller, callee).
/// Scores and excerpts are excluded: the relation is structural identity.
pub type HitTuple = (String, String, String, u32, u32, String, String, String);

/// INTENT: the writable starting point every core invalidation test builds
/// from — a private corpus root plus an external index database, with the
/// shared refresh/search/projector surface. The caller keeps the fixture alive;
/// dropping it removes the corpus and the DB.
pub struct InvalidationFixture {
    _corpus: TempDir,
    _index: TempDir,
    /// Writable corpus directory under the private temp root.
    pub root: PathBuf,
    /// Explicit path to the real on-disk SQLite database file.
    pub db: PathBuf,
}

impl InvalidationFixture {
    /// Fresh corpus (`src/` created) plus an external `index.db` path.
    pub fn new() -> Self {
        let corpus = tempfile::tempdir().unwrap();
        let index = tempfile::tempdir().unwrap();
        let root = corpus.path().to_path_buf();
        std::fs::create_dir_all(root.join("src")).unwrap();
        let db = index.path().join("index.db");
        Self {
            _corpus: corpus,
            _index: index,
            root,
            db,
        }
    }

    /// Open the fixture's store (for status/generation/row probes).
    pub fn open_store(&self) -> IndexStore {
        IndexStore::open(&self.root, Some(&self.db)).unwrap()
    }

    /// Write `rel` under the corpus root (parents created); returns the abs path.
    pub fn write(&self, rel: &str, content: &str) -> PathBuf {
        let abs = self.root.join(rel);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&abs, content).unwrap();
        abs
    }

    pub fn indexer(&self) -> Indexer {
        hermetic_indexer(&self.root, &self.db)
    }

    pub fn reindex(&self) -> IndexStats {
        self.indexer().index_all().unwrap()
    }

    pub fn update(&self, paths: &[PathBuf]) -> WatchUpdateStats {
        self.indexer().update_paths(paths).unwrap()
    }

    /// Lexical [`Searcher`] over this fixture (limit 256, no embed): the
    /// [`crate::hermetic_search_options`] triple pinned to this fixture.
    pub fn searcher(&self) -> Searcher {
        Searcher::new(crate::hermetic_search_options(&self.root, &self.db, 256)).unwrap()
    }

    /// Sorted hit tuples for a fixed query battery.
    pub fn battery(&self, queries: &[&str]) -> Vec<HitTuple> {
        let searcher = self.searcher();
        let mut out = Vec::new();
        for query in queries {
            for h in searcher.search(query).unwrap().hits.iter() {
                out.push((
                    query.to_string(),
                    h.kind.as_str().to_string(),
                    h.file.clone(),
                    h.line_start,
                    h.line_end,
                    h.symbol.clone().unwrap_or_default(),
                    h.caller.clone().unwrap_or_default(),
                    h.callee.clone().unwrap_or_default(),
                ));
            }
        }
        out.sort();
        out
    }

    /// Sorted `file` values of Def hits whose symbol is exactly `symbol`.
    pub fn def_files(&self, symbol: &str) -> Vec<String> {
        let query = format!("defs:{symbol}");
        let mut files: Vec<String> = self
            .searcher()
            .search(&query)
            .unwrap()
            .hits
            .iter()
            .filter(|h| h.kind == HitKind::Def && h.symbol.as_deref() == Some(symbol))
            .map(|h| h.file.clone())
            .collect();
        files.sort();
        files
    }

    /// Sorted `(file, line_start)` spots for a `literal:` query (all hits).
    pub fn literal_spots(&self, token: &str) -> Vec<(String, u32)> {
        let query = format!("literal:{token}");
        let mut spots: Vec<(String, u32)> = self
            .searcher()
            .search(&query)
            .unwrap()
            .hits
            .iter()
            .map(|h| (h.file.clone(), h.line_start))
            .collect();
        spots.sort();
        spots
    }

    /// Sorted `file` values of Caller hits whose callee is exactly `callee`.
    pub fn caller_files(&self, callee: &str) -> Vec<String> {
        let query = format!("callers:{callee}");
        let mut files: Vec<String> = self
            .searcher()
            .search(&query)
            .unwrap()
            .hits
            .iter()
            .filter(|h| h.kind == HitKind::Caller && h.callee.as_deref() == Some(callee))
            .map(|h| h.file.clone())
            .collect();
        files.sort();
        files
    }

    pub fn generation(&self) -> i64 {
        self.open_store().index_data_version().unwrap()
    }

    /// Stored row counts: (files, lines, symbols, callers, imports).
    pub fn stored_counts(&self) -> (usize, usize, usize, usize, usize) {
        let status = self.open_store().status().unwrap();
        (
            status.file_count,
            status.line_count,
            status.symbol_count,
            status.caller_count,
            status.import_count,
        )
    }
}

impl Default for InvalidationFixture {
    fn default() -> Self {
        Self::new()
    }
}

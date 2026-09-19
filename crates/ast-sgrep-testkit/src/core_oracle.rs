//! Core pipeline/oracle fixtures: corpus+DB pairs, option builders, and
//! response-key projections shared by the `tests/core/oracle_*` suites.
//!
//! # Contract
//!
//! - One canonical copy of the file-local helpers formerly triplicated across
//!   `oracle_pipeline` / `oracle_stages` / `oracle_scoring` (corpus fixture,
//!   index/search options, full-build and searcher legs, hit-key projections).
//! - Builders index real tempdir corpora into real on-disk SQLite (`index.db`
//!   under a private [`tempfile::TempDir`]); `index_path` is always explicit,
//!   so ambient `ASGREP_INDEX_PATH` / XDG cache cannot leak across tests.
//! - Embeddings are off by construction (`embed_semantic: false`,
//!   `use_embed: false`): these are the lexical/pipeline oracles.
//! - Helpers panic (never `Result`) on IO/index failure, matching suite
//!   convention: a broken fixture is a test failure, not a fallible op.
//! - Key projections are pure functions of the response.

use ast_sgrep_core::intent::ChannelWeights;
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, SearchResponse, Searcher};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// INTENT: private writable corpus + private on-disk DB kept alive for one
/// pipeline test. The caller keeps the fixture alive; `root`/`db` feed the
/// option builders below.
pub struct CorePipelineFixture {
    /// Owns the corpus tree; must outlive `root` use.
    pub _corpus: TempDir,
    /// Owns the DB directory; must outlive `db` use.
    pub _index: TempDir,
    /// Writable corpus root.
    pub root: PathBuf,
    /// Explicit on-disk index path (`index.db` under the private index dir).
    pub db: PathBuf,
}

/// INTENT: build the corpus+DB pair from `(rel, body)` files (parents
/// created). Panics on IO failure.
pub fn write_core_fixture(files: &[(&str, &str)]) -> CorePipelineFixture {
    let corpus = tempfile::tempdir().expect("corpus tempdir");
    for (rel, body) in files {
        let path = corpus.path().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        fs::write(&path, body).expect("write fixture");
    }
    let index = tempfile::tempdir().expect("index tempdir");
    let db = index.path().join("index.db");
    CorePipelineFixture {
        root: corpus.path().to_path_buf(),
        db,
        _corpus: corpus,
        _index: index,
    }
}

/// INTENT: force-reindex, embed-off index options over the fixture.
/// Pure constructor.
pub fn core_index_options(root: &Path, db: &Path) -> IndexOptions {
    IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    }
}

/// INTENT: embed-off search options with an explicit limit. Pure constructor.
pub fn core_search_options(root: &Path, db: &Path, limit: usize) -> SearchOptions {
    SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        limit,
        use_embed: false,
        ..SearchOptions::default()
    }
}

/// INTENT: full build leg — fresh indexer over the fixture, whole tree
/// indexed. Panics on index failure.
pub fn build_core_index(fixture: &CorePipelineFixture) -> Indexer {
    let mut indexer =
        Indexer::new(core_index_options(&fixture.root, &fixture.db)).expect("indexer new");
    indexer.index_all().expect("index_all");
    indexer
}

/// INTENT: searcher over the fixture DB with the given limit. Panics when the
/// searcher cannot be constructed.
pub fn core_searcher(fixture: &CorePipelineFixture, limit: usize) -> Searcher {
    Searcher::new(core_search_options(&fixture.root, &fixture.db, limit)).expect("searcher new")
}

/// INTENT: order-sensitive identity of a ranked response (file, span, kind,
/// symbol, exact score bits) — two full runs agree iff these agree. Pure
/// projection; [`crate::response_hit_keys`] has no score bits, so pipeline
/// bit-identity needs this richer key.
pub fn response_hit_keys_with_scores(
    response: &SearchResponse,
) -> Vec<(String, u32, u32, String, Option<String>, u64)> {
    response
        .hits
        .iter()
        .map(|hit| {
            (
                hit.file.clone(),
                hit.line_start,
                hit.line_end,
                hit.kind.as_str().to_string(),
                hit.symbol.clone(),
                hit.score.to_bits(),
            )
        })
        .collect()
}

/// INTENT: order-free file-set comparison for shaping facets. Pure projection.
pub fn sorted_hit_files(response: &SearchResponse) -> Vec<String> {
    let mut files: Vec<String> = response.hits.iter().map(|hit| hit.file.clone()).collect();
    files.sort();
    files
}

/// INTENT: minimal `SearchOptions` (root + limit, rerank off) shared by every
/// finish-gate facet. Pure constructor.
pub fn finish_options(root: &Path, limit: usize) -> SearchOptions {
    SearchOptions {
        root: root.to_path_buf(),
        limit,
        file_filter: None,
        count_only: false,
        use_rerank: false,
        ..SearchOptions::default()
    }
}

/// INTENT: all-ones channel weights — the neutral element every weighted-RRF
/// facet builds from. Pure constructor.
pub fn unit_channel_weights() -> ChannelWeights {
    ChannelWeights {
        lexical: 1.0,
        def: 1.0,
        caller: 1.0,
        graph: 1.0,
        anchor: 1.0,
        embed: 1.0,
        pattern: 1.0,
        import: 1.0,
    }
}

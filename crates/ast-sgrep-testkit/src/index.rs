use crate::fixture::sample_root;
use crate::hit::HitKey;
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, SearchResponse, Searcher};
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

/// Sample-fixture index with a **private on-disk SQLite** (`TempDir` / `index.db`).
///
/// Isolation: `index_path` is always set under `_temp`, so `ASGREP_INDEX_PATH` /
/// XDG cache cannot share state across tests. The corpus defaults to the
/// read-only shared [`sample_root`] (immutable fixture files). For a private
/// **writable** corpus + DB, use [`crate::IsolatedIndexSession`].
pub struct IndexedFixture {
    /// Keeps the private DB directory alive for the test lifetime.
    pub _temp: TempDir,
    pub indexer: Indexer,
}

pub fn reopen_indexer(indexed: &IndexedFixture, overrides: IndexOptions) -> Indexer {
    Indexer::new(IndexOptions {
        root: indexed.indexer.store().root().to_path_buf(),
        index_path: Some(indexed.indexer.store().db_path().to_path_buf()),
        ..overrides
    })
    .expect("indexer")
}

/// Index the shared sample fixture into a **fresh real SQLite** file under a
/// private [`TempDir`]. Always sets an explicit `index_path` (never env/cache).
pub fn index_sample(mut opts: IndexOptions) -> IndexedFixture {
    let temp = TempDir::new().expect("tempdir");
    opts.index_path = Some(temp.path().join("index.db"));
    if opts.root.as_os_str() == "." {
        opts.root = sample_root();
    }
    let mut indexer = Indexer::new(opts).expect("indexer");
    indexer.index_all().expect("index");
    IndexedFixture {
        _temp: temp,
        indexer,
    }
}
pub fn searcher_from(indexed: &IndexedFixture, mut opts: SearchOptions) -> Searcher {
    opts.root = indexed.indexer.store().root().to_path_buf();
    opts.index_path = Some(indexed.indexer.store().db_path().to_path_buf());
    Searcher::new(opts).expect("searcher")
}
/// Stable identity shared by surface-equivalence tests. Scores, excerpts, and
/// response wrappers intentionally do not participate. Callers must align
/// surface-specific limit and embedding defaults before comparing these keys.
///
/// x1p5: rich HitKey includes symbol/callee/caller when present.
pub fn response_hit_keys(response: &SearchResponse) -> Vec<HitKey> {
    response
        .hits
        .iter()
        .map(crate::hit::HitKey::from_search_hit)
        .collect()
}
/// Native-face hit keys through the one fallible core
/// ([`crate::hit::hit_keys`]); panics when the response carries no hit
/// array. Pure projection.
pub fn json_hit_keys(response: &Value) -> Vec<HitKey> {
    crate::hit::hit_keys(response).expect("search response hits")
}
/// Core search → surface hit keys.
///
/// `use_embed` must match the CLI/LSP surface under comparison. Default
/// production is embed-on (hashed offline); pass `false` only for explicit
/// `--no-embed` parity (lbx1.13: embed-on parity must use `true`).
pub fn core_search_hit_keys(
    root: &Path,
    index_path: &Path,
    query: &str,
    limit: usize,
    use_embed: bool,
) -> Vec<HitKey> {
    let searcher = Searcher::new(SearchOptions {
        use_embed,
        ..crate::hermetic_search_options(root, index_path, limit)
    })
    .expect("core searcher");
    response_hit_keys(&searcher.search(query).expect("core search"))
}

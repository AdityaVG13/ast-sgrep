use crate::fixture::sample_root;
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
    // Explicit path: never fall through to ASGREP_INDEX_PATH / shared cache.
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
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HitKey {
    pub file: String,
    pub line_start: u32,
    pub kind: String,
    pub symbol: Option<String>,
    pub callee: Option<String>,
    pub caller: Option<String>,
}
pub fn response_hit_keys(response: &SearchResponse) -> Vec<HitKey> {
    response
        .hits
        .iter()
        .map(|hit| HitKey {
            file: hit.file.clone(),
            line_start: hit.line_start,
            kind: hit.kind.as_str().to_owned(),
            symbol: hit.symbol.clone(),
            callee: hit.callee.clone(),
            caller: hit.caller.clone(),
        })
        .collect()
}
pub fn json_hit_keys(response: &Value) -> Vec<HitKey> {
    response["hits"]
        .as_array()
        .expect("search response hits")
        .iter()
        .map(|hit| HitKey {
            file: hit["file"].as_str().expect("hit file").to_owned(),
            line_start: hit["line_start"].as_u64().expect("hit line_start") as u32,
            kind: hit["kind"].as_str().expect("hit kind").to_owned(),
            symbol: hit
                .get("symbol")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            callee: hit
                .get("callee")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            caller: hit
                .get("caller")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
        })
        .collect()
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
        root: root.to_path_buf(),
        index_path: Some(index_path.to_path_buf()),
        limit,
        use_embed,
        ..SearchOptions::default()
    })
    .expect("core searcher");
    response_hit_keys(&searcher.search(query).expect("core search"))
}

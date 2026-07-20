use std::path::Path;

use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, SearchResponse, Searcher}; use serde_json::Value; use tempfile::TempDir;

use crate::fixture::sample_root;

pub struct IndexedFixture {
    pub _temp: TempDir, pub indexer: Indexer,
}

pub fn reopen_indexer(indexed: &IndexedFixture, overrides: IndexOptions) -> Indexer {
    Indexer::new(IndexOptions {
        root: indexed.indexer.store().root().to_path_buf(), index_path: Some(indexed.indexer.store().db_path().to_path_buf()), ..overrides
    }) .expect("indexer")
}

pub fn index_sample(mut opts: IndexOptions) -> IndexedFixture {
    let temp = TempDir::new().expect("tempdir"); opts.index_path = Some(temp.path().join("index.db"));
    if opts.root.as_os_str() == "." { opts.root = sample_root(); } let mut indexer = Indexer::new(opts).expect("indexer"); indexer.index_all().expect("index"); IndexedFixture {
        _temp: temp, indexer,
    }
}

pub fn searcher_from(indexed: &IndexedFixture, mut opts: SearchOptions) -> Searcher {
    opts.root = indexed.indexer.store().root().to_path_buf(); opts.index_path = Some(indexed.indexer.store().db_path().to_path_buf()); Searcher::new(opts).expect("searcher")
}

/// Stable identity shared by surface-equivalence tests. Scores, excerpts, and
/// response wrappers intentionally do not participate. Callers must align
/// surface-specific limit and embedding defaults before comparing these keys.
#[derive(Debug, Clone, PartialEq, Eq)] pub struct HitKey {
    pub file: String, pub line_start: u32, pub kind: String,
}

pub fn response_hit_keys(response: &SearchResponse) -> Vec<HitKey> {
    response
        .hits .iter() .map(|hit| HitKey {
            file: hit.file.clone(), line_start: hit.line_start, kind: hit.kind.as_str().to_owned(),
        }) .collect()
}

pub fn json_hit_keys(response: &Value) -> Vec<HitKey> {
    response["hits"]
        .as_array() .expect("search response hits") .iter() .map(|hit| HitKey {
            file: hit["file"].as_str().expect("hit file").to_owned(), line_start: hit["line_start"].as_u64().expect("hit line_start") as u32, kind: hit["kind"].as_str().expect("hit kind").to_owned(),
        }) .collect()
}

pub fn core_search_hit_keys(
    root: &Path, index_path: &Path, query: &str, limit: usize,
) -> Vec<HitKey> {
    let searcher = Searcher::new(SearchOptions {
        root: root.to_path_buf(), index_path: Some(index_path.to_path_buf()), limit, use_embed: false, ..SearchOptions::default()
    }) .expect("core searcher"); response_hit_keys(&searcher.search(query).expect("core search"))
}

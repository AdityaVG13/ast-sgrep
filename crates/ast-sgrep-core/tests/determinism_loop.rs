//! Determinism regression (6ulo): identical no-embed searches must be stable.
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use std::fs;
use tempfile::TempDir;

#[test]
fn fifty_identical_searches_are_byte_stable() {
    let corpus = TempDir::new().unwrap();
    fs::write(
        corpus.path().join("stable.rs"),
        "fn auth_refresh() { renew_credentials(); }\nfn renew_credentials() {}\n",
    )
    .unwrap();
    let index_dir = TempDir::new().unwrap();
    let index_path = index_dir.path().join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        embed_semantic: false,
        force_reindex: true,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();

    let searcher = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        use_embed: false,
        limit: 16,
        ..SearchOptions::default()
    })
    .unwrap();

    let first = searcher.search("auth_refresh").unwrap();
    assert!(
        !first.hits.is_empty(),
        "expected non-empty hits for determinism baseline"
    );
    let first_json = serde_json::to_string(&first).unwrap();
    for i in 0..50 {
        let next = searcher.search("auth_refresh").unwrap();
        assert_eq!(
            next.hits.len(),
            first.hits.len(),
            "hit_count drifted on iteration {i}"
        );
        let next_json = serde_json::to_string(&next).unwrap();
        assert_eq!(
            next_json, first_json,
            "JSON identity drifted on iteration {i}"
        );
    }
}

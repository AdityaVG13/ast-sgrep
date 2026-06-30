//! Scale regression: SQL-filtered symbol pass must stay bounded.

use std::path::PathBuf;

use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use tempfile::TempDir;

#[test]
fn symbol_pass_bounded_on_large_term_set() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/sample")
        .canonicalize()
        .unwrap();
    let temp = TempDir::new().unwrap();
    let index_path = temp.path().join("index.db");

    let mut indexer = Indexer::new(IndexOptions {
        root: fixture.clone(),
        index_path: Some(index_path.clone()),
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();

    let searcher = Searcher::new(SearchOptions {
        root: fixture,
        index_path: Some(index_path),
        limit: 64,
        ..SearchOptions::default()
    })
    .unwrap();

    let response = searcher
        .search("how does auth refresh and process_request work")
        .unwrap();
    assert!(!response.hits.is_empty());
    assert!(response.hits.len() <= 64);
}

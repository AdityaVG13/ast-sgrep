use std::path::PathBuf;

use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use tempfile::TempDir;

#[test]
fn indexes_and_searches_polyglot_fixture() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/sample");
    let fixture = fixture.canonicalize().expect("fixture path");

    let temp = TempDir::new().unwrap();
    let index_path = temp.path().join("index.db");

    let mut indexer = Indexer::new(IndexOptions {
        root: fixture.clone(),
        index_path: Some(index_path.clone()),
        ..IndexOptions::default()
    })
    .unwrap();

    let stats = indexer.index_all().unwrap();
    assert!(stats.files_indexed >= 4, "expected at least 4 files indexed");
    assert!(stats.symbols_extracted > 0);
    assert!(stats.callers_extracted > 0);

    let status = indexer.store().status().unwrap();
    assert!(status.symbol_count > 0);
    assert!(status.caller_count > 0);

    let searcher = Searcher::new(SearchOptions {
        root: fixture.clone(),
        index_path: Some(index_path),
        limit: 16,
        lang_filter: None,
        use_embed: false,
        use_tantivy: false,
        use_cloud_embed: false,
    })
    .unwrap();

    let response = searcher.search("process_request").unwrap();
    assert!(!response.hits.is_empty());
    assert!(response.hits.iter().any(|h| {
        h.symbol.as_deref() == Some("process_request")
            || h.callee.as_deref() == Some("process_request")
            || h.excerpt.contains("process_request")
    }));

    let callers = searcher.search("callers:process_request").unwrap();
    assert!(callers.hits.iter().any(|h| h.kind == ast_sgrep_core::search::HitKind::Caller));

    let defs = searcher.search("defs:auth_refresh").unwrap();
    assert!(defs.hits.iter().any(|h| h.kind == ast_sgrep_core::search::HitKind::Def));

    let nl = searcher.search("how does auth refresh work").unwrap();
    assert!(!nl.hits.is_empty());
}

#[test]
fn incremental_reindex_skips_unchanged() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/sample");
    let fixture = fixture.canonicalize().expect("fixture path");

    let temp = TempDir::new().unwrap();
    let index_path = temp.path().join("index.db");

    let opts = IndexOptions {
        root: fixture,
        index_path: Some(index_path),
        ..IndexOptions::default()
    };

    let mut indexer = Indexer::new(opts.clone()).unwrap();
    let first = indexer.index_all().unwrap();
    let second = indexer.index_all().unwrap();
    assert!(first.files_indexed >= 4);
    assert_eq!(second.files_removed, 0);
}

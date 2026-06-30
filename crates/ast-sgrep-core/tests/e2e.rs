//! End-to-end tests covering query modes, indexing edge cases, and FTS safety.

use std::path::PathBuf;

use ast_sgrep_core::search::HitKind;
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use tempfile::TempDir;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/sample")
        .canonicalize()
        .expect("fixture")
}

fn indexed_searcher(no_embed: bool) -> (TempDir, Searcher) {
    let temp = TempDir::new().unwrap();
    let index_path = temp.path().join("index.db");
    let root = fixture();

    let mut indexer = Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        embed_semantic: !no_embed,
        force_reindex: true,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();

    let searcher = Searcher::new(SearchOptions {
        root,
        index_path: Some(index_path),
        limit: 32,
        lang_filter: None,
        use_embed: !no_embed,
        use_tantivy: false,
        use_cloud_embed: false,
        use_ollama_embed: false,
        use_semantic_only: false,
    })
    .unwrap();

    (temp, searcher)
}

#[test]
fn callers_query_uses_exact_symbol_not_sorted_token() {
    let (_temp, searcher) = indexed_searcher(false);
    let response = searcher.search("callers:process_request").unwrap();
    assert!(
        response.hits.iter().any(|h| {
            h.kind == HitKind::Caller
                && h.callee.as_deref() == Some("process_request")
                && h.caller.as_deref() == Some("main")
        }),
        "expected main -> process_request caller edge"
    );
}

#[test]
fn defs_query_preserves_qualified_symbol() {
    let (_temp, searcher) = indexed_searcher(false);
    let response = searcher.search("defs:process_request").unwrap();
    assert!(
        response
            .hits
            .iter()
            .any(|h| h.kind == HitKind::Def && h.symbol.as_deref() == Some("process_request")),
        "expected process_request definition"
    );
}

#[test]
fn hybrid_natural_language_finds_auth_refresh() {
    let (_temp, searcher) = indexed_searcher(false);
    let response = searcher.search("how does auth refresh work").unwrap();
    assert!(!response.hits.is_empty());
    assert!(
        response.hits.iter().any(|h| {
            h.excerpt.contains("auth_refresh") || h.excerpt.contains("authRefresh")
        })
    );
}

#[test]
fn fts_special_characters_do_not_crash() {
    let (_temp, searcher) = indexed_searcher(false);
    for query in ["foo\"bar", "OR AND NOT", "auth*refresh", "(process)"] {
        let result = searcher.search(query);
        assert!(result.is_ok(), "query {query:?} should not error");
    }
}

#[test]
fn force_reindex_reextracts_symbols() {
    let temp = TempDir::new().unwrap();
    let index_path = temp.path().join("index.db");
    let root = fixture();

    let opts = IndexOptions {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        ..IndexOptions::default()
    };

    let mut indexer = Indexer::new(opts.clone()).unwrap();
    let first = indexer.index_all().unwrap();
    assert!(first.symbols_extracted > 0);

    let second = indexer.index_all().unwrap();
    assert_eq!(second.symbols_extracted, 0, "unchanged files should skip");

    let mut force = Indexer::new(IndexOptions {
        force_reindex: true,
        ..opts
    })
    .unwrap();
    let forced = force.reindex_all().unwrap();
    assert!(
        forced.symbols_extracted > 0,
        "force reindex should re-extract symbols"
    );
}

#[test]
fn lang_filter_removes_stale_files_from_index() {
    let temp = TempDir::new().unwrap();
    let index_path = temp.path().join("index.db");
    let root = fixture();

    let mut full = Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        ..IndexOptions::default()
    })
    .unwrap();
    full.index_all().unwrap();
    let full_count = full.store().status().unwrap().file_count;
    assert!(full_count >= 4);

    let mut rust_only = Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        lang_filter: Some("rust".into()),
        force_reindex: true,
        ..IndexOptions::default()
    })
    .unwrap();
    rust_only.index_all().unwrap();
    let rust_count = rust_only.store().status().unwrap().file_count;
    assert!(rust_count < full_count);
    assert!(rust_count >= 1);
}

#[test]
fn embed_pass_runs_by_default() {
    let (_temp, searcher) = indexed_searcher(false);
    let response = searcher.search("credential renewal").unwrap();
    assert!(
        response.hits.iter().any(|h| h.kind == HitKind::Embed)
            || response.hits.iter().any(|h| {
                h.excerpt.contains("auth_refresh") || h.excerpt.contains("authRefresh")
            }),
        "semantic embed or hybrid hits expected by default"
    );
}

#[test]
fn embed_pass_empty_when_disabled() {
    let (_temp, searcher) = indexed_searcher(true);
    let response = searcher.search("credential renewal").unwrap();
    assert!(
        !response.hits.iter().any(|h| h.kind == HitKind::Embed),
        "embed pass should be off with --no-embed"
    );
}

#[test]
fn lexical_sidecar_search_works() {
    let temp = TempDir::new().unwrap();
    let index_path = temp.path().join("index.db");
    let root = fixture();

    let mut indexer = Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        use_tantivy: true,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();

    let searcher = Searcher::new(SearchOptions {
        root,
        index_path: Some(index_path),
        limit: 16,
        lang_filter: None,
        use_embed: false,
        use_tantivy: true,
        use_cloud_embed: false,
        use_ollama_embed: false,
        use_semantic_only: false,
    })
    .unwrap();

    let response = searcher.search("process_request").unwrap();
    assert!(!response.hits.is_empty());
}

#[test]
fn json_output_shape() {
    let (_temp, searcher) = indexed_searcher(false);
    let response = searcher.search("process_request").unwrap();
    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["query"], "process_request");
    assert!(json["hits"].as_array().unwrap().len() > 0);
    assert!(json["hits"][0]["kind"].is_string());
}

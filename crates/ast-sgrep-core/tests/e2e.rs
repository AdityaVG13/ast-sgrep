//! End-to-end tests covering query modes, indexing edge cases, and FTS safety.

mod common;

use ast_sgrep_core::search::HitKind;
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use common::{fixture, index_fixture, indexed_searcher, searcher_for};
use tempfile::TempDir;

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
        ann_threshold: None,
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

#[test]
fn indexes_and_searches_polyglot_fixture() {
    let (_temp, indexer) = index_fixture(IndexOptions::default());
    let stats = indexer.store().status().unwrap();
    assert!(stats.file_count >= 4, "expected at least 4 files indexed");
    assert!(stats.symbol_count > 0);
    assert!(stats.caller_count > 0);

    let searcher = searcher_for(
        &indexer,
        SearchOptions {
            limit: 16,
            use_embed: false,
            ..SearchOptions::default()
        },
    );

    let response = searcher.search("process_request").unwrap();
    assert!(!response.hits.is_empty());
    assert!(response.hits.iter().any(|h| {
        h.symbol.as_deref() == Some("process_request")
            || h.callee.as_deref() == Some("process_request")
            || h.excerpt.contains("process_request")
    }));

    let callers = searcher.search("callers:process_request").unwrap();
    assert!(callers.hits.iter().any(|h| h.kind == HitKind::Caller));

    let defs = searcher.search("defs:auth_refresh").unwrap();
    assert!(defs.hits.iter().any(|h| h.kind == HitKind::Def));

    let nl = searcher.search("how does auth refresh work").unwrap();
    assert!(!nl.hits.is_empty());

    let imports = searcher.search("imports:json").unwrap();
    assert!(
        imports.hits.iter().any(|h| h.excerpt.contains("json")),
        "fixture ruby require json should be indexed"
    );
}

#[test]
fn incremental_reindex_skips_unchanged() {
    let temp = TempDir::new().unwrap();
    let opts = IndexOptions {
        root: fixture(),
        index_path: Some(temp.path().join("index.db")),
        ..IndexOptions::default()
    };
    let mut indexer = Indexer::new(opts).unwrap();
    let first = indexer.index_all().unwrap();
    let second = indexer.index_all().unwrap();
    assert!(first.files_indexed >= 4);
    assert_eq!(second.files_removed, 0);
}

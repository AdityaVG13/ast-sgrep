//! Search and query integration tests.

use ast_sgrep_core::search::HitKind;
use ast_sgrep_core::{IndexOptions, SearchOptions};
use ast_sgrep_testkit::{
    assert_excerpt_contains, assert_has_embed_hits, assert_no_embed_hits, assert_query_finds,
    hybrid_searcher, index_sample, searcher_from, semantic_searcher, top_symbols,
};

#[test]
fn polyglot_search_smoke() {
    let indexed = index_sample(IndexOptions::default());
    let stats = indexed.indexer.store().status().unwrap();
    assert!(stats.file_count >= 4);
    assert!(stats.symbol_count > 0);
    assert!(stats.caller_count > 0);

    let searcher = searcher_from(
        &indexed,
        SearchOptions {
            limit: 16,
            use_embed: false,
            ..SearchOptions::default()
        },
    );

    assert_excerpt_contains(&searcher, "process_request", "process_request");
    assert_query_finds(&searcher, "callers:process_request", |h| h.kind == HitKind::Caller);
    assert_query_finds(&searcher, "defs:auth_refresh", |h| h.kind == HitKind::Def);
    assert_query_finds(&searcher, "how does auth refresh work", |_| true);
    assert_excerpt_contains(&searcher, "imports:json", "json");
}

#[test]
fn query_modes_resolve_symbols() {
    let (_indexed, searcher) = hybrid_searcher(false);

    assert_query_finds(&searcher, "callers:process_request", |h| {
        h.kind == HitKind::Caller
            && h.callee.as_deref() == Some("process_request")
            && h.caller.as_deref() == Some("main")
    });

    assert_query_finds(&searcher, "defs:process_request", |h| {
        h.kind == HitKind::Def && h.symbol.as_deref() == Some("process_request")
    });
}

#[test]
fn semantic_synonym_queries() {
    let (_indexed, searcher) = semantic_searcher(false);

    let cases: &[(&str, &[&str])] = &[
        ("credential renewal", &["auth_refresh"]),
        ("persist access token", &["fetch_token", "store_token", "auth_refresh"]),
        ("sanitize user input", &["validate_input"]),
    ];

    for (query, symbols) in cases {
        let response = searcher.search(query).unwrap();
        assert!(
            symbols.iter().any(|sym| {
                response.hits.iter().any(|h| {
                    h.symbol.as_deref() == Some(*sym) || h.excerpt.contains(sym)
                })
            }),
            "query {query:?} expected one of {symbols:?}; hits: {:?}",
            top_symbols(&response)
        );
    }
}

#[test]
fn fts_queries_do_not_error() {
    let (_indexed, searcher) = hybrid_searcher(false);
    for query in ["foo\"bar", "OR AND NOT", "auth*refresh", "(process)"] {
        assert!(searcher.search(query).is_ok(), "query {query:?} should not error");
    }
}

#[test]
fn embed_enabled_by_default() {
    let (_indexed, searcher) = hybrid_searcher(false);
    assert_has_embed_hits(&searcher, "credential renewal");
}

#[test]
fn embed_disabled_skips_embed_pass() {
    let (_indexed, searcher) = semantic_searcher(true);
    assert_no_embed_hits(&searcher, "credential renewal");
}

#[test]
fn json_output_shape() {
    let (_indexed, searcher) = hybrid_searcher(false);
    let response = searcher.search("process_request").unwrap();
    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["query"], "process_request");
    assert!(json["hits"].as_array().unwrap().iter().len() > 0);
    assert!(json["hits"][0]["kind"].is_string());
}

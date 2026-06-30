//! Semantic regression tests — zero token-overlap queries must hit the right symbols.

mod common;

use ast_sgrep_core::search::HitKind;
use ast_sgrep_core::{IndexOptions, Indexer};
use common::{fixture, semantic_searcher};

fn top_symbols(response: &ast_sgrep_core::SearchResponse) -> Vec<String> {
    response
        .hits
        .iter()
        .filter_map(|h| h.symbol.clone())
        .collect()
}

#[test]
fn credential_renewal_finds_auth_refresh_without_token_overlap() {
    let (_temp, searcher) = semantic_searcher(false);
    let response = searcher.search("credential renewal").unwrap();
    assert!(
        response.hits.iter().any(|h| {
            h.symbol.as_deref() == Some("auth_refresh")
                || h.excerpt.contains("auth_refresh")
                || h.excerpt.contains("authRefresh")
        }),
        "expected auth_refresh for synonym query; hits: {:?}",
        top_symbols(&response)
    );
}

#[test]
fn token_storage_finds_fetch_and_store_symbols() {
    let (_temp, searcher) = semantic_searcher(false);
    let response = searcher.search("persist access token").unwrap();
    assert!(
        response.hits.iter().any(|h| {
            matches!(
                h.symbol.as_deref(),
                Some("fetch_token") | Some("store_token") | Some("auth_refresh")
            )
        }),
        "expected token-related symbols; hits: {:?}",
        top_symbols(&response)
    );
}

#[test]
fn input_validation_finds_validate_input() {
    let (_temp, searcher) = semantic_searcher(false);
    let response = searcher.search("sanitize user input").unwrap();
    assert!(
        response.hits.iter().any(|h| h.symbol.as_deref() == Some("validate_input")),
        "expected validate_input; hits: {:?}",
        top_symbols(&response)
    );
}

#[test]
fn semantic_pass_disabled_with_no_embed() {
    let (_temp, searcher) = semantic_searcher(true);
    let response = searcher.search("credential renewal").unwrap();
    assert!(
        !response.hits.iter().any(|h| h.kind == HitKind::Embed),
        "embed pass should be off when --no-embed"
    );
}

#[test]
fn semantic_chunks_indexed_by_default() {
    let temp = tempfile::TempDir::new().unwrap();
    let index_path = temp.path().join("index.db");
    let root = fixture();

    let mut indexer = Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(index_path),
        force_reindex: true,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    let status = indexer.store().status().unwrap();
    assert!(
        status.semantic_chunk_count > 0,
        "semantic chunks should be indexed by default"
    );
}

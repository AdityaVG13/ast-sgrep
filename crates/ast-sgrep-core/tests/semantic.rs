//! Semantic regression tests — zero token-overlap queries must hit the right symbols.

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
        use_semantic_only: true,
    })
    .unwrap();

    (temp, searcher)
}

fn top_symbols(response: &ast_sgrep_core::SearchResponse) -> Vec<String> {
    response
        .hits
        .iter()
        .filter_map(|h| h.symbol.clone())
        .collect()
}

#[test]
fn credential_renewal_finds_auth_refresh_without_token_overlap() {
    let (_temp, searcher) = indexed_searcher(false);
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
    let (_temp, searcher) = indexed_searcher(false);
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
    let (_temp, searcher) = indexed_searcher(false);
    let response = searcher.search("sanitize user input").unwrap();
    assert!(
        response.hits.iter().any(|h| h.symbol.as_deref() == Some("validate_input")),
        "expected validate_input; hits: {:?}",
        top_symbols(&response)
    );
}

#[test]
fn semantic_pass_disabled_with_no_embed() {
    let (_temp, searcher) = indexed_searcher(true);
    let response = searcher.search("credential renewal").unwrap();
    assert!(
        !response.hits.iter().any(|h| h.kind == HitKind::Embed),
        "embed pass should be off when --no-embed"
    );
}

#[test]
fn semantic_chunks_indexed_by_default() {
    let temp = TempDir::new().unwrap();
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

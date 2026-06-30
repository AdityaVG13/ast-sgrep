#![allow(dead_code)]

use std::path::PathBuf;

use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use tempfile::TempDir;

pub fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/sample")
        .canonicalize()
        .expect("fixture")
}

pub fn indexed_searcher(no_embed: bool) -> (TempDir, Searcher) {
    indexed_searcher_with(no_embed, false)
}

pub fn semantic_searcher(no_embed: bool) -> (TempDir, Searcher) {
    indexed_searcher_with(no_embed, true)
}

fn indexed_searcher_with(no_embed: bool, semantic_only: bool) -> (TempDir, Searcher) {
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
        use_semantic_only: semantic_only,
        ann_threshold: None,
    })
    .unwrap();

    (temp, searcher)
}

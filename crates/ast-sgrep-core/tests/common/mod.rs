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

pub fn index_fixture(mut opts: IndexOptions) -> (TempDir, Indexer) {
    let temp = TempDir::new().unwrap();
    opts.index_path = Some(temp.path().join("index.db"));
    if opts.root.as_os_str() == "." {
        opts.root = fixture();
    }
    let mut indexer = Indexer::new(opts).unwrap();
    indexer.index_all().unwrap();
    (temp, indexer)
}

pub fn searcher_for(indexer: &Indexer, mut opts: SearchOptions) -> Searcher {
    opts.root = indexer.store().root().to_path_buf();
    opts.index_path = Some(indexer.store().db_path().to_path_buf());
    Searcher::new(opts).unwrap()
}

pub fn indexed_searcher(no_embed: bool) -> (TempDir, Searcher) {
    indexed_searcher_with(no_embed, false)
}

pub fn semantic_searcher(no_embed: bool) -> (TempDir, Searcher) {
    indexed_searcher_with(no_embed, true)
}

fn indexed_searcher_with(no_embed: bool, semantic_only: bool) -> (TempDir, Searcher) {
    let (temp, indexer) = index_fixture(IndexOptions {
        embed_semantic: !no_embed,
        force_reindex: true,
        ..IndexOptions::default()
    });
    let searcher = searcher_for(
        &indexer,
        SearchOptions {
            limit: 32,
            use_embed: !no_embed,
            use_semantic_only: semantic_only,
            ..SearchOptions::default()
        },
    );
    (temp, searcher)
}

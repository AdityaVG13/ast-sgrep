//! Pattern routing tests (e9qc) — native union / prefix routing without external ast-grep.
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use std::fs;
use tempfile::TempDir;

fn indexed_rs(body: &str) -> (tempfile::TempDir, tempfile::TempDir, std::path::PathBuf) {
    let corpus = TempDir::new().unwrap();
    fs::write(corpus.path().join("mod.rs"), body).unwrap();
    let index_dir = TempDir::new().unwrap();
    let index_path = index_dir.path().join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    (corpus, index_dir, index_path)
}

#[test]
fn pattern_prefix_routes_to_native_or_index_hits() {
    let (corpus, _idx, index_path) = indexed_rs("fn greet_user() {}\nfn other() { greet_user(); }\n");
    let searcher = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        use_embed: false,
        limit: 32,
        ..SearchOptions::default()
    })
    .unwrap();
    let response = searcher.search("pattern: greet_user").unwrap();
    assert!(
        !response.hits.is_empty(),
        "pattern: greet_user should hit via index signatures and/or native matcher"
    );
}

#[test]
fn exotic_pattern_without_ast_grep_is_structured_empty_not_panic() {
    let (corpus, _idx, index_path) = indexed_rs("fn alpha() {}\n");
    let searcher = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        use_embed: false,
        limit: 8,
        ..SearchOptions::default()
    })
    .unwrap();
    // Deliberately exotic rule syntax — must not panic; empty or structured error via Result.
    let result = searcher.search("pattern: $$$UNLIKELY_EXOTIC_RULE<<<");
    assert!(result.is_ok(), "exotic pattern must not panic: {result:?}");
}

#[test]
fn hybrid_quoted_literal_intent_hits_phrase_line() {
    let (corpus, _idx, index_path) =
        indexed_rs("fn main() {\n    let msg = \"foo bar unique_phrase\";\n}\n");
    let searcher = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        use_embed: false,
        limit: 16,
        ..SearchOptions::default()
    })
    .unwrap();
    let hybrid = searcher.search("\"foo bar unique_phrase\"").unwrap();
    let literal = searcher.search("literal:foo bar unique_phrase").unwrap();
    assert!(
        !literal.hits.is_empty(),
        "literal phrase must hit: {:?}",
        literal.hits
    );
    let lit_line = literal.hits[0].line_start;
    assert!(
        hybrid.hits.iter().any(|h| h.line_start == lit_line),
        "quoted hybrid Literal intent must hit same line as literal: (50hx); hybrid={:?} literal={:?}",
        hybrid.hits,
        literal.hits
    );
}

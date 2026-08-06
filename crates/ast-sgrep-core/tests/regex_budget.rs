//! Wall-clock budget for `regex:` scans (bead ast-sgrep-56w1.3).
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use std::fs;
#[test]
fn regex_pass_errors_when_wall_clock_budget_exhausted() {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    // Many distinct lines so the scanner has work to interrupt between matches.
    let mut body = String::new();
    for i in 0..5_000 {
        body.push_str(&format!("line_{i}_payload_abcdef\n"));
    }
    fs::write(corpus.path().join("big.rs"), body).unwrap();
    let index_path = index_dir.path().join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    // Zero-ms budget forces the between-line deadline check to fire immediately.
    std::env::set_var("ASGREP_REGEX_BUDGET_MS", "0");
    let searcher = Searcher::new(SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path),
        limit: 32,
        use_embed: false,
        ..SearchOptions::default()
    })
    .unwrap();
    let err = searcher
        .search("regex:payload")
        .expect_err("zero budget must fail closed");
    std::env::remove_var("ASGREP_REGEX_BUDGET_MS");
    let msg = err.to_string();
    assert!(
        msg.contains("wall-clock budget") || msg.contains("ASGREP_REGEX_BUDGET_MS"),
        "unexpected error: {msg}"
    );
}

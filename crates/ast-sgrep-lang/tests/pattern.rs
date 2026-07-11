use ast_sgrep_lang::{match_literal_pattern, needs_ast_grep_fallback, Language};
use ast_sgrep_testkit::sample_file;

#[test]
fn literal_pattern_matches_rust_symbol() {
    let source = sample_file("src/main.rs");
    let hits = match_literal_pattern(Language::Rust, &source, "process_request").unwrap();
    assert!(!hits.is_empty());
}

#[test]
fn metavariable_pattern_needs_ast_grep() {
    assert!(needs_ast_grep_fallback("fn $NAME($$$)"));
    assert!(!needs_ast_grep_fallback("process_request"));
}

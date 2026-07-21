use ast_sgrep_lang::{match_literal_pattern, needs_ast_grep_fallback, Language};
use ast_sgrep_testkit::sample_file;
#[test]
fn literal_pattern_matches_rust_symbol() {
    let source = sample_file("src/main.rs");
    let hits = match_literal_pattern(Language::Rust, &source, "process_request").unwrap();
    assert!(!hits.is_empty());
}
#[test]
fn literal_pattern_matching_is_case_sensitive() {
    let source = "fn Foo() {}\nfn foo() {}\nfn FOO() {}\n";
    let upper_camel = match_literal_pattern(Language::Rust, source, "Foo").unwrap();
    let lower = match_literal_pattern(Language::Rust, source, "foo").unwrap();
    let upper = match_literal_pattern(Language::Rust, source, "FOO").unwrap();
    assert!(!upper_camel.is_empty());
    assert!(upper_camel.iter().all(|hit| hit.line_start == 1));
    assert!(!lower.is_empty());
    assert!(lower.iter().all(|hit| hit.line_start == 2));
    assert!(!upper.is_empty());
    assert!(upper.iter().all(|hit| hit.line_start == 3));
}
#[test]
fn literal_pattern_case_mismatch_has_no_match() {
    let source = "fn foo() {}\n";
    assert!(match_literal_pattern(Language::Rust, source, "Foo")
        .unwrap()
        .is_empty());
}
#[test]
fn common_metavariable_patterns_are_native() {
    // Common shapes run in-process; only exotic rules need external ast-grep.
    assert!(!needs_ast_grep_fallback("fn $NAME($$$)"));
    assert!(!needs_ast_grep_fallback("def $NAME"));
    assert!(!needs_ast_grep_fallback("$OBJ.$METHOD($$$)"));
    assert!(!needs_ast_grep_fallback("process_request"));
    assert!(needs_ast_grep_fallback("if ($COND) { $BODY }"));
}

#[test]
fn structural_fn_pattern_matches_rust_source() {
    use ast_sgrep_lang::match_pattern;
    let source = sample_file("src/main.rs");
    let hits = match_pattern(Language::Rust, &source, "fn $NAME($$$)").unwrap();
    assert!(
        !hits.is_empty(),
        "expected native structural matches for fn $NAME($$$)"
    );
}

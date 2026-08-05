use ast_sgrep_lang::{match_literal_pattern, match_pattern, needs_ast_grep_fallback, Language};
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
fn malformed_metavariable_patterns_fall_back_without_panicking() {
    for pattern in ["$)(", "foo($X + 1)", "foo.$M+.bar($$$)", "foo.$M.($$$)"] {
        assert!(needs_ast_grep_fallback(pattern), "{pattern}");
        assert!(
            match_pattern(Language::CSharp, "class Foo {}", pattern)
                .unwrap()
                .is_empty(),
            "{pattern}"
        );
    }
}

#[test]
fn csharp_type_patterns_preserve_declaration_kind() {
    let source = "class Alpha {}\nstruct Beta {}\ninterface Gamma {}\n";
    let class_hits = match_pattern(Language::CSharp, source, "class $NAME").unwrap();
    let struct_hits = match_pattern(Language::CSharp, source, "struct $NAME").unwrap();
    let interface_hits = match_pattern(Language::CSharp, source, "interface $NAME").unwrap();
    let type_hits = match_pattern(Language::CSharp, source, "type $NAME").unwrap();
    assert_eq!(class_hits.len(), 1, "{class_hits:?}");
    assert!(class_hits[0].excerpt.contains("Alpha"));
    assert_eq!(struct_hits.len(), 1, "{struct_hits:?}");
    assert!(struct_hits[0].excerpt.contains("Beta"));
    assert_eq!(interface_hits.len(), 1, "{interface_hits:?}");
    assert!(interface_hits[0].excerpt.contains("Gamma"));
    assert_eq!(type_hits.len(), 3, "{type_hits:?}");
}

#[test]
fn csharp_function_pattern_finds_local_functions() {
    let source = "class Program { void Outer() { int Inner() => 1; } }";
    let hits = match_pattern(Language::CSharp, source, "function $NAME($$$)").unwrap();
    assert!(hits.iter().any(|hit| hit.excerpt.contains("Inner")), "{hits:?}");
}

#[test]
fn csharp_literal_pattern_uses_csharp_fixture() {
    let source = include_str!("fixtures/extract/csharp.cs");
    let hits = match_literal_pattern(Language::CSharp, source, "Render").unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().any(|hit| hit.excerpt.contains("Render")));
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

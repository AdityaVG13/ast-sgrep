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
        assert!(
            needs_ast_grep_fallback(pattern),
            "malformed pattern must use the full parser: {pattern}"
        );
        assert!(
            match_pattern(Language::Rust, "fn foo() {}", pattern)
                .unwrap()
                .is_empty(),
            "malformed pattern must not produce broad native matches: {pattern}"
        );
    }
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

/// difu.5: C# patterns must use the real tree-sitter-c-sharp grammar, not Java.
/// Pre-fix, tree_sitter_language(CSharp) returned the Java grammar, causing
/// misparses of C#-specific syntax. Also is_call_kind lacked invocation_expression
/// so C# calls produced zero pattern signatures.
#[test]
fn csharp_pattern_uses_real_grammar_not_java() {
    use ast_sgrep_lang::{match_pattern, tree_sitter_language};
    // The C# and Java grammars must be different tree-sitter languages.
    let cs_grammar = tree_sitter_language(Language::CSharp);
    let java_grammar = tree_sitter_language(Language::Java);
    assert_ne!(
        cs_grammar, java_grammar,
        "C# and Java must use different tree-sitter grammars"
    );
    let source = "struct Point { public int X; public int Y; }\nclass Program { static void Main() { Helper(); } }\n";
    // Function pattern should match method Main via the C# grammar.
    let hits = match_pattern(Language::CSharp, source, "function $NAME($$$)").unwrap();
    assert!(
        !hits.is_empty(),
        "C# method declarations must match with the real C# grammar; got {hits:?}"
    );
    // Call pattern should match Helper() invocation_expression.
    let call_hits = match_pattern(Language::CSharp, source, "Helper($$$)").unwrap();
    assert!(
        !call_hits.is_empty(),
        "C# invocation_expression must match call pattern; got {call_hits:?}"
    );
}

/// difu.1: Per-language pattern conformance — function patterns must match
/// declarations in all supported languages. This complements the extraction
/// goldens (which test symbol/call/import extraction) by verifying the pattern
/// channel works for every language, not just Rust and C#.
#[test]
fn function_pattern_matches_all_languages() {
    let cases: &[(Language, &str, &str)] = &[
        (Language::Rust, "fn my_func() {}", "my_func"),
        (Language::Python, "def my_func():\n    pass\n", "my_func"),
        (Language::Go, "func myFunc() {}\n", "myFunc"),
        (Language::Java, "class Foo { void myFunc() {} }\n", "myFunc"),
        (
            Language::CSharp,
            "class Foo { void MyFunc() {} }\n",
            "MyFunc",
        ),
        (Language::JavaScript, "function myFunc() {}\n", "myFunc"),
        (
            Language::TypeScript,
            "function myFunc(): void {}\n",
            "myFunc",
        ),
        (Language::Ruby, "def my_func\nend\n", "my_func"),
        (
            Language::Ruby,
            "class Foo\n  def self.my_func\n  end\nend\n",
            "my_func",
        ),
        (Language::Swift, "func myFunc() {}\n", "myFunc"),
        (Language::C, "void my_func(void) {}\n", "my_func"),
        (Language::Cpp, "void my_func() {}\n", "my_func"),
        (Language::Kotlin, "fun myFunc() {}\n", "myFunc"),
        (
            Language::Php,
            "<?php\nfunction my_func() {}\n",
            "my_func",
        ),
    ];
    for &(lang, source, expected_name) in cases {
        let hits = match_pattern(lang, source, "function $NAME($$$)").unwrap();
        assert!(
            !hits.is_empty(),
            "function pattern must match {expected_name} in {lang}; got {hits:?}"
        );
        assert!(
            hits.iter().any(|h| h.excerpt.contains(expected_name)),
            "function pattern hit for {lang} must contain {expected_name}; got {hits:?}"
        );
    }
}

#[test]
fn csharp_struct_pattern_does_not_match_class() {
    let source = r#"
class Alpha {}
struct Beta {}
interface Gamma {}
"#;
    let class_hits = match_pattern(Language::CSharp, source, "class $NAME").unwrap();
    let struct_hits = match_pattern(Language::CSharp, source, "struct $NAME").unwrap();
    let interface_hits = match_pattern(Language::CSharp, source, "interface $NAME").unwrap();
    assert_eq!(class_hits.len(), 1, "got {class_hits:?}");
    assert!(class_hits[0].excerpt.contains("Alpha"), "{class_hits:?}");
    assert!(!class_hits.iter().any(|h| h.excerpt.contains("Beta")));
    assert_eq!(struct_hits.len(), 1, "got {struct_hits:?}");
    assert!(struct_hits[0].excerpt.contains("Beta"), "{struct_hits:?}");
    assert!(!struct_hits.iter().any(|h| h.excerpt.contains("Alpha")));
    assert_eq!(interface_hits.len(), 1, "got {interface_hits:?}");
    assert!(
        interface_hits[0].excerpt.contains("Gamma"),
        "{interface_hits:?}"
    );
}

#[test]
fn swift_struct_pattern_does_not_match_class_or_protocol() {
    let source = r#"
class Alpha {}
struct Beta {}
protocol Gamma {}
"#;
    let class_hits = match_pattern(Language::Swift, source, "class $NAME").unwrap();
    let struct_hits = match_pattern(Language::Swift, source, "struct $NAME").unwrap();
    let interface_hits = match_pattern(Language::Swift, source, "interface $NAME").unwrap();
    assert_eq!(class_hits.len(), 1, "got {class_hits:?}");
    assert!(class_hits[0].excerpt.contains("Alpha"), "{class_hits:?}");
    assert!(!class_hits.iter().any(|h| h.excerpt.contains("Beta")));
    assert_eq!(struct_hits.len(), 1, "got {struct_hits:?}");
    assert!(struct_hits[0].excerpt.contains("Beta"), "{struct_hits:?}");
    assert!(!struct_hits.iter().any(|h| h.excerpt.contains("Alpha")));
    assert_eq!(interface_hits.len(), 1, "got {interface_hits:?}");
    assert!(
        interface_hits[0].excerpt.contains("Gamma"),
        "{interface_hits:?}"
    );
}

#[test]
fn cpp_class_pattern_does_not_match_struct() {
    let source = r#"
class Alpha {};
struct Beta {};
"#;
    let class_hits = match_pattern(Language::Cpp, source, "class $NAME").unwrap();
    let struct_hits = match_pattern(Language::Cpp, source, "struct $NAME").unwrap();
    assert_eq!(class_hits.len(), 1, "got {class_hits:?}");
    assert!(class_hits[0].excerpt.contains("Alpha"), "{class_hits:?}");
    assert!(!class_hits.iter().any(|h| h.excerpt.contains("Beta")));
    assert_eq!(struct_hits.len(), 1, "got {struct_hits:?}");
    assert!(struct_hits[0].excerpt.contains("Beta"), "{struct_hits:?}");
}

#[test]
fn kotlin_class_pattern_does_not_match_interface_or_enum() {
    let source = r#"
class Alpha {}
interface Gamma {}
enum class Beta { A }
"#;
    let class_hits = match_pattern(Language::Kotlin, source, "class $NAME").unwrap();
    let interface_hits = match_pattern(Language::Kotlin, source, "interface $NAME").unwrap();
    assert_eq!(class_hits.len(), 1, "got {class_hits:?}");
    assert!(class_hits[0].excerpt.contains("Alpha"), "{class_hits:?}");
    assert!(!class_hits.iter().any(|h| h.excerpt.contains("Gamma")));
    assert!(!class_hits.iter().any(|h| h.excerpt.contains("Beta")));
    assert_eq!(interface_hits.len(), 1, "got {interface_hits:?}");
    assert!(
        interface_hits[0].excerpt.contains("Gamma"),
        "{interface_hits:?}"
    );
}

#[test]
fn php_class_pattern_does_not_match_interface() {
    let source = r#"
<?php
class Alpha {}
interface Gamma {}
"#;
    let class_hits = match_pattern(Language::Php, source, "class $NAME").unwrap();
    let interface_hits = match_pattern(Language::Php, source, "interface $NAME").unwrap();
    assert_eq!(class_hits.len(), 1, "got {class_hits:?}");
    assert!(class_hits[0].excerpt.contains("Alpha"), "{class_hits:?}");
    assert!(!class_hits.iter().any(|h| h.excerpt.contains("Gamma")));
    assert_eq!(interface_hits.len(), 1, "got {interface_hits:?}");
    assert!(
        interface_hits[0].excerpt.contains("Gamma"),
        "{interface_hits:?}"
    );
}

/// difu.6: Ruby singleton_method nodes must extract as symbols with call ownership.
#[test]
fn ruby_singleton_method_is_visible_to_extraction() {
    use ast_sgrep_lang::ParserRegistry;
    use ast_sgrep_lang::SymbolKind;
    let source = r#"
class Widget
  def self.build(name)
    normalize(name)
  end
end
"#;
    let result = ParserRegistry::new()
        .parse(Language::Ruby, source)
        .unwrap();
    assert!(
        result
            .symbols
            .iter()
            .any(|s| s.name == "build" && s.kind == SymbolKind::Method),
        "singleton_method build must be extracted; got {:?}",
        result.symbols
    );
    assert!(
        result
            .calls
            .iter()
            .any(|c| c.caller == "build" && c.callee == "normalize"),
        "calls inside singleton_method must attribute to build; got {:?}",
        result.calls
    );
}

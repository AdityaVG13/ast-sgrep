//! E1 error-taxonomy inventory for `ast-sgrep-lang`.
//!
//! One test per failure site on the lang public surface, each with a
//! hand-built trigger. Assertions are discriminants only (`is_none` /
//! `is_some` / `is_ok` / emptiness / bools) — never message text.
//!
//! Taxonomy rows (E1-LANG-01..12); oracle-foundry gaps covered here are the
//! depth guard, signature-indexability `None`s, prefilter-literal `None`s,
//! `declaration_prefix` `None`, and the unsupported-language `None` paths.

use ast_sgrep_lang::{
    cached_pattern_signatures, candidate_kind_signatures, classify_native,
    declaration_prefix, detect_language, index_can_serve_pattern, is_pattern_ident,
    is_universal_root_pattern, match_literal_pattern, match_pattern,
    native_pattern_answerable, needs_ast_grep_fallback, required_pattern_literal,
    tree_sitter_language, Language, ParserRegistry,
};
use std::path::Path;

// E1-LANG-01: language-id parsing rejections.
#[test]
fn lang_id_rejections() {
    // Unknown extension / label / blank input -> None.
    assert!(Language::from_extension("fortran").is_none());
    assert!(Language::from_extension("").is_none());
    assert!(Language::parse("").is_none());
    assert!(Language::parse("   ").is_none());
    assert!(Language::parse("fortran").is_none());
    assert!(Language::canonical_filter(None).is_none());
    assert!(Language::canonical_filter(Some("  ")).is_none());
    // Positive controls.
    assert_eq!(Language::from_extension("rs"), Some(Language::Rust));
    assert_eq!(Language::parse("golang"), Some(Language::Go));
    assert_eq!(
        Language::canonical_filter(Some("hpp")).as_deref(),
        Some("cpp")
    );
    // Unknown labels stay infallible through normalize_id (lowercased).
    assert_eq!(Language::normalize_id("Fortran"), "fortran");
}

// E1-LANG-02: unsupported-language detection paths.
#[test]
fn detect_language_unsupported_paths() {
    // Unknown extension, no content, or content matching no shebang rule.
    assert!(detect_language(Path::new("n.fortran"), None).is_none());
    assert!(detect_language(Path::new("n.fortran"), Some("print(1)")).is_none());
    assert!(detect_language(Path::new("Makefile"), None).is_none());
    // Positive controls: known extension and shebang fallbacks.
    assert_eq!(
        detect_language(Path::new("n.py"), None),
        Some(Language::Python)
    );
    assert_eq!(
        detect_language(Path::new("run"), Some("#!/usr/bin/env python\nx=1")),
        Some(Language::Python)
    );
    assert_eq!(
        detect_language(Path::new("run"), Some("<?php echo 1;")),
        Some(Language::Php)
    );
}

// E1-LANG-03: the registry serves every language; its `Err` side
// ("no parser registered", "failed to set language", "failed to parse
// source") is unreachable via the public API, so pin `Ok` on hostile input.
#[test]
fn registry_parses_every_language_ok() {
    let registry = ParserRegistry::new();
    for lang in Language::all() {
        assert!(registry.parse(*lang, "").is_ok(), "{lang}");
        assert!(registry.parse(*lang, "{{{{\n\x00\x01garbage").is_ok(), "{lang}");
    }
}

// E1-LANG-04: extraction never errors on empty/garbage source; it fails
// closed with empty rows.
#[test]
fn extraction_empty_and_garbage_fail_closed() {
    let registry = ParserRegistry::new();
    let empty = registry.parse(Language::Rust, "").unwrap();
    assert!(empty.symbols.is_empty());
    assert!(empty.calls.is_empty());
    assert!(empty.imports.is_empty());
    assert!(!empty.depth_truncated);
    let garbage = registry.parse(Language::Rust, "{{{{ !! not code").unwrap();
    assert!(garbage.symbols.is_empty());
    assert!(!garbage.depth_truncated);
    // Positive control: a real item still extracts.
    let real = registry.parse(Language::Rust, "fn foo() {}").unwrap();
    assert_eq!(real.symbols.len(), 1);
    assert!(!real.depth_truncated);
}

// E1-LANG-05: the MAX_EXTRACTION_DEPTH (256) threshold guard.
#[test]
fn depth_guard_threshold() {
    let registry = ParserRegistry::new();
    let shallow_src = "fn f() { let x = (1 + (2 * 3)); }";
    assert!(!registry.parse(Language::Rust, shallow_src).unwrap().depth_truncated);
    // 300-deep paren nesting breaches the 256 cap.
    let deep_src = format!("fn f() {{ let x = {}1{}; }}", "(".repeat(300), ")".repeat(300));
    assert!(registry.parse(Language::Rust, &deep_src).unwrap().depth_truncated);
}

// E1-LANG-06: classifier rejections (exotic shapes -> None).
#[test]
fn classify_native_rejections() {
    // Fallback-loud in the oracle suite implies classifier rejection.
    assert!(classify_native("greet($Ü)").is_none());
    assert!(classify_native("def a(x): $$$B").is_none());
    assert!(classify_native("$A $B").is_none());
    // Positive control: a call shape classifies.
    assert!(classify_native("foo($$$)").is_some());
}

// E1-LANG-07: index-signature indexability gaps.
#[test]
fn cached_and_candidate_signature_gaps() {
    // Braced body templates and multi-segment chains are not indexable.
    assert!(cached_pattern_signatures("fn $N($$$) { $B }").is_none());
    assert!(cached_pattern_signatures("fetch()?.$M($$$A)").is_none());
    assert!(cached_pattern_signatures("a.b($$$C).d()").is_none());
    // Malformed declaration tails stay match-none upstream (None here).
    assert!(cached_pattern_signatures("fn $3").is_none());
    // Empty pattern is indexable-to-nothing, not a rejection.
    assert_eq!(cached_pattern_signatures(""), Some(vec![]));
    // Positive controls: concrete callee addresses one call: row,
    // metavariable callee addresses the kind rows.
    assert_eq!(cached_pattern_signatures("foo($$$)").unwrap(), vec!["call:foo".to_string()]);
    assert_eq!(
        cached_pattern_signatures("$F($$$)").unwrap(),
        vec!["kind:call_expression".to_string(), "kind:call".to_string()]
    );
    // Candidate kinds only serve declaration prefixes.
    assert!(candidate_kind_signatures("").is_none());
    assert!(candidate_kind_signatures("foo($$$)").is_none());
    assert!(candidate_kind_signatures("fn $N($$$)").is_some());
}

// E1-LANG-08: SIMD prefilter literal absence.
#[test]
fn required_pattern_literal_absent() {
    assert!(required_pattern_literal("").is_none());
    // Metavariable-only callee: no usable byte literal.
    assert!(required_pattern_literal("$O.$M($$$)").is_none());
    // Declaration keyword + meta name: keywords are never literals.
    assert!(required_pattern_literal("fn $N($$$)").is_none());
    // All-comment pattern: comment text is invisible to the matcher.
    assert!(required_pattern_literal("// only a comment").is_none());
    // Positive controls.
    assert_eq!(required_pattern_literal("foo($$$)").as_deref(), Some("foo"));
    assert_eq!(required_pattern_literal("foo").as_deref(), Some("foo"));
}

// E1-LANG-09: serve/answer gate denials.
#[test]
fn serve_and_answer_gates() {
    // index_can_serve_pattern denials.
    assert!(!index_can_serve_pattern("foo", &[]));
    assert!(!index_can_serve_pattern("fn $N", &["kind:function_item".to_string()]));
    assert!(!index_can_serve_pattern("return", &["return".to_string()]));
    assert!(index_can_serve_pattern("foo", &["foo".to_string()]));
    // Degenerate `;;` roots are unanswerable outside py/swift/kt.
    assert!(!native_pattern_answerable(Language::Rust, ";;"));
    assert!(native_pattern_answerable(Language::Python, ";;"));
    // Keyword-literal roots route to the walk, never ident-serve.
    assert!(ast_sgrep_lang::pattern_is_keyword_literal_root("null"));
    assert!(!ast_sgrep_lang::pattern_is_keyword_literal_root("foo"));
    // Identifier-shape gate.
    assert!(is_pattern_ident("foo"));
    assert!(!is_pattern_ident("foo bar"));
    assert!(!is_pattern_ident(""));
    // Universal-root gate.
    assert!(is_universal_root_pattern(Language::Rust, "$$A"));
    assert!(!is_universal_root_pattern(Language::Rust, "$A"));
}

// E1-LANG-10: the loud fail-closed class (needs external ast-grep).
#[test]
fn fallback_loud_class() {
    assert!(needs_ast_grep_fallback("if ($COND) { $A; $B }"));
    assert!(needs_ast_grep_fallback("greet($Ü)"));
    assert!(!needs_ast_grep_fallback(""));
    assert!(!needs_ast_grep_fallback("process_request"));
    assert!(!needs_ast_grep_fallback("fn $NAME($$$)"));
}

// E1-LANG-11: match entry points return Ok(empty) on rejected shapes,
// never Err, even on garbage source.
#[test]
fn match_ok_empty_rejections() {
    // Empty pattern.
    assert!(match_pattern(Language::Rust, "fn foo() {}", "").unwrap().is_empty());
    assert!(match_literal_pattern(Language::Rust, "fn foo() {}", "").unwrap().is_empty());
    // Bare-connector garbage class.
    assert!(match_pattern(Language::Rust, "fn foo() {}", "->").unwrap().is_empty());
    assert!(match_pattern(Language::Rust, "fn foo() {}", "::").unwrap().is_empty());
    // PHP-only garbage spellings.
    assert!(match_pattern(Language::Php, "<?php $a && $b;", "&&").unwrap().is_empty());
    assert!(match_pattern(Language::Php, "<?php $a && $b;", ".").unwrap().is_empty());
    // Garbage source is Ok (per-file robustness), never Err.
    assert!(match_pattern(Language::Rust, "{{{{ !!", "foo").is_ok());
    // Positive control: a literal hit is found.
    assert!(!match_pattern(Language::Rust, "fn foo() {}", "foo").unwrap().is_empty());
}

// E1-LANG-12: declaration_prefix is None for non-declaration nodes.
#[test]
fn declaration_prefix_non_decl_none() {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_language(Language::Rust)).unwrap();
    let tree = parser.parse("fn foo() { bar(); }", None).unwrap();
    let root = tree.root_node();
    // The function_item node has a prefix; the identifier / call nodes do not.
    let func = root.named_child(0).unwrap();
    assert_eq!(func.kind(), "function_item");
    assert_eq!(declaration_prefix(&func, "fn foo() { bar(); }"), Some("fn"));
    let name = func.child_by_field_name("name").unwrap();
    assert!(declaration_prefix(&name, "fn foo() { bar(); }").is_none());
}

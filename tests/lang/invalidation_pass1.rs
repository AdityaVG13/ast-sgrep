//! I1 invalidation contracts for ast-sgrep-lang.
//!
//! The crate holds seven append-only memo/reuse caches behind `pub(crate)`
//! (thread-local parser maps in `extract`/`templates`, the process-wide
//! `SUPPORTED` gate memo and compiled-`Query` cache, and the per-thread
//! general/literal/if-cond template maps) with NO invalidation or refresh
//! API. The contract is therefore: every cache is a pure function of its
//! key, so reuse is behaviorally unobservable — sequential parses never leak
//! state, mutated sources re-read fresh, and repeated queries are identical.
//! These tests pin that contract through the public API only.

use ast_sgrep_lang::{
    cached_pattern_signatures, index_can_serve_pattern, match_literal_pattern, match_pattern,
    native_pattern_answerable, needs_ast_grep_fallback, Language, ParserRegistry,
};
use std::collections::HashSet;

fn symbol_names(result: &ast_sgrep_lang::ExtractionResult) -> Vec<&str> {
    result
        .symbols
        .iter()
        .map(|s| s.name.as_str())
        .collect::<Vec<_>>()
}

#[test]
fn parser_reuse_returns_identical_extraction() {
    let registry = ParserRegistry::new();
    let source = "fn alpha() {}\nfn beta() {}\n";
    let first = registry.parse(Language::Rust, source).unwrap();
    let second = registry.parse(Language::Rust, source).unwrap();
    assert_eq!(first, second);
    assert_eq!(symbol_names(&first), vec!["alpha", "beta"]);
}

#[test]
fn interleaved_sources_do_not_leak_symbols() {
    let registry = ParserRegistry::new();
    let src_a = "fn alpha() {}\n";
    let src_b = "fn beta() {}\n";
    let a1 = registry.parse(Language::Rust, src_a).unwrap();
    let b = registry.parse(Language::Rust, src_b).unwrap();
    let a2 = registry.parse(Language::Rust, src_a).unwrap();
    // The B parse in the middle must not stain the re-parse of A.
    assert_eq!(a1, a2);
    assert_eq!(symbol_names(&a1), vec!["alpha"]);
    assert_eq!(symbol_names(&b), vec!["beta"]);
    assert_ne!(a1, b);
}

#[test]
fn interleaved_languages_do_not_pollute_parsers() {
    let registry = ParserRegistry::new();
    let rust_src = "fn alpha() {}\n";
    let py_src = "def beta():\n    pass\n";
    let r1 = registry.parse(Language::Rust, rust_src).unwrap();
    let p = registry.parse(Language::Python, py_src).unwrap();
    let r2 = registry.parse(Language::Rust, rust_src).unwrap();
    assert_eq!(r1, r2);
    assert_eq!(symbol_names(&r1), vec!["alpha"]);
    assert_eq!(symbol_names(&p), vec!["beta"]);
}

#[test]
fn match_pattern_tracks_source_mutations() {
    // Same pattern, evolving source: results must follow the CURRENT source,
    // never the first-seen parse, and re-querying v1 must equal the original.
    let v1 = "fn foo() {}\n";
    let v2 = "fn foo() {}\nfn foo() {}\n";
    let hits_v1 = match_pattern(Language::Rust, v1, "foo").unwrap();
    let hits_v2 = match_pattern(Language::Rust, v2, "foo").unwrap();
    let hits_v1_again = match_pattern(Language::Rust, v1, "foo").unwrap();
    assert!(!hits_v1.is_empty());
    assert!(hits_v1.iter().all(|h| h.line_start == 1));
    let lines_v2: HashSet<u32> = hits_v2.iter().map(|h| h.line_start).collect();
    assert_eq!(lines_v2, HashSet::from([1, 2]));
    assert!(hits_v2.len() > hits_v1.len());
    assert_eq!(hits_v1, hits_v1_again);
}

#[test]
fn match_pattern_repeatable_across_interleaved_patterns() {
    let source = "fn Foo() {}\nfn foo() {}\nfn FOO() {}\n";
    let lower_first = match_pattern(Language::Rust, source, "foo").unwrap();
    // Interleave unrelated literal + structural queries (populates other
    // template/parser-cache slots) before re-querying.
    let _ = match_pattern(Language::Rust, source, "Foo").unwrap();
    let _ = match_pattern(Language::Rust, source, "fn $NAME($$$)").unwrap();
    let _ = match_literal_pattern(Language::Rust, source, "FOO").unwrap();
    let lower_again = match_pattern(Language::Rust, source, "foo").unwrap();
    assert_eq!(lower_first, lower_again);
    assert!(!lower_first.is_empty());
    assert!(lower_first.iter().all(|h| h.line_start == 2));
}

#[test]
fn same_pattern_text_is_keyed_by_language() {
    // Poison order: populate the Python slots first; the Rust query for the
    // identical pattern text must still answer Rust-correctly (and vice versa
    // for a Python-native structural shape).
    let rust_src = "fn foo() {}\n";
    let py_src = "def bar():\n    pass\n";
    let py_hits = match_pattern(Language::Python, py_src, "foo").unwrap();
    let rust_hits = match_pattern(Language::Rust, rust_src, "foo").unwrap();
    assert!(py_hits.is_empty());
    assert!(!rust_hits.is_empty());
    assert!(rust_hits.iter().all(|h| h.line_start == 1));
    assert_eq!(
        rust_hits,
        match_pattern(Language::Rust, rust_src, "foo").unwrap()
    );

    let rust_fn_src = "fn alpha() {}\n";
    let py_def_src = "def beta():\n    pass\n";
    // Population-order independence: this thread queries Rust-first while a
    // fresh thread (fresh thread-local template slots) queries Python-first;
    // per-language answers must be identical under both orders.
    let rust_first = match_pattern(Language::Rust, rust_fn_src, "def $NAME").unwrap();
    let py_after = match_pattern(Language::Python, py_def_src, "def $NAME").unwrap();
    assert!(!py_after.is_empty());
    let reversed = std::thread::scope(|s| {
        s.spawn(|| {
            let py_first = match_pattern(Language::Python, py_def_src, "def $NAME").unwrap();
            let rust_after = match_pattern(Language::Rust, rust_fn_src, "def $NAME").unwrap();
            (rust_after, py_first)
        })
        .join()
        .unwrap()
    });
    assert_eq!((rust_first, py_after), reversed);
}

#[test]
fn fallback_gate_memo_is_stable_under_repetition() {
    // `needs_ast_grep_fallback` consults the process-wide SUPPORTED memo on
    // the general-lane path; repetition with interleaved patterns must not
    // flip any verdict.
    assert!(needs_ast_grep_fallback("if ($COND) { $A; $B }"));
    assert!(!needs_ast_grep_fallback("fn $NAME($$$)"));
    assert!(!needs_ast_grep_fallback("process_request"));
    assert!(needs_ast_grep_fallback("foo.$M+.bar($$$)"));
    assert!(needs_ast_grep_fallback("if ($COND) { $A; $B }"));
    assert!(!needs_ast_grep_fallback("fn $NAME($$$)"));
    assert!(!needs_ast_grep_fallback("process_request"));
    assert!(needs_ast_grep_fallback("foo.$M+.bar($$$)"));
}

#[test]
fn signature_helpers_are_pure_across_repetition() {
    let ident_once = cached_pattern_signatures("SearchHit").unwrap();
    let decl_once = cached_pattern_signatures("fn greet_user").unwrap();
    let kind_once = cached_pattern_signatures("fn $NAME").unwrap();
    assert_eq!(ident_once, cached_pattern_signatures("SearchHit").unwrap());
    assert_eq!(decl_once, cached_pattern_signatures("fn greet_user").unwrap());
    assert_eq!(kind_once, cached_pattern_signatures("fn $NAME").unwrap());
    // Discriminant: distinct pattern shapes yield distinct signature rows.
    assert_ne!(ident_once, decl_once);
    assert_ne!(ident_once, kind_once);
    assert!(index_can_serve_pattern("SearchHit", &ident_once));
    assert!(!index_can_serve_pattern("fn $NAME", &kind_once));
    assert!(index_can_serve_pattern("SearchHit", &ident_once));
    assert!(!index_can_serve_pattern("fn $NAME", &kind_once));
}

#[test]
fn native_answerability_stable_across_languages() {
    // The gate consults per-(language, pattern) template caches; interleaved
    // cross-language consults must not flip any verdict.
    let rust_fn = native_pattern_answerable(Language::Rust, "fn $NAME($$$)");
    let py_fn = native_pattern_answerable(Language::Python, "fn $NAME($$$)");
    let py_def = native_pattern_answerable(Language::Python, "def $NAME");
    let rust_def = native_pattern_answerable(Language::Rust, "def $NAME");
    assert_eq!(
        rust_fn,
        native_pattern_answerable(Language::Rust, "fn $NAME($$$)")
    );
    assert_eq!(
        py_fn,
        native_pattern_answerable(Language::Python, "fn $NAME($$$)")
    );
    assert_eq!(
        py_def,
        native_pattern_answerable(Language::Python, "def $NAME")
    );
    assert_eq!(
        rust_def,
        native_pattern_answerable(Language::Rust, "def $NAME")
    );
}

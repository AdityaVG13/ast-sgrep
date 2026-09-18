//! I2 delta-discriminating tests for ast-sgrep-lang memo caches.
//!
//! I1 pinned the reuse-is-unobservable contract (sequential parses never leak
//! state, mutated sources re-read fresh, repeated queries identical). I2 proves
//! the PER-KEY delta behavior of the same seven append-only caches (thread-local
//! parser maps in `extract`/`templates`, the process-wide `SUPPORTED` gate memo
//! and compiled-`Query` cache, and the per-thread general/literal/if-cond
//! template maps), through the public API only:
//!
//! - distinct languages/keys do not cross-contaminate (per-key deltas);
//! - repeated same-key use is consistent under reversed population order;
//! - unsupported-first vs supported-first gate behavior holds per key;
//! - cache growth does not change earlier results (append-only safety).
//!
//! All assertions are discriminant values (lines, names, verdicts, rows) —
//! never message text.

use ast_sgrep_lang::{
    cached_pattern_signatures, index_can_serve_pattern, match_literal_pattern, match_pattern,
    native_pattern_answerable, needs_ast_grep_fallback, Language, ParserRegistry,
};

fn symbol_names(result: &ast_sgrep_lang::ExtractionResult) -> Vec<String> {
    result
        .symbols
        .iter()
        .map(|s| s.name.clone())
        .collect::<Vec<_>>()
}

fn lines(hits: &[ast_sgrep_lang::PatternMatch]) -> Vec<u32> {
    hits.iter().map(|h| h.line_start).collect::<Vec<_>>()
}

#[test]
fn parser_keys_are_per_language_under_growth() {
    let registry = ParserRegistry::new();
    let rust_src = "fn alpha() {}\n";
    let py_src = "def beta():\n    pass\n";
    let js_src = "function gamma() {}\n";
    let rust_before = registry.parse(Language::Rust, rust_src).unwrap();
    let py_before = registry.parse(Language::Python, py_src).unwrap();
    let js_before = registry.parse(Language::JavaScript, js_src).unwrap();
    assert_eq!(symbol_names(&rust_before), vec!["alpha"]);
    assert_eq!(symbol_names(&py_before), vec!["beta"]);
    assert_eq!(symbol_names(&js_before), vec!["gamma"]);
    // Growth: populate parser slots for further language keys.
    let go_src = "package main\n\nfunc delta() {}\n";
    let ts_src = "function epsilon() {}\n";
    let go_first = registry.parse(Language::Go, go_src).unwrap();
    let ts_first = registry.parse(Language::TypeScript, ts_src).unwrap();
    assert_eq!(go_first, registry.parse(Language::Go, go_src).unwrap());
    assert_eq!(ts_first, registry.parse(Language::TypeScript, ts_src).unwrap());
    // Earlier keys are identical after growth; cross-key delta holds.
    assert_eq!(rust_before, registry.parse(Language::Rust, rust_src).unwrap());
    assert_eq!(py_before, registry.parse(Language::Python, py_src).unwrap());
    assert_eq!(
        js_before,
        registry.parse(Language::JavaScript, js_src).unwrap()
    );
    assert_ne!(symbol_names(&rust_before), symbol_names(&py_before));
    assert_ne!(symbol_names(&rust_before), symbol_names(&js_before));
    assert_ne!(symbol_names(&py_before), symbol_names(&js_before));
}

#[test]
fn literal_keys_are_case_isolated() {
    let source = "fn Foo() {}\nfn foo() {}\nfn FOO() {}\n";
    let query = |pattern: &str| {
        lines(&match_pattern(Language::Rust, source, pattern).unwrap())
    };
    let foo_first = query("foo");
    let big_first = query("Foo");
    let caps_first = query("FOO");
    assert_eq!(foo_first, vec![2]);
    assert_eq!(big_first, vec![1]);
    assert_eq!(caps_first, vec![3]);
    // Reverse population order plus growth keys, then re-read every key.
    let _ = query("FOO");
    let _ = query("Foo");
    let _ = query("foo");
    let _ = query("alpha");
    let _ = query("beta");
    let _ = query("gamma");
    assert_eq!(query("foo"), foo_first);
    assert_eq!(query("Foo"), big_first);
    assert_eq!(query("FOO"), caps_first);
}

#[test]
fn literal_keys_are_language_isolated() {
    let rust_src = "fn alpha() {}\n";
    let py_src = "def beta():\n    pass\n";
    // Poison order: populate the Python (language, pattern) slot first.
    let py_first = match_literal_pattern(Language::Python, py_src, "alpha").unwrap();
    let rust_first = match_literal_pattern(Language::Rust, rust_src, "alpha").unwrap();
    assert!(py_first.is_empty());
    assert_eq!(lines(&rust_first), vec![1]);
    // Reverse order, then growth across further (language, pattern) keys.
    let rust_again = match_literal_pattern(Language::Rust, rust_src, "alpha").unwrap();
    let py_again = match_literal_pattern(Language::Python, py_src, "alpha").unwrap();
    assert_eq!(rust_first, rust_again);
    assert_eq!(py_first, py_again);
    let _ = match_literal_pattern(Language::Rust, rust_src, "beta").unwrap();
    let _ = match_literal_pattern(Language::Python, py_src, "beta").unwrap();
    let _ = match_literal_pattern(Language::Go, "package main\nfunc alpha() {}\n", "alpha").unwrap();
    assert_eq!(
        rust_first,
        match_literal_pattern(Language::Rust, rust_src, "alpha").unwrap()
    );
    assert!(
        match_literal_pattern(Language::Python, py_src, "alpha")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn class_query_keys_are_keyword_isolated() {
    let source = "struct Widget {}\ntrait Greet {}\n";
    let strukt = match_pattern(Language::Rust, source, "struct $NAME").unwrap();
    let iface = match_pattern(Language::Rust, source, "interface $NAME").unwrap();
    assert_eq!(lines(&strukt), vec![1]);
    assert_eq!(lines(&iface), vec![2]);
    // Reverse order plus repetition: keyword-indexed query keys never merge.
    let iface2 = match_pattern(Language::Rust, source, "interface $NAME").unwrap();
    let strukt2 = match_pattern(Language::Rust, source, "struct $NAME").unwrap();
    assert_eq!(strukt, strukt2);
    assert_eq!(iface, iface2);
    let _ = match_pattern(Language::Rust, source, "type $NAME").unwrap();
    assert_eq!(
        strukt,
        match_pattern(Language::Rust, source, "struct $NAME").unwrap()
    );
    assert_eq!(
        iface,
        match_pattern(Language::Rust, source, "interface $NAME").unwrap()
    );
}

#[test]
fn class_query_keys_are_language_isolated() {
    let py_src = "class Widget:\n    pass\n";
    let rust_src = "struct Widget {}\n";
    // Populate the empty (Rust, class-query) slot first: must not poison Python.
    let rust_first = match_pattern(Language::Rust, rust_src, "class $NAME").unwrap();
    let py_first = match_pattern(Language::Python, py_src, "class $NAME").unwrap();
    assert!(rust_first.is_empty());
    assert_eq!(lines(&py_first), vec![1]);
    // Reverse order plus growth, then re-read both language keys.
    let py_again = match_pattern(Language::Python, py_src, "class $NAME").unwrap();
    let rust_again = match_pattern(Language::Rust, rust_src, "class $NAME").unwrap();
    assert_eq!(py_first, py_again);
    assert_eq!(rust_first, rust_again);
    let _ = match_pattern(Language::JavaScript, "class Widget {}\n", "class $NAME").unwrap();
    let _ = match_pattern(Language::Rust, rust_src, "struct $NAME").unwrap();
    assert_eq!(
        py_first,
        match_pattern(Language::Python, py_src, "class $NAME").unwrap()
    );
    assert!(
        match_pattern(Language::Rust, rust_src, "class $NAME")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn supported_gate_holds_per_pattern_verdicts() {
    // Per-key delta: classify-accepted (supported) vs identifier soup (unsupported).
    assert!(!needs_ast_grep_fallback("class $NAME"));
    assert!(needs_ast_grep_fallback("$A $B"));
    assert!(!needs_ast_grep_fallback("foo($$$)"));
    assert!(needs_ast_grep_fallback("$FOO $BAR"));
    // Unsupported-first order must not poison the supported keys.
    assert!(needs_ast_grep_fallback("$A $B"));
    assert!(!needs_ast_grep_fallback("class $NAME"));
    assert!(needs_ast_grep_fallback("$FOO $BAR"));
    assert!(!needs_ast_grep_fallback("foo($$$)"));
    // Supported-first order: identical per-key verdicts.
    assert!(!needs_ast_grep_fallback("foo($$$)"));
    assert!(!needs_ast_grep_fallback("class $NAME"));
    assert!(needs_ast_grep_fallback("$A $B"));
    assert!(needs_ast_grep_fallback("$FOO $BAR"));
}

#[test]
fn supported_gate_stable_under_cache_growth() {
    let panel = [
        "class $NAME",
        "$A $B",
        "foo($$$)",
        "$FOO $BAR",
        "fn $NAME($$$)",
        "process_request",
    ];
    let before: Vec<bool> = panel.iter().map(|p| needs_ast_grep_fallback(p)).collect();
    assert_eq!(before, vec![false, true, false, true, false, false]);
    // Growth: twenty fresh keys across supported/unsupported genera.
    let growth = [
        "alpha",
        "beta",
        "gamma",
        "fn alpha",
        "def beta",
        "class Gamma",
        "struct Delta",
        "foo($A)",
        "bar($A, $B)",
        "$X $Y $Z",
        "$P + $Q",
        "return $X",
        "if ($C) { $B }",
        "while ($C) { $B }",
        "$O.$M($$$)",
        "a.b($$$)",
        "interface $N",
        "type $T",
        "func $F",
        "function $G",
    ];
    for pattern in growth {
        let _ = needs_ast_grep_fallback(pattern);
    }
    let after: Vec<bool> = panel.iter().map(|p| needs_ast_grep_fallback(p)).collect();
    assert_eq!(before, after);
}

#[test]
fn native_answerability_is_per_language_keyed() {
    // Same concrete-cond if pattern, different language keys: JavaScript is a
    // covered grammar with a buildable cond template; Swift is uncovered.
    assert!(native_pattern_answerable(
        Language::JavaScript,
        "if (alpha) { $B }"
    ));
    assert!(!native_pattern_answerable(Language::Swift, "if (alpha) { $B }"));
    // Reverse order plus repetition: per-key verdicts never flip.
    assert!(!native_pattern_answerable(Language::Swift, "if (alpha) { $B }"));
    assert!(native_pattern_answerable(
        Language::JavaScript,
        "if (alpha) { $B }"
    ));
    // Growth across other (language, pattern) keys leaves both verdicts pinned.
    let _ = native_pattern_answerable(Language::Rust, "if (alpha) { $B }");
    let _ = native_pattern_answerable(Language::JavaScript, "if (beta) { $B }");
    let _ = native_pattern_answerable(Language::Python, "class $NAME");
    assert!(native_pattern_answerable(
        Language::JavaScript,
        "if (alpha) { $B }"
    ));
    assert!(!native_pattern_answerable(Language::Swift, "if (alpha) { $B }"));
}

#[test]
fn general_template_keys_are_pattern_isolated() {
    let source = "function f() {\n  return 1;\n  return 2;\n}\n";
    let ret = match_pattern(Language::JavaScript, source, "return $X").unwrap();
    let thr = match_pattern(Language::JavaScript, source, "throw $X").unwrap();
    assert_eq!(lines(&ret), vec![2, 3]);
    assert!(thr.is_empty());
    // Reverse order plus growth keys, then re-read both pattern keys.
    let thr2 = match_pattern(Language::JavaScript, source, "throw $X").unwrap();
    let ret2 = match_pattern(Language::JavaScript, source, "return $X").unwrap();
    assert_eq!(ret, ret2);
    assert_eq!(thr, thr2);
    let _ = match_pattern(Language::JavaScript, source, "$P + $Q").unwrap();
    let _ = match_pattern(Language::JavaScript, source, "yield $X").unwrap();
    let _ = match_pattern(Language::Rust, "fn f() { return 1; }\n", "return $X").unwrap();
    assert_eq!(
        ret,
        match_pattern(Language::JavaScript, source, "return $X").unwrap()
    );
    assert!(
        match_pattern(Language::JavaScript, source, "throw $X")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn if_cond_keys_are_cond_isolated() {
    let source = "if (alpha) { foo(); }\nif (beta) { bar(); }\n";
    let alpha = match_pattern(Language::JavaScript, source, "if (alpha) { $$$ }").unwrap();
    let beta = match_pattern(Language::JavaScript, source, "if (beta) { $$$ }").unwrap();
    let meta = match_pattern(Language::JavaScript, source, "if ($C) { $$$ }").unwrap();
    assert_eq!(lines(&alpha), vec![1]);
    assert_eq!(lines(&beta), vec![2]);
    assert_eq!(lines(&meta), vec![1, 2]);
    // Reverse order plus a further cond key, then re-read.
    let beta2 = match_pattern(Language::JavaScript, source, "if (beta) { $$$ }").unwrap();
    let alpha2 = match_pattern(Language::JavaScript, source, "if (alpha) { $$$ }").unwrap();
    assert_eq!(alpha, alpha2);
    assert_eq!(beta, beta2);
    let gamma = match_pattern(Language::JavaScript, source, "if (gamma) { $$$ }").unwrap();
    assert!(gamma.is_empty());
    assert_eq!(
        lines(&match_pattern(Language::JavaScript, source, "if (alpha) { $$$ }").unwrap()),
        vec![1]
    );
    assert_eq!(
        lines(&match_pattern(Language::JavaScript, source, "if ($C) { $$$ }").unwrap()),
        vec![1, 2]
    );
}

#[test]
fn signature_keys_are_per_pattern_stable_under_growth() {
    let widget = cached_pattern_signatures("Widget").unwrap();
    let gadget = cached_pattern_signatures("Gadget").unwrap();
    let decl_a = cached_pattern_signatures("fn alpha").unwrap();
    let decl_b = cached_pattern_signatures("fn beta").unwrap();
    let call_f = cached_pattern_signatures("foo($$$)").unwrap();
    let call_b = cached_pattern_signatures("bar($$$)").unwrap();
    let kind = cached_pattern_signatures("fn $NAME").unwrap();
    assert_eq!(widget, vec!["Widget".to_string()]);
    assert_eq!(gadget, vec!["Gadget".to_string()]);
    assert_eq!(decl_a, vec!["decl:fn:alpha".to_string()]);
    assert_eq!(decl_b, vec!["decl:fn:beta".to_string()]);
    assert_eq!(call_f, vec!["call:foo".to_string()]);
    assert_eq!(call_b, vec!["call:bar".to_string()]);
    assert!(!kind.is_empty());
    assert!(kind.iter().all(|s| s.starts_with("kind:")));
    assert_ne!(widget, gadget);
    assert_ne!(decl_a, decl_b);
    assert_ne!(call_f, call_b);
    assert!(index_can_serve_pattern("Widget", &widget));
    assert!(index_can_serve_pattern("fn alpha", &decl_a));
    assert!(index_can_serve_pattern("foo($$$)", &call_f));
    assert!(!index_can_serve_pattern("fn $NAME", &kind));
    // Growth across fresh keys, then re-read the whole panel.
    for pattern in [
        "Alpha",
        "Beta",
        "fn gamma",
        "def delta",
        "qux($$$)",
        "class Omega",
        "$X $Y",
    ] {
        let _ = cached_pattern_signatures(pattern);
    }
    assert_eq!(widget, cached_pattern_signatures("Widget").unwrap());
    assert_eq!(decl_a, cached_pattern_signatures("fn alpha").unwrap());
    assert_eq!(call_f, cached_pattern_signatures("foo($$$)").unwrap());
    assert_eq!(kind, cached_pattern_signatures("fn $NAME").unwrap());
    assert!(index_can_serve_pattern("Widget", &widget));
    assert!(!index_can_serve_pattern("fn $NAME", &kind));
}

//! Lang template-cache contract suite: general/literal/if-cond template maps.
//!
//! Contract under test: the per-thread general/literal/if-cond template maps
//! are append-only with NO invalidation API — every cache is a pure function
//! of its key, pinned through the public API only. Discriminants are line
//! vectors, never message text.

use ast_sgrep_lang::{match_literal_pattern, match_pattern, Language};
use ast_sgrep_testkit::match_lines as lines;

/// INTENT: general-lane template keys are isolated per pattern/keyword/
/// language and stable under reverse order, growth, interleave, and N-repeat.
/// KILLS: general-template-key-merge / cross-template-slot-poisoning /
/// keyword-index-key-merge / (language,class-query)-key-collapse /
/// repeat-use-result-drift.
/// ABSORBS: match_pattern_repeatable_across_interleaved_patterns,
/// class_query_keys_are_keyword_isolated, class_query_keys_are_language_isolated,
/// match_pattern_repeat_n_equals_once.
#[test]
fn general_template_keys_are_pattern_isolated() {
    // Leg 1 (anchor): return/throw keys isolated under reverse order + growth.
    let source = "function f() {\n  return 1;\n  return 2;\n}\n";
    let ret = match_pattern(Language::JavaScript, source, "return $X").unwrap();
    let thr = match_pattern(Language::JavaScript, source, "throw $X").unwrap();
    assert_eq!(lines(&ret), vec![2, 3]);
    assert!(thr.is_empty());
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

    // Leg 2 (interleaved patterns): repeat query identical after unrelated
    // literal + structural queries populate other template slots.
    let mixed = "fn Foo() {}\nfn foo() {}\nfn FOO() {}\n";
    let lower_first = match_pattern(Language::Rust, mixed, "foo").unwrap();
    let _ = match_pattern(Language::Rust, mixed, "Foo").unwrap();
    let _ = match_pattern(Language::Rust, mixed, "fn $NAME($$$)").unwrap();
    let _ = match_literal_pattern(Language::Rust, mixed, "FOO").unwrap();
    let lower_again = match_pattern(Language::Rust, mixed, "foo").unwrap();
    assert_eq!(lower_first, lower_again);
    assert!(!lower_first.is_empty());
    assert!(lower_first.iter().all(|h| h.line_start == 2));

    // Leg 3 (keyword isolation): struct/interface query keys never merge.
    let classes = "struct Widget {}\ntrait Greet {}\n";
    let strukt = match_pattern(Language::Rust, classes, "struct $NAME").unwrap();
    let iface = match_pattern(Language::Rust, classes, "interface $NAME").unwrap();
    assert_eq!(lines(&strukt), vec![1]);
    assert_eq!(lines(&iface), vec![2]);
    let iface2 = match_pattern(Language::Rust, classes, "interface $NAME").unwrap();
    let strukt2 = match_pattern(Language::Rust, classes, "struct $NAME").unwrap();
    assert_eq!(strukt, strukt2);
    assert_eq!(iface, iface2);
    let _ = match_pattern(Language::Rust, classes, "type $NAME").unwrap();
    assert_eq!(
        strukt,
        match_pattern(Language::Rust, classes, "struct $NAME").unwrap()
    );
    assert_eq!(
        iface,
        match_pattern(Language::Rust, classes, "interface $NAME").unwrap()
    );

    // Leg 4 (language isolation): empty (Rust, class-query) slot first must
    // not poison the Python key.
    let py_src = "class Widget:\n    pass\n";
    let rust_src = "struct Widget {}\n";
    let rust_first = match_pattern(Language::Rust, rust_src, "class $NAME").unwrap();
    let py_first = match_pattern(Language::Python, py_src, "class $NAME").unwrap();
    assert!(rust_first.is_empty());
    assert_eq!(lines(&py_first), vec![1]);
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

    // Leg 5 (repeat-N): 25 uses of general- and literal-lane keys equal 1 use.
    let first = match_pattern(Language::JavaScript, source, "return $X").unwrap();
    assert_eq!(lines(&first), vec![2, 3]);
    for _ in 0..25 {
        assert_eq!(
            match_pattern(Language::JavaScript, source, "return $X").unwrap(),
            first
        );
    }
    let rust_fns = "fn alpha() {}\nfn beta() {}\nfn gamma() {}\n";
    let lit_first = match_pattern(Language::Rust, rust_fns, "beta").unwrap();
    assert_eq!(lines(&lit_first), vec![2]);
    for _ in 0..25 {
        assert_eq!(
            match_pattern(Language::Rust, rust_fns, "beta").unwrap(),
            lit_first
        );
    }
}

/// INTENT: literal-lane keys are isolated per case and per language and
/// stable under reverse order, growth, poison order, and N-repeat.
/// KILLS: case-folding-key-collapse / (language,literal)-key-collapse /
/// (language,pattern)-key-collapse / literal-repeat-drift / empty-flip.
/// ABSORBS: literal_keys_are_language_isolated,
/// same_pattern_text_is_keyed_by_language, match_literal_repeat_n_equals_once.
#[test]
fn literal_keys_are_case_isolated() {
    // Leg 1 (anchor): foo/Foo/FOO keys isolated under reverse order + growth.
    let source = "fn Foo() {}\nfn foo() {}\nfn FOO() {}\n";
    let query =
        |pattern: &str| lines(&match_pattern(Language::Rust, source, pattern).unwrap());
    let foo_first = query("foo");
    let big_first = query("Foo");
    let caps_first = query("FOO");
    assert_eq!(foo_first, vec![2]);
    assert_eq!(big_first, vec![1]);
    assert_eq!(caps_first, vec![3]);
    let _ = query("FOO");
    let _ = query("Foo");
    let _ = query("foo");
    let _ = query("alpha");
    let _ = query("beta");
    let _ = query("gamma");
    assert_eq!(query("foo"), foo_first);
    assert_eq!(query("Foo"), big_first);
    assert_eq!(query("FOO"), caps_first);

    // Leg 2 (language isolation): (language, literal) keys isolated under
    // poison/reverse order + growth.
    let rust_src = "fn alpha() {}\n";
    let py_src = "def beta():\n    pass\n";
    let py_first = match_literal_pattern(Language::Python, py_src, "alpha").unwrap();
    let rust_first = match_literal_pattern(Language::Rust, rust_src, "alpha").unwrap();
    assert!(py_first.is_empty());
    assert_eq!(lines(&rust_first), vec![1]);
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

    // Leg 3 (same text, keyed by language): identical pattern text answers
    // per-language correctly under both population orders incl fresh thread.
    let rust_foo = "fn foo() {}\n";
    let py_bar = "def bar():\n    pass\n";
    let py_hits = match_pattern(Language::Python, py_bar, "foo").unwrap();
    let rust_hits = match_pattern(Language::Rust, rust_foo, "foo").unwrap();
    assert!(py_hits.is_empty());
    assert!(!rust_hits.is_empty());
    assert!(rust_hits.iter().all(|h| h.line_start == 1));
    assert_eq!(
        rust_hits,
        match_pattern(Language::Rust, rust_foo, "foo").unwrap()
    );
    let rust_fn_src = "fn alpha() {}\n";
    let py_def_src = "def beta():\n    pass\n";
    let rust_first_struct = match_pattern(Language::Rust, rust_fn_src, "def $NAME").unwrap();
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
    assert_eq!((rust_first_struct, py_after), reversed);

    // Leg 4 (repeat-N): 25 uses of hit and empty literal keys equal 1 use.
    let lit = "fn alpha() {}\n";
    let first = match_literal_pattern(Language::Rust, lit, "alpha").unwrap();
    assert_eq!(lines(&first), vec![1]);
    for _ in 0..25 {
        assert_eq!(
            match_literal_pattern(Language::Rust, lit, "alpha").unwrap(),
            first
        );
    }
    let empty_first =
        match_literal_pattern(Language::Python, "def beta():\n    pass\n", "alpha").unwrap();
    assert!(empty_first.is_empty());
    for _ in 0..25 {
        assert!(
            match_literal_pattern(Language::Python, "def beta():\n    pass\n", "alpha")
                .unwrap()
                .is_empty()
        );
    }
}

/// INTENT: if-cond template keys are isolated per cond (alpha/beta/meta)
/// under reverse order + growth.
/// KILLS: cond-key-merge.
/// ABSORBS: (standalone — no merges).
#[test]
fn if_cond_keys_are_cond_isolated() {
    let source = "if (alpha) { foo(); }\nif (beta) { bar(); }\n";
    let alpha = match_pattern(Language::JavaScript, source, "if (alpha) { $$$ }").unwrap();
    let beta = match_pattern(Language::JavaScript, source, "if (beta) { $$$ }").unwrap();
    let meta = match_pattern(Language::JavaScript, source, "if ($C) { $$$ }").unwrap();
    assert_eq!(lines(&alpha), vec![1]);
    assert_eq!(lines(&beta), vec![2]);
    assert_eq!(lines(&meta), vec![1, 2]);
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

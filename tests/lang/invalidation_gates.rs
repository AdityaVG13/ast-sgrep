//! Lang gate/memo contract suite: SUPPORTED gate, signatures, answerability.
//!
//! Contract under test: the process-wide `SUPPORTED` gate memo, the signature
//! memo, and the per-(language, pattern) answerability consults are
//! append-only with NO invalidation API — every cache is a pure function of
//! its key, pinned through the public API only. Discriminants are verdicts
//! and signature rows, never message text.

use ast_sgrep_lang::{
    cached_pattern_signatures, index_can_serve_pattern, native_pattern_answerable,
    needs_ast_grep_fallback, Language,
};

/// INTENT: supported/unsupported gate verdicts hold per key under both
/// population orders, repetition with interleave, 20-key growth, and 25-round
/// panel repetition.
/// KILLS: supported/unsupported-verdict-swap / gate-memo-verdict-flip /
/// growth-eviction-verdict-drift / gate-repeat-use-flip.
/// ABSORBS: fallback_gate_memo_is_stable_under_repetition,
/// supported_gate_stable_under_cache_growth,
/// fallback_gate_repeat_n_equals_once_panel.
#[test]
fn supported_gate_holds_per_pattern_verdicts() {
    // Leg 1 (anchor): per-key verdicts hold under both population orders.
    assert!(!needs_ast_grep_fallback("class $NAME"));
    assert!(needs_ast_grep_fallback("$A $B"));
    assert!(!needs_ast_grep_fallback("foo($$$)"));
    assert!(needs_ast_grep_fallback("$FOO $BAR"));
    assert!(needs_ast_grep_fallback("$A $B"));
    assert!(!needs_ast_grep_fallback("class $NAME"));
    assert!(needs_ast_grep_fallback("$FOO $BAR"));
    assert!(!needs_ast_grep_fallback("foo($$$)"));
    assert!(!needs_ast_grep_fallback("foo($$$)"));
    assert!(!needs_ast_grep_fallback("class $NAME"));
    assert!(needs_ast_grep_fallback("$A $B"));
    assert!(needs_ast_grep_fallback("$FOO $BAR"));

    // Leg 2 (repetition with interleave): verdicts stable under repetition.
    assert!(needs_ast_grep_fallback("if ($COND) { $A; $B }"));
    assert!(!needs_ast_grep_fallback("fn $NAME($$$)"));
    assert!(!needs_ast_grep_fallback("process_request"));
    assert!(needs_ast_grep_fallback("foo.$M+.bar($$$)"));
    assert!(needs_ast_grep_fallback("if ($COND) { $A; $B }"));
    assert!(!needs_ast_grep_fallback("fn $NAME($$$)"));
    assert!(!needs_ast_grep_fallback("process_request"));
    assert!(needs_ast_grep_fallback("foo.$M+.bar($$$)"));

    // Leg 3 (growth): 6-key verdict panel identical after 20-key growth.
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

    // Leg 4 (repeat-N): gate verdict panel reproduces exactly across 25 rounds.
    let repeat_panel = [
        "class $NAME",
        "$A $B",
        "foo($$$)",
        "process_request",
        "if ($COND) { $A; $B }",
    ];
    let first: Vec<bool> = repeat_panel
        .iter()
        .map(|p| needs_ast_grep_fallback(p))
        .collect();
    assert_eq!(first, vec![false, true, false, false, true]);
    for _ in 0..25 {
        let round: Vec<bool> = repeat_panel
            .iter()
            .map(|p| needs_ast_grep_fallback(p))
            .collect();
        assert_eq!(round, first);
    }
}

/// INTENT: signature rows + index verdicts are exact per pattern shape and
/// stable under repetition, growth, and 25-round panel repetition.
/// KILLS: signature-key-merge / growth-row-drift / signature-memo-row-drift /
/// signature/index-repeat-drift.
/// ABSORBS: signature_helpers_are_pure_across_repetition,
/// signature_helpers_repeat_n_equals_once.
#[test]
fn signature_keys_are_per_pattern_stable_under_growth() {
    // Leg 1 (anchor): exact rows + verdicts, stable under growth.
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

    // Leg 2 (purity across repetition): rows + verdicts pure with shape deltas.
    let ident_once = cached_pattern_signatures("SearchHit").unwrap();
    let decl_once = cached_pattern_signatures("fn greet_user").unwrap();
    let kind_once = cached_pattern_signatures("fn $NAME").unwrap();
    assert_eq!(ident_once, cached_pattern_signatures("SearchHit").unwrap());
    assert_eq!(decl_once, cached_pattern_signatures("fn greet_user").unwrap());
    assert_eq!(kind_once, cached_pattern_signatures("fn $NAME").unwrap());
    assert_ne!(ident_once, decl_once);
    assert_ne!(ident_once, kind_once);
    assert!(index_can_serve_pattern("SearchHit", &ident_once));
    assert!(!index_can_serve_pattern("fn $NAME", &kind_once));
    assert!(index_can_serve_pattern("SearchHit", &ident_once));
    assert!(!index_can_serve_pattern("fn $NAME", &kind_once));

    // Leg 3 (repeat-N): rows + verdicts identical across 25 rounds.
    let panel = ["Gadget", "fn beta", "bar($$$)", "fn $NAME"];
    let first_rows: Vec<Vec<String>> = panel
        .iter()
        .map(|p| cached_pattern_signatures(p).unwrap())
        .collect();
    let first_verdicts: Vec<bool> = panel
        .iter()
        .zip(first_rows.iter())
        .map(|(p, rows)| index_can_serve_pattern(p, rows))
        .collect();
    assert_eq!(first_verdicts, vec![true, true, true, false]);
    for _ in 0..25 {
        let rows: Vec<Vec<String>> = panel
            .iter()
            .map(|p| cached_pattern_signatures(p).unwrap())
            .collect();
        assert_eq!(rows, first_rows);
        let verdicts: Vec<bool> = panel
            .iter()
            .zip(rows.iter())
            .map(|(p, rows)| index_can_serve_pattern(p, rows))
            .collect();
        assert_eq!(verdicts, first_verdicts);
    }
}

/// INTENT: answerability verdicts are keyed per (language, pattern): covered
/// vs uncovered verdicts pinned under reverse order, growth, and interleaved
/// cross-language repetition.
/// KILLS: covered/uncovered-verdict-collapse / answerability-verdict-flip.
/// ABSORBS: native_answerability_stable_across_languages.
#[test]
fn native_answerability_is_per_language_keyed() {
    // Leg 1 (anchor): covered-JS vs uncovered-Swift pinned under reverse
    // order + growth.
    assert!(native_pattern_answerable(
        Language::JavaScript,
        "if (alpha) { $B }"
    ));
    assert!(!native_pattern_answerable(Language::Swift, "if (alpha) { $B }"));
    assert!(!native_pattern_answerable(Language::Swift, "if (alpha) { $B }"));
    assert!(native_pattern_answerable(
        Language::JavaScript,
        "if (alpha) { $B }"
    ));
    let _ = native_pattern_answerable(Language::Rust, "if (alpha) { $B }");
    let _ = native_pattern_answerable(Language::JavaScript, "if (beta) { $B }");
    let _ = native_pattern_answerable(Language::Python, "class $NAME");
    assert!(native_pattern_answerable(
        Language::JavaScript,
        "if (alpha) { $B }"
    ));
    assert!(!native_pattern_answerable(Language::Swift, "if (alpha) { $B }"));

    // Leg 2 (cross-language repetition): interleaved consults never flip.
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

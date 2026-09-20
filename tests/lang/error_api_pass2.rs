//! E2 error-propagation oracles for `ast-sgrep-lang`.
//!
//! Where E1 inventories each failure site in isolation, E2 pins the LINKS:
//! a failure at stage N must surface as the documented discriminant at
//! stage N+1 — never a silently-wrong answer, never a panic, never an `Err`
//! on user-influenced input.
//!
//! Pipeline pinned here (lang-public surface only; score/rank live
//! downstream and consume these same discriminants):
//!
//! ```text
//! detect/parse (Option<Language>) -> classify (Option<NativeKind>)
//!   -> signatures (Option<Vec<String>>) -> gates (bool)
//!   -> match (Ok(hits)) / extract (Ok + depth_truncated flag)
//! ```
//!
//! Documented propagation contract:
//! - `None` / `false` at an early stage routes the late stage to its
//!   documented default (`Ok(vec![])`, walk fallback, full scan) — never
//!   to a fabricated answer.
//! - `match_pattern` / `match_literal_pattern` / `ParserRegistry::parse`
//!   are `Ok` on ALL user-influenced input (their `Err` sides are
//!   unreachable: no-parser-registered, set-language, tree-sitter parse).
//! - `depth_truncated` is the loud flag for the extraction cap: extraction
//!   stays `Ok` and downstream must treat that file's rows as incomplete.
//!
//! Panic-guard inventory (grep `\.unwrap\(\)\|\.expect\(` over
//! `crates/ast-sgrep-lang/src`, user-influenced paths only):
//! - `pattern/answerable.rs:195,347` `.expect("single node has a child")`:
//!   guarded by `while is_sg_single_node(node)` (child_count 1, or 2 with
//!   a missing/empty second child) — `child(0)` always exists. Pinned by
//!   `match_never_errs_or_panics_on_hostile_input`.
//! - `pattern/calls.rs:770` `.end.unwrap()`: guarded by
//!   `path[..=absorb].iter().all(|seg| seg.end.is_some())` over the same
//!   slice. Pinned by `chain_span_guards_hold`.
//! - Every other production call site is `unwrap_or*` (total). The only
//!   bare `.unwrap()`s are inside `#[cfg(test)]` unit tests.
//!
//! Assertions are discriminants only (`is_none` / `is_ok` / emptiness /
//! bools) — never message text.

use ast_sgrep_lang::{
    cached_pattern_signatures, candidate_kind_signatures, classify_native, detect_language,
    index_can_serve_pattern, match_literal_pattern, match_pattern, native_pattern_answerable,
    needs_ast_grep_fallback, required_pattern_literal, Language, ParserRegistry,
};
use std::path::Path;

/// E2-LANG-01: detect -> parse handoff. Every indexed extension detects to a
/// language whose registry parser accepts hostile input with Ok — a detected
/// language can never strand the parse stage, and an undetectable path
/// yields None (no silent default language).
/// INTENT: every indexed ext detects + parses hostile Ok; undetectable stays None.
/// KILLS: detect-parse-handoff-break, silent-default.
/// OVERLAP: E1 registry rows (adds the handoff link).
/// ABSORBS: none (propagation pin; nothing merged).
#[test]
fn detect_some_always_parses_ok() {
    let registry = ParserRegistry::new();
    for (ext, lang) in Language::SOURCE_EXTENSIONS {
        let rel = format!("n.{ext}");
        assert_eq!(detect_language(Path::new(&rel), None), Some(*lang));
        assert!(
            registry.parse(*lang, "{{{{\n\x00\x01garbage").is_ok(),
            "{lang}"
        );
    }
    // Undetectable input propagates as None through detection, never a guess.
    assert!(detect_language(Path::new("n.fortran"), None).is_none());
    assert!(detect_language(Path::new("n.fortran"), Some("print(1)")).is_none());
}

/// E2-LANG-02: classify -> signatures handoff. Classifier rejection
/// propagates to index-stage None, so a rejected shape is never served
/// from the index; classifiable shapes keep their signatures.
/// INTENT: classifier None → both signature stages None; classified keeps signatures.
/// KILLS: serve-rejected-shape-from-index.
/// ABSORBS: none (propagation pin; nothing merged).
#[test]
fn classifier_rejection_propagates_to_index_none() {
    for pattern in ["$A $B", "greet($Ü)", "def a(x): $$$B"] {
        assert!(classify_native(pattern).is_none(), "{pattern}");
        assert!(cached_pattern_signatures(pattern).is_none(), "{pattern}");
        assert!(candidate_kind_signatures(pattern).is_none(), "{pattern}");
    }
    // Positive control: classified shapes carry signatures downstream.
    assert!(classify_native("foo($$$)").is_some());
    assert!(cached_pattern_signatures("foo($$$)").is_some());
    assert!(candidate_kind_signatures("fn $N($$$)").is_some());
}

/// E2-LANG-03: signatures -> gate -> match handoff. Unindexable shapes
/// (braced bodies, member chains) propagate as gate denial, and the match
/// stage still answers Ok via the walk fallback — denial never becomes a
/// silent wrong answer or an Err.
/// INTENT: unindexable → gate deny → match Ok via walk; indexable control served.
/// KILLS: deny-to-Err, deny-to-silent-empty.
/// ABSORBS: none (propagation pin; nothing merged).
#[test]
fn unindexable_propagates_to_gate_denial_then_ok_match() {
    for pattern in ["fn $N($$$) { $B }", "fetch()?.$M($$$A)", "a.b($$$C).d()"] {
        assert!(cached_pattern_signatures(pattern).is_none(), "{pattern}");
        assert!(!index_can_serve_pattern(pattern, &[]), "{pattern}");
        assert!(
            match_pattern(Language::Rust, "fn foo() { foo(1); }", pattern).is_ok(),
            "{pattern}"
        );
    }
    // Positive control: an indexable shape is serveable AND matchable.
    let sigs = cached_pattern_signatures("foo($$$)").unwrap();
    assert!(index_can_serve_pattern("foo($$$)", &sigs));
    assert!(match_pattern(Language::Rust, "fn foo() { foo(1); }", "foo($$$)").is_ok());
}

/// E2-LANG-04: prefilter None means "scan", never "skip". A missing SIMD
/// literal must not empty the match stage: metavariable-callee patterns
/// still answer through the walk, while the all-comment shape stays
/// fail-closed (Ok + empty, not fail-open garbage).
/// INTENT: no-literal patterns still scan via walk; all-comment stays Ok-empty.
/// KILLS: None-means-skip, comment-fail-open.
/// ABSORBS: none (propagation pin; nothing merged).
#[test]
fn prefilter_none_still_scans() {
    assert!(required_pattern_literal("$F($$$)").is_none());
    let hits = match_pattern(Language::Rust, "fn f() { foo(1); bar(2); }", "$F($$$)").unwrap();
    assert!(!hits.is_empty());
    // All-comment pattern: no literal, no hits, still Ok (fail-closed).
    assert!(required_pattern_literal("// only a comment").is_none());
    let hits = match_pattern(Language::Rust, "fn f() { foo(1); }", "// only a comment").unwrap();
    assert!(hits.is_empty());
    // Positive control: a concrete literal both prefilters and matches.
    assert_eq!(required_pattern_literal("foo($$$)").as_deref(), Some("foo"));
    assert!(
        !match_pattern(Language::Rust, "fn f() { foo(1); }", "foo($$$)")
            .unwrap()
            .is_empty()
    );
}

/// E2-LANG-05: match is total on user input. Hostile pattern x hostile
/// source across every language: always Ok, never Err, never panic. This
/// pins the `answerable.rs` single-node `.expect` guards end to end.
/// INTENT: hostile pattern × hostile source × every lang always Ok both entries.
/// KILLS: Err-on-user-input, single-node-expect-panic (answerable.rs guard pin).
/// ABSORBS: none (propagation pin; nothing merged).
#[test]
fn match_never_errs_or_panics_on_hostile_input() {
    let patterns = [
        ";;",
        "->",
        "::",
        ")))(((",
        "\x00\x01",
        "$Ü",
        "µµµ$A",
        "$A$$B",
        "a?.b($X)",
        "$O.out\n.$M($A)",
        "if ($COND) { $A; $B }",
        "fn $N($$$) { $B }",
        "namespace A { f(); $B }",
        "C::$s = 5",
        "del $X;",
        "return($X)",
        "\u{feff}foo",
    ];
    let sources = [
        "",
        "{{{{ !!",
        "\x00\x01\x02",
        "fn f() { let x = (((1; }",
        ";;;\n;;",
    ];
    for lang in Language::all() {
        for pattern in patterns {
            for source in sources {
                assert!(
                    match_pattern(*lang, source, pattern).is_ok(),
                    "{lang} {pattern:?}"
                );
                assert!(
                    match_literal_pattern(*lang, source, pattern).is_ok(),
                    "{lang} {pattern:?}"
                );
            }
        }
    }
}

/// E2-LANG-06: parse -> extract -> match agreement on garbage. Garbage
/// source propagates as Ok + empty rows (documented defaults), and the
/// match stage agrees (Ok) — no stage invents hits or errors.
/// INTENT: garbage → Ok + empty rows, flag clear, match Ok, every lang.
/// KILLS: invented-hits-on-garbage, garbage-Err.
/// OVERLAP: E1-LANG-04 (adds all-lang + match agreement).
/// ABSORBS: none (propagation pin; nothing merged).
#[test]
fn garbage_propagates_ok_empty_through_extract_and_match() {
    let registry = ParserRegistry::new();
    for lang in Language::all() {
        let extraction = registry.parse(*lang, "{{{{ !! not code").unwrap();
        assert!(extraction.symbols.is_empty(), "{lang}");
        assert!(!extraction.depth_truncated, "{lang}");
        assert!(
            match_pattern(*lang, "{{{{ !! not code", "foo").is_ok(),
            "{lang}"
        );
        assert!(
            match_literal_pattern(*lang, "{{{{ !! not code", "foo").is_ok(),
            "{lang}"
        );
    }
}

/// E2-LANG-07: the depth cap propagates LOUD. A breached cap keeps
/// extraction Ok but sets `depth_truncated` (downstream must treat rows as
/// incomplete); the match stage is independent of the cap and stays Ok.
/// INTENT: breach keeps Ok + sets depth_truncated; match independent Ok.
/// KILLS: breach-as-Err, silent-breach.
/// ABSORBS: none (propagation pin; nothing merged).
#[test]
fn depth_breach_propagates_loud_flag_not_err() {
    let registry = ParserRegistry::new();
    let deep_src = format!(
        "fn f() {{ let x = {}1{}; }}",
        "(".repeat(300),
        ")".repeat(300)
    );
    let extraction = registry.parse(Language::Rust, &deep_src).unwrap();
    assert!(extraction.depth_truncated);
    assert!(match_pattern(Language::Rust, &deep_src, "f").is_ok());
    // Shallow control: same stages, flag clear.
    let shallow = registry
        .parse(Language::Rust, "fn f() { let x = (1 + (2 * 3)); }")
        .unwrap();
    assert!(!shallow.depth_truncated);
}

/// E2-LANG-08: gate denial forces the walk, not an empty. Statement-keyword
/// shapes are un-serveable from the index (keyword tokens are never rows)
/// yet still answered by the native walk — denial must not propagate as
/// silent empty.
/// INTENT: statement-keyword shapes unserveable yet walk-answered (break/return).
/// KILLS: deny-to-silent-empty.
/// ABSORBS: none (propagation pin; nothing merged).
#[test]
fn gate_denial_still_answers_via_walk() {
    assert!(!index_can_serve_pattern("break", &["break".to_string()]));
    let hits = match_pattern(Language::JavaScript, "for(;;){break;}", "break").unwrap();
    assert!(!hits.is_empty());
    assert!(!index_can_serve_pattern("return", &["return".to_string()]));
    let hits = match_pattern(Language::JavaScript, "function f(){return 1;}", "return").unwrap();
    assert!(!hits.is_empty());
}

/// E2-LANG-09: fallback-loud shapes stay match-closed natively. A pattern
/// the search ingress refuses (`needs_ast_grep_fallback`) must not be
/// answered with fabricated hits by the native lane: Ok + empty is the
/// documented default, and the degenerates stay unanswerable per-language.
/// INTENT: fallback-loud patterns natively Ok-empty; degenerates unanswerable + control.
/// KILLS: fabricated-hits-on-loud-shape.
/// ABSORBS: none (propagation pin; nothing merged).
#[test]
fn fallback_loud_shapes_match_closed() {
    assert!(needs_ast_grep_fallback("greet($Ü)"));
    assert!(
        match_pattern(Language::Rust, "fn f() { greet(x); }", "greet($Ü)")
            .unwrap()
            .is_empty()
    );
    assert!(!native_pattern_answerable(Language::Rust, ";;"));
    assert!(match_pattern(Language::Rust, "fn f() {}", ";;")
        .unwrap()
        .is_empty());
    // Positive control: the same source answers a native shape.
    assert!(
        !match_pattern(Language::Rust, "fn f() { greet(x); }", "greet($$$)")
            .unwrap()
            .is_empty()
    );
}

/// E2-LANG-10: chain-span guards hold. Optional/member chains across the
/// connector spellings exercise the `calls.rs` span-slice guard
/// (`all(end.is_some())` before `.end.unwrap()`): always Ok, never panic,
/// with bound hits where the shape genuinely matches.
/// INTENT: optional/member chains across connectors always Ok, genuine hit binds.
/// KILLS: calls.rs-end-unwrap-panic (span-guard pin).
/// ABSORBS: none (propagation pin; nothing merged).
#[test]
fn chain_span_guards_hold() {
    let cases = [
        (Language::TypeScript, "a?.b(1); a.b(2);", "a?.b($X)"),
        (Language::JavaScript, "o.out.m(1);", "$O.out.$M($A)"),
        (Language::Rust, "fn f() { a.b(c); }", "a.b($X)"),
        (Language::Php, "<?php $a->b(1);", "$O->b($A)"),
        (Language::Python, "a.b(1)\n", "a.b($X)"),
    ];
    for (lang, source, pattern) in cases {
        assert!(
            match_pattern(lang, source, pattern).is_ok(),
            "{lang} {pattern}"
        );
    }
    // Positive control: a plain chain hit binds.
    assert!(
        !match_pattern(Language::Rust, "fn f() { a.b(c); }", "a.b($X)")
            .unwrap()
            .is_empty()
    );
}

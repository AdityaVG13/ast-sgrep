//! Error-API matrix for `ast-sgrep-lang`.
//!
//! F35 collapse of `error_api_pass{1,2,3,4}.rs` into one parametric target.
//! Pure collapse: every test below is unchanged from its pass-file original —
//! same triggers, same assertions, same INTENT/KILLS/ABSORBS audit lines. Only
//! the file layout changed (one import block, E1-E4 sections). Per-pass module
//! docs are kept as section banners; shared setup lives in
//! `error_testkit.rs` (canonical beats promoted to `ast-sgrep-testkit`).
//!
//! Matrix (mission -> test):
//! - E1 taxonomy: `lang_id_rejections`, `detect_language_unsupported_paths`, `registry_parses_every_language_ok`, `extraction_empty_and_garbage_fail_closed`, `depth_guard_threshold`, `classify_native_rejections`, `cached_and_candidate_signature_gaps`, `required_pattern_literal_absent`, `serve_and_answer_gates`, `fallback_loud_class`, `match_ok_empty_rejections`, `declaration_prefix_non_decl_none`
//! - E2 propagation: `detect_some_always_parses_ok`, `classifier_rejection_propagates_to_index_none`, `unindexable_propagates_to_gate_denial_then_ok_match`, `prefilter_none_still_scans`, `match_never_errs_or_panics_on_hostile_input`, `garbage_propagates_ok_empty_through_extract_and_match`, `depth_breach_propagates_loud_flag_not_err`, `gate_denial_still_answers_via_walk`, `fallback_loud_shapes_match_closed`, `chain_span_guards_hold`
//! - E3 relations: `match_entries_agree_and_repeat_deterministically`, `language_id_entries_agree`, `classifier_acceptance_implies_downstream_presence`, `gate_denial_is_total_and_order_invariant`, `prefilter_literal_is_sound_substring`, `semicolon_count_boundary_consistent_across_languages`, `depth_cap_monotone_with_loud_flip`, `padding_invariant_entries_vs_sensitive_ident`
//! - E4 drills: `empty_and_garbage_source_end_to_end_rank_empty`, `unicode_bom_nul_source_end_to_end_sound_and_stable`, `unsupported_language_short_circuits_with_no_ranked_output`, `hostile_pattern_on_real_source_stays_sound`, `fallback_loud_pattern_end_to_end_match_closed`, `depth_breach_end_to_end_loud_flag_with_sound_rank`, `mixed_hostile_corpus_drill`
//!
//! No asserted behavior changed in this collapse (see F35: keep every distinct
//! behavior assertion, drop pure duplication — no pure duplication remained;
//! net test count is unchanged).
use ast_sgrep_lang::{
    cached_pattern_signatures, candidate_kind_signatures, classify_native, declaration_prefix,
    detect_language, index_can_serve_pattern, is_pattern_ident, is_universal_root_pattern,
    literal_trailing_comment_lane, match_literal_pattern, match_pattern, native_pattern_answerable,
    needs_ast_grep_fallback, pattern_is_keyword_literal_root, php_comment_transparent_operand_lane,
    required_pattern_literal, tree_sitter_language, Language, ParserRegistry, PatternMatch,
};
use ast_sgrep_testkit::{assert_rank_sound, assert_spans_in_source, run_pipeline};
use std::path::Path;

// ── E1 taxonomy inventory (from error_api_pass1.rs) ──
// E1 error-taxonomy inventory for `ast-sgrep-lang`.
//
// One test per failure site on the lang public surface, each with a
// hand-built trigger. Assertions are discriminants only (`is_none` /
// `is_some` / `is_ok` / emptiness / bools) — never message text.
//
// Taxonomy rows (E1-LANG-01..12); oracle-foundry gaps covered here are the
// depth guard, signature-indexability `None`s, prefilter-literal `None`s,
// `declaration_prefix` `None`, and the unsupported-language `None` paths.
//

/// E1-LANG-01: language-id parsing rejections.
/// INTENT: unknown/blank lang ids rejected (None) across parse/from_extension/canonical + controls.
/// KILLS: silent-default-language, blank-accepted.
/// ABSORBS: none (taxonomy anchor; nothing merged).
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

/// E1-LANG-02: unsupported-language detection paths.
/// INTENT: unknown ext / no content / no shebang → None + controls.
/// KILLS: detect-guess-on-unknown.
/// ABSORBS: none (taxonomy anchor; nothing merged).
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

/// E1-LANG-03: the registry serves every language; its `Err` side
/// ("no parser registered", "failed to set language", "failed to parse
/// source") is unreachable via the public API, so pin `Ok` on hostile input.
/// INTENT: registry parse Ok on empty + hostile input for every language (Err side unreachable).
/// KILLS: introduced-Err-on-user-input, per-language-panic.
/// ABSORBS: none (taxonomy anchor; nothing merged).
#[test]
fn registry_parses_every_language_ok() {
    let registry = ParserRegistry::new();
    for lang in Language::all() {
        assert!(registry.parse(*lang, "").is_ok(), "{lang}");
        assert!(
            registry.parse(*lang, "{{{{\n\x00\x01garbage").is_ok(),
            "{lang}"
        );
    }
}

/// E1-LANG-04: extraction never errors on empty/garbage source; it fails
/// closed with empty rows.
/// INTENT: empty/garbage source → Ok + empty rows, flag clear + control.
/// KILLS: invented-rows-on-garbage, Err-on-garbage.
/// ABSORBS: none (taxonomy anchor; nothing merged).
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

/// E1-LANG-05: the MAX_EXTRACTION_DEPTH (256) threshold guard.
/// INTENT: shallow clear / 300-deep paren nesting sets depth_truncated (256 cap).
/// KILLS: cap-drop(fail-open-deep), cap-too-tight.
/// ABSORBS: none (taxonomy anchor; nothing merged).
#[test]
fn depth_guard_threshold() {
    let registry = ParserRegistry::new();
    let shallow_src = "fn f() { let x = (1 + (2 * 3)); }";
    assert!(
        !registry
            .parse(Language::Rust, shallow_src)
            .unwrap()
            .depth_truncated
    );
    // 300-deep paren nesting breaches the 256 cap.
    let deep_src = format!(
        "fn f() {{ let x = {}1{}; }}",
        "(".repeat(300),
        ")".repeat(300)
    );
    assert!(
        registry
            .parse(Language::Rust, &deep_src)
            .unwrap()
            .depth_truncated
    );
}

/// E1-LANG-06: classifier rejections (exotic shapes -> None).
/// INTENT: exotic shapes classify None + call-shape control.
/// KILLS: misclassified-exotic.
/// ABSORBS: none (taxonomy anchor; nothing merged).
#[test]
fn classify_native_rejections() {
    // Fallback-loud in the oracle suite implies classifier rejection.
    assert!(classify_native("greet($Ü)").is_none());
    assert!(classify_native("def a(x): $$$B").is_none());
    assert!(classify_native("$A $B").is_none());
    // Positive control: a call shape classifies.
    assert!(classify_native("foo($$$)").is_some());
}

/// E1-LANG-07: index-signature indexability gaps.
/// INTENT: braced/chain/malformed unindexable None; empty→Some([]); + controls.
/// KILLS: index-serve-unindexable, empty-rejected.
/// ABSORBS: none (taxonomy anchor; nothing merged).
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
    assert_eq!(
        cached_pattern_signatures("foo($$$)").unwrap(),
        vec!["call:foo".to_string()]
    );
    assert_eq!(
        cached_pattern_signatures("$F($$$)").unwrap(),
        vec!["kind:call_expression".to_string(), "kind:call".to_string()]
    );
    // Candidate kinds only serve declaration prefixes.
    assert!(candidate_kind_signatures("").is_none());
    assert!(candidate_kind_signatures("foo($$$)").is_none());
    assert!(candidate_kind_signatures("fn $N($$$)").is_some());
}

/// E1-LANG-08: SIMD prefilter literal absence.
/// INTENT: meta-only/keyword/comment/empty patterns yield no prefilter literal + controls.
/// KILLS: wrong-literal-filter(drops-files), keyword-as-literal.
/// ABSORBS: none (taxonomy anchor; nothing merged).
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

/// E1-LANG-09: serve/answer gate denials.
/// INTENT: serve/answer/keyword/ident/universal-root gate denials + controls.
/// KILLS: gate-serve-unserveable, keyword-ident-serve.
/// ABSORBS: none (taxonomy anchor; nothing merged).
#[test]
fn serve_and_answer_gates() {
    // index_can_serve_pattern denials.
    assert!(!index_can_serve_pattern("foo", &[]));
    assert!(!index_can_serve_pattern(
        "fn $N",
        &["kind:function_item".to_string()]
    ));
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

/// E1-LANG-10: the loud fail-closed class (needs external ast-grep).
/// INTENT: fallback-loud shapes need ast-grep; native/empty shapes do not.
/// KILLS: fallback-misroute.
/// ABSORBS: none (taxonomy anchor; nothing merged).
#[test]
fn fallback_loud_class() {
    assert!(needs_ast_grep_fallback("if ($COND) { $A; $B }"));
    assert!(needs_ast_grep_fallback("greet($Ü)"));
    assert!(!needs_ast_grep_fallback(""));
    assert!(!needs_ast_grep_fallback("process_request"));
    assert!(!needs_ast_grep_fallback("fn $NAME($$$)"));
}

/// E1-LANG-11: match entry points return Ok(empty) on rejected shapes,
/// never Err, even on garbage source.
/// INTENT: rejected shapes match Ok(empty) incl. garbage source, never Err + control.
/// KILLS: Err-on-rejected-shape, garbage-Err.
/// ABSORBS: none (taxonomy anchor; nothing merged).
#[test]
fn match_ok_empty_rejections() {
    // Empty pattern.
    assert!(match_pattern(Language::Rust, "fn foo() {}", "")
        .unwrap()
        .is_empty());
    assert!(match_literal_pattern(Language::Rust, "fn foo() {}", "")
        .unwrap()
        .is_empty());
    // Bare-connector garbage class.
    assert!(match_pattern(Language::Rust, "fn foo() {}", "->")
        .unwrap()
        .is_empty());
    assert!(match_pattern(Language::Rust, "fn foo() {}", "::")
        .unwrap()
        .is_empty());
    // PHP-only garbage spellings.
    assert!(match_pattern(Language::Php, "<?php $a && $b;", "&&")
        .unwrap()
        .is_empty());
    assert!(match_pattern(Language::Php, "<?php $a && $b;", ".")
        .unwrap()
        .is_empty());
    // Garbage source is Ok (per-file robustness), never Err.
    assert!(match_pattern(Language::Rust, "{{{{ !!", "foo").is_ok());
    // Positive control: a literal hit is found.
    assert!(!match_pattern(Language::Rust, "fn foo() {}", "foo")
        .unwrap()
        .is_empty());
}

/// E1-LANG-12: declaration_prefix is None for non-declaration nodes.
/// INTENT: declaration_prefix Some("fn") on function_item, None on name node.
/// KILLS: prefix-on-non-decl, prefix-drop.
/// ABSORBS: none (only node-level pin; nothing merged).
#[test]
fn declaration_prefix_non_decl_none() {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_language(Language::Rust))
        .unwrap();
    let tree = parser.parse("fn foo() { bar(); }", None).unwrap();
    let root = tree.root_node();
    // The function_item node has a prefix; the identifier / call nodes do not.
    let func = root.named_child(0).unwrap();
    assert_eq!(func.kind(), "function_item");
    assert_eq!(declaration_prefix(&func, "fn foo() { bar(); }"), Some("fn"));
    let name = func.child_by_field_name("name").unwrap();
    assert!(declaration_prefix(&name, "fn foo() { bar(); }").is_none());
}

// ── E2 propagation oracles (from error_api_pass2.rs) ──
// E2 error-propagation oracles for `ast-sgrep-lang`.
//
// Where E1 inventories each failure site in isolation, E2 pins the LINKS:
// a failure at stage N must surface as the documented discriminant at
// stage N+1 — never a silently-wrong answer, never a panic, never an `Err`
// on user-influenced input.
//
// Pipeline pinned here (lang-public surface only; score/rank live
// downstream and consume these same discriminants):
//
// ```text
// detect/parse (Option<Language>) -> classify (Option<NativeKind>)
//   -> signatures (Option<Vec<String>>) -> gates (bool)
//   -> match (Ok(hits)) / extract (Ok + depth_truncated flag)
// ```
//
// Documented propagation contract:
// - `None` / `false` at an early stage routes the late stage to its
//   documented default (`Ok(vec![])`, walk fallback, full scan) — never
//   to a fabricated answer.
// - `match_pattern` / `match_literal_pattern` / `ParserRegistry::parse`
//   are `Ok` on ALL user-influenced input (their `Err` sides are
//   unreachable: no-parser-registered, set-language, tree-sitter parse).
// - `depth_truncated` is the loud flag for the extraction cap: extraction
//   stays `Ok` and downstream must treat that file's rows as incomplete.
//
// Panic-guard inventory (grep `\.unwrap\(\)\|\.expect\(` over
// `crates/ast-sgrep-lang/src`, user-influenced paths only):
// - `pattern/answerable.rs:195,347` `.expect("single node has a child")`:
//   guarded by `while is_sg_single_node(node)` (child_count 1, or 2 with
//   a missing/empty second child) — `child(0)` always exists. Pinned by
//   `match_never_errs_or_panics_on_hostile_input`.
// - `pattern/calls.rs:770` `.end.unwrap()`: guarded by
//   `path[..=absorb].iter().all(|seg| seg.end.is_some())` over the same
//   slice. Pinned by `chain_span_guards_hold`.
// - Every other production call site is `unwrap_or*` (total). The only
//   bare `.unwrap()`s are inside `#[cfg(test)]` unit tests.
//
// Assertions are discriminants only (`is_none` / `is_ok` / emptiness /
// bools) — never message text.
//

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

// ── E3 metamorphic relations (from error_api_pass3.rs) ──
// E3 negative-path metamorphic tests for `ast-sgrep-lang`.
//
// Where E1 inventories each failure site and E2 pins stage-to-stage
// propagation, E3 asserts RELATIONS that must hold across entry points,
// repetitions, and threshold boundaries:
//
// - Cross-entry consistency: the same invalid input through different
//   entry points yields a consistent failure discriminant — never `Ok` in
//   one and `Err` in another for the same contract, never `Some` in one
//   index view and `None` in another where the contract links them.
// - Repetition determinism: every pure query repeated returns the equal
//   discriminant (no hash-order, no global-state drift).
// - Degenerate-input totality: empty / whitespace / NUL / unicode / BOM
//   inputs always produce the documented outcome — never a panic, never a
//   silently-wrong value.
// - Threshold-boundary consistency: just-below vs just-above the
//   documented caps behave per contract. The lang crate has no rankers;
//   its thresholds are `MAX_EXTRACTION_DEPTH` (256) and the degenerate
//   `;`-root count (1 vs 2+), and the "across all rankers" leg is covered
//   across all languages and all consuming entry points instead.
//
// Assertions are discriminants only (`is_some` / `is_ok` / emptiness /
// bools / equality of data values) — never message text.
//

// Area-local degenerate corpus: swept by the E3-01 match/extraction legs and
// the E3-09 pure-entry legs (the split halves of the former
// `degenerate_inputs_total_and_stable` no-panic sweep). Kept here, not
// promoted: only these two tests sweep it.
const DEGENERATE_PATTERNS: &[&str] = &[
    "", " ", "\t\n", "\x00", "µ", "$", "$$$", "$Ü", "💥", "\u{feff}", "(", ")", "{", "}", ";",
    ";;", "->", "&&", ".", "$A$$B",
];

/// E3-LANG-01: match entry-point agreement + repetition determinism.
/// Same hostile (lang, source, pattern) through both match entries: the
/// `is_ok` discriminant always agrees (never Ok in one and Err in the
/// other), and each entry repeated returns the identical hit vector.
/// Corpus is disjoint from E2's hostile set.
/// INTENT: both match entries agree is_ok + identical hit vectors on repeat (disjoint corpus).
/// KILLS: entry-divergence(Ok-vs-Err), nondeterministic-hits.
/// OVERLAP: E2 hostile set (corpus disjoint by design).
/// ABSORBS: degenerate_inputs_total_and_stable match/extraction repetition legs (kept as the degenerate no-panic sweep leg).
#[test]
fn match_entries_agree_and_repeat_deterministically() {
    let patterns = [
        "", "$", "$$$", "$$A", "µ", "💥", "\x00", "(", "{", "a.", ".b", "$A;",
    ];
    let sources = ["", " ", "\x00", "(((", "💥", "fn f(){}"];
    for lang in Language::all() {
        for pattern in patterns {
            for source in sources {
                let via_struct = match_pattern(*lang, source, pattern);
                let via_literal = match_literal_pattern(*lang, source, pattern);
                assert_eq!(
                    via_struct.is_ok(),
                    via_literal.is_ok(),
                    "{lang} {pattern:?} {source:?}"
                );
                // Repetition determinism per entry (full hit-vector equality).
                let again_struct = match_pattern(*lang, source, pattern).unwrap();
                assert_eq!(via_struct.unwrap(), again_struct, "{lang} {pattern:?}");
                let again_literal = match_literal_pattern(*lang, source, pattern).unwrap();
                assert_eq!(via_literal.unwrap(), again_literal, "{lang} {pattern:?}");
            }
        }
    }
    // The empty-pattern degenerate agrees as Ok + empty in BOTH entries.
    for lang in Language::all() {
        assert!(match_pattern(*lang, "fn f() {}", "").unwrap().is_empty());
        assert!(match_literal_pattern(*lang, "fn f() {}", "")
            .unwrap()
            .is_empty());
    }
    // Absorbed degenerate leg: no-panic sweep — both match entries stay Ok
    // with identical results across repetitions, and extraction treats the
    // degenerates as source with Ok + identical results.
    let registry = ParserRegistry::new();
    for pattern in DEGENERATE_PATTERNS {
        let a = match_pattern(Language::Rust, "fn f() { foo(1); }", pattern).unwrap();
        let b = match_pattern(Language::Rust, "fn f() { foo(1); }", pattern).unwrap();
        assert_eq!(a, b, "{pattern:?}");
        let c = match_literal_pattern(Language::Rust, "fn f() { foo(1); }", pattern).unwrap();
        let d = match_literal_pattern(Language::Rust, "fn f() { foo(1); }", pattern).unwrap();
        assert_eq!(c, d, "{pattern:?}");
        let e1 = registry.parse(Language::Rust, pattern).unwrap();
        let e2 = registry.parse(Language::Rust, pattern).unwrap();
        assert_eq!(e1, e2, "{pattern:?}");
    }
}

/// E3-LANG-02: language-id cross-entry consistency.
/// Every indexed extension agrees across parse / from_extension /
/// canonical_filter / normalize_id / detect_language, invariant under case
/// and padding; unknown labels agree as rejections with the documented
/// lowercase-normalize relation (never a silent default language).
/// INTENT: parse/ext/filter/normalize/detect agree per ext, case/padding invariant, unknowns lowercase.
/// KILLS: entry-divergence, case-sensitivity-regression.
/// ABSORBS: none (relation pin; nothing merged).
#[test]
fn language_id_entries_agree() {
    for (ext, lang) in Language::SOURCE_EXTENSIONS {
        assert_eq!(Language::parse(ext), Some(*lang), "{ext}");
        assert_eq!(Language::from_extension(ext), Some(*lang), "{ext}");
        assert_eq!(Language::parse(ext), Language::from_extension(ext), "{ext}");
        // Case + padding invariance across entries.
        assert_eq!(
            Language::from_extension(&ext.to_ascii_uppercase()),
            Some(*lang),
            "{ext}"
        );
        assert_eq!(Language::parse(&format!("  {ext}  ")), Some(*lang), "{ext}");
        assert_eq!(
            Language::canonical_filter(Some(ext)).as_deref(),
            Some(lang.as_str()),
            "{ext}"
        );
        assert_eq!(Language::normalize_id(ext), lang.as_str(), "{ext}");
        assert_eq!(
            Language::normalize_id(&ext.to_ascii_uppercase()),
            lang.as_str(),
            "{ext}"
        );
        // Detection agrees with the extension table, case-insensitively.
        let rel = format!("n.{ext}");
        assert_eq!(detect_language(Path::new(&rel), None), Some(*lang), "{ext}");
        let upper = format!("n.{}", ext.to_ascii_uppercase());
        assert_eq!(
            detect_language(Path::new(&upper), None),
            Some(*lang),
            "{ext}"
        );
        // Repetition determinism.
        assert_eq!(
            detect_language(Path::new(&rel), None),
            detect_language(Path::new(&rel), None)
        );
    }
    // Unknown labels: rejections agree; canonical/normalize lowercase.
    for unknown in ["fortran", "cobol", "not-a-lang", "rs!", "FORTRAN", " r s "] {
        assert!(Language::from_extension(unknown).is_none(), "{unknown:?}");
        assert!(Language::parse(unknown).is_none(), "{unknown:?}");
        let lowered = unknown.trim().to_ascii_lowercase();
        assert_eq!(
            Language::canonical_filter(Some(unknown)).as_deref(),
            Some(lowered.as_str()),
            "{unknown:?}"
        );
        assert_eq!(Language::normalize_id(unknown), lowered, "{unknown:?}");
        let rel = format!("n.{unknown}");
        assert!(
            detect_language(Path::new(&rel), None).is_none(),
            "{unknown:?}"
        );
    }
    // Blank input is the no-filter degenerate in every entry.
    for blank in ["", "   "] {
        assert!(Language::from_extension(blank).is_none());
        assert!(Language::parse(blank).is_none());
        assert!(Language::canonical_filter(Some(blank)).is_none());
    }
    assert!(Language::canonical_filter(None).is_none());
}

/// E3-LANG-03: classifier-to-downstream implication.
/// `cached_*` / `candidate_*` return `Some` only for classifier-accepted
/// shapes (they bail on classify rejection), and a classified shape never
/// needs the external fallback. Holds over a mixed valid/invalid corpus;
/// the empty pattern is the documented exception (indexable-to-nothing).
/// INTENT: cached/candidate Some ⟹ classified (dollar-less ident fast-path excepted); classified ⟹ no fallback.
/// KILLS: implication-break, fast-path-regression.
/// ABSORBS: none (relation pin; nothing merged).
#[test]
fn classifier_acceptance_implies_downstream_presence() {
    let patterns = [
        "foo($$$)",
        "$F($$$)",
        "fn $N($$$)",
        "fn foo($$$)",
        "class $C",
        "foo",
        "fn $N($$$) { $B }",
        "fetch()?.$M($$$A)",
        "a.b($$$C).d()",
        "fn $3",
        "$A $B",
        "greet($Ü)",
        "def a(x): $$$B",
        "if ($COND) { $A; $B }",
        "->",
        ";;",
        "$Ü",
        "µµµ$A",
    ];
    for pattern in patterns {
        let classified = classify_native(pattern).is_some();
        if cached_pattern_signatures(pattern).is_some() {
            // Exact contract: the dollar-less ident fast path serves
            // without classifying; every other served shape must classify.
            let fast_path = !pattern.contains('$') && is_pattern_ident(pattern.trim());
            assert!(
                classified || fast_path,
                "cached Some without classify: {pattern:?}"
            );
        }
        if candidate_kind_signatures(pattern).is_some() {
            assert!(classified, "candidate Some without classify: {pattern:?}");
        }
        if classified {
            assert!(
                !needs_ast_grep_fallback(pattern),
                "classified yet fallback: {pattern:?}"
            );
        }
        // Repetition determinism across the linked entries.
        assert_eq!(
            cached_pattern_signatures(pattern).is_some(),
            cached_pattern_signatures(pattern).is_some()
        );
        assert_eq!(classify_native(pattern).is_some(), classified);
    }
    // Documented exception: empty is indexable-to-nothing without classifying.
    assert_eq!(cached_pattern_signatures(""), Some(vec![]));
    assert!(cached_pattern_signatures("").is_some());
    assert!(classify_native("").is_none());
}

/// E3-LANG-04: gate-denial totality.
/// Empty or kind-bearing signature sets deny EVERY pattern; statement
/// keywords deny under ANY otherwise-serving signatures; the verdict is
/// invariant under pattern padding and signature order.
/// INTENT: empty/kind sigs deny every pattern; keywords deny under any sigs; padding/order invariant.
/// KILLS: deny-hole, order-dependence.
/// ABSORBS: none (relation pin; nothing merged).
#[test]
fn gate_denial_is_total_and_order_invariant() {
    let patterns = [
        "foo",
        "foo($$$)",
        "fn $N($$$)",
        "",
        "->",
        ";;",
        "$F($$$)",
        "break",
    ];
    for pattern in patterns {
        assert!(!index_can_serve_pattern(pattern, &[]), "{pattern:?}");
        assert!(
            !index_can_serve_pattern(pattern, &["kind:call".to_string()]),
            "{pattern:?}"
        );
        assert!(
            !index_can_serve_pattern(pattern, &["foo".to_string(), "kind:call".to_string()]),
            "{pattern:?}"
        );
        // Padding invariance.
        let padded = format!("  {pattern}  ");
        let sigs = ["foo".to_string()];
        assert_eq!(
            index_can_serve_pattern(pattern, &sigs),
            index_can_serve_pattern(&padded, &sigs),
            "{pattern:?}"
        );
    }
    // Statement keywords deny regardless of signatures.
    for kw in [
        "return", "break", "continue", "import", "pass", "del", "assert", "use", "goto", "defer",
        "go", "delete", "lock", "using", "var", "unsafe", "debugger", "throw", "yield",
    ] {
        assert!(!index_can_serve_pattern(kw, &[kw.to_string()]), "{kw}");
        assert!(
            !index_can_serve_pattern(kw, &["call:foo".to_string()]),
            "{kw}"
        );
        assert!(
            !index_can_serve_pattern(kw, &["decl:fn:foo".to_string()]),
            "{kw}"
        );
    }
    // Signature-order invariance, both verdicts.
    let serving = ["call:foo".to_string(), "decl:fn:foo".to_string()];
    let swapped = ["decl:fn:foo".to_string(), "call:foo".to_string()];
    assert!(index_can_serve_pattern("foo($$$)", &serving));
    assert_eq!(
        index_can_serve_pattern("foo($$$)", &serving),
        index_can_serve_pattern("foo($$$)", &swapped)
    );
    let denying = ["foo".to_string(), "kind:call".to_string()];
    let denying_swapped = ["kind:call".to_string(), "foo".to_string()];
    assert_eq!(
        index_can_serve_pattern("foo", &denying),
        index_can_serve_pattern("foo", &denying_swapped)
    );
}

/// E3-LANG-05: prefilter-literal soundness relations.
/// A `Some` literal is always a non-empty, `$`-free, whitespace-free
/// substring of the pattern; the answer is invariant under padding and
/// repetition (never a silently-wrong filter that drops matching files).
/// INTENT: Some literal is non-empty $-free whitespace-free substring; padding/repeat invariant.
/// KILLS: unsound-literal(drops-matching-files).
/// ABSORBS: none (relation pin; nothing merged).
#[test]
fn prefilter_literal_is_sound_substring() {
    let patterns = [
        "foo",
        "foo($$$)",
        "$F($$$)",
        "fn $N($$$)",
        "fn foo($$$)",
        "if ($C) { $B }",
        "// only a comment",
        "",
        "   ",
        "$O.$M($$$)",
        "Some($A).unwrap_or($B)",
        "a?.b($X)",
        "$O.out\n.$M($A)",
        "namespace A { f(); $B }",
        "break",
        "return($X)",
        "$Ü",
    ];
    for pattern in patterns {
        let lit = required_pattern_literal(pattern);
        if let Some(ref s) = lit {
            assert!(!s.is_empty(), "{pattern:?}");
            assert!(!s.contains('$'), "{pattern:?} -> {s:?}");
            assert!(!s.chars().any(char::is_whitespace), "{pattern:?} -> {s:?}");
            assert!(pattern.contains(s), "{pattern:?} -> {s:?}");
        }
        // Padding + repetition invariance.
        let padded = format!("  {pattern}  ");
        assert_eq!(lit, required_pattern_literal(&padded), "{pattern:?}");
        assert_eq!(lit, required_pattern_literal(pattern), "{pattern:?}");
    }
}

/// E3-LANG-06: semicolon-degenerate threshold boundary (1 vs 2+).
/// A single `;` is answerable in every language; 2+ `;`s (any ascii
/// layout) are unanswerable except py/swift/kt; the whole 2+ family shares
/// one verdict per language; unanswerable members match Ok + empty (never
/// Err, never fabricated hits).
/// INTENT: 1×`;` answerable everywhere; 2+ family one verdict per lang (py/swift/kt only); match agrees.
/// KILLS: threshold-off-by-one, per-lang-divergence.
/// ABSORBS: none (relation pin; nothing merged).
#[test]
fn semicolon_count_boundary_consistent_across_languages() {
    let family = [";;", ";;;", ";;;;", "; ;", ";\n;"];
    for lang in Language::all() {
        assert!(native_pattern_answerable(*lang, ";"), "{lang}");
        let verdict = native_pattern_answerable(*lang, ";;");
        assert_eq!(
            verdict,
            matches!(lang, Language::Python | Language::Swift | Language::Kotlin),
            "{lang}"
        );
        // The 2+ family is one equivalence class per language.
        for member in family {
            assert_eq!(
                native_pattern_answerable(*lang, member),
                verdict,
                "{lang} {member:?}"
            );
            // Repetition determinism.
            assert_eq!(
                native_pattern_answerable(*lang, member),
                native_pattern_answerable(*lang, member)
            );
            // Match agrees: always Ok; unanswerable means Ok + empty.
            let hits = match_pattern(*lang, "fn f() {}", member).unwrap();
            let hits2 = match_pattern(*lang, "fn f() {}", member).unwrap();
            assert_eq!(hits, hits2);
            if !verdict {
                assert!(hits.is_empty(), "{lang} {member:?}");
            }
        }
        // Just-below (1) vs just-above (2) differ exactly outside py/swift/kt.
        assert_eq!(
            native_pattern_answerable(*lang, ";") != verdict,
            !matches!(lang, Language::Python | Language::Swift | Language::Kotlin),
            "{lang}"
        );
    }
}

/// E3-LANG-07: depth-cap threshold monotonicity + just-below/above.
/// `depth_truncated` is monotone non-decreasing in nesting depth; the
/// empirical flip point has a clear just-below side; extraction stays `Ok`
/// and deterministic on both sides; match stays `Ok` on both sides.
/// INTENT: depth_truncated monotone in nesting; bisected flip has clear-below/loud-above; match Ok both sides.
/// KILLS: nonmonotone-flag, flip-regression.
/// ABSORBS: none (relation pin; nothing merged).
#[test]
fn depth_cap_monotone_with_loud_flip() {
    let registry = ParserRegistry::new();
    let rust_src = |n: usize| format!("fn f() {{ let x = {}1{}; }}", "(".repeat(n), ")".repeat(n));
    let flag = |n: usize| {
        registry
            .parse(Language::Rust, &rust_src(n))
            .unwrap()
            .depth_truncated
    };
    // Anchor the bracket: clear at 1, breached at 300 (E1 pins the points;
    // E3 pins the monotone relation between them).
    assert!(!flag(1));
    assert!(flag(300));
    // Bisect to the first breaching depth (deep parses are expensive, so
    // search logarithmically, then verify densely around the flip).
    let (mut lo, mut hi) = (1usize, 300usize);
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if flag(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let flip = hi;
    assert!(!flag(flip - 1), "just-below must be clear");
    assert!(flag(flip), "just-above must be loud");
    // Dense monotonicity around the flip + coarse monotonicity below/above.
    let mut prev = false;
    for n in [1usize, flip / 2, flip - 1, flip, flip + 1, 300] {
        let f = flag(n);
        assert!(f >= prev, "truncation not monotone at depth {n}");
        prev = f;
    }
    // Determinism + match totality on both sides of the flip.
    for n in [1usize, flip - 1, flip, 300] {
        assert_eq!(flag(n), flag(n), "flag must repeat at depth {n}");
        assert!(
            match_pattern(Language::Rust, &rust_src(n), "f").is_ok(),
            "depth {n}"
        );
        assert!(
            match_literal_pattern(Language::Rust, &rust_src(n), "f").is_ok(),
            "depth {n}"
        );
    }
    // Same monotone contract in a second grammar (shared walk, own syntax).
    let js_src = |n: usize| format!("function f(){{var x={}1{};}}", "(".repeat(n), ")".repeat(n));
    let js_flag = |n: usize| {
        registry
            .parse(Language::JavaScript, &js_src(n))
            .unwrap()
            .depth_truncated
    };
    let mut prev_js = false;
    for n in [1usize, 150, 300] {
        let f = js_flag(n);
        assert!(f >= prev_js, "js truncation not monotone at depth {n}");
        prev_js = f;
        assert!(match_pattern(Language::JavaScript, &js_src(n), "f").is_ok());
    }
    assert_eq!(js_flag(150), js_flag(150));
}

// NOTE: `degenerate_inputs_total_and_stable` (former E3-LANG-08) was SPLIT
// per the errorapi catalog: its match/extraction repetition legs moved into
// `match_entries_agree_and_repeat_deterministically` (E3-01) and its
// pure-entry repetition legs moved into
// `padding_invariant_entries_vs_sensitive_ident` (E3-09) below, each kept as
// a no-panic sweep over DEGENERATE_PATTERNS. Net no new test.
//
// NOTE: `structural_term_signatures_shape_and_injectivity` (former E3-LANG-10)
// was DELETED per the errorapi catalog: not error-API (no failure,
// rejection, or boundary under test; kills no error mutant). The structural
// boost-signature shape belongs to the structural/boost suites.

/// E3-LANG-09: padding invariance vs padding sensitivity contrast.
/// Entries that trim (classify, signatures, literal, fallback, answerable,
/// serve-gate, structural match) are padding-invariant, and the structural
/// match additionally strips a BOM; `is_pattern_ident` is documented
/// padding-SENSITIVE — the contrast itself is the pinned relation.
/// INTENT: trimming entries padding-invariant (+BOM-strip for match) vs is_pattern_ident padding-SENSITIVE.
/// KILLS: trim-regression, ident-trim-added.
/// ABSORBS: degenerate_inputs_total_and_stable pure-entry repetition legs (kept as the degenerate no-panic sweep leg).
#[test]
fn padding_invariant_entries_vs_sensitive_ident() {
    let patterns = [
        "foo",
        "foo($$$)",
        "fn $N($$$)",
        "$A $B",
        "greet($Ü)",
        ";;",
        "->",
        "return",
        "",
        "$F($$$)",
        "if ($COND) { $A; $B }",
    ];
    for pattern in patterns {
        let padded = format!("  {pattern}  ");
        assert_eq!(
            classify_native(pattern).is_some(),
            classify_native(&padded).is_some(),
            "{pattern:?}"
        );
        assert_eq!(
            cached_pattern_signatures(pattern),
            cached_pattern_signatures(&padded)
        );
        assert_eq!(
            candidate_kind_signatures(pattern),
            candidate_kind_signatures(&padded)
        );
        assert_eq!(
            required_pattern_literal(pattern),
            required_pattern_literal(&padded)
        );
        assert_eq!(
            needs_ast_grep_fallback(pattern),
            needs_ast_grep_fallback(&padded)
        );
        assert_eq!(
            native_pattern_answerable(Language::Rust, pattern),
            native_pattern_answerable(Language::Rust, &padded)
        );
        assert_eq!(
            native_pattern_answerable(Language::Python, pattern),
            native_pattern_answerable(Language::Python, &padded)
        );
        let sigs = ["foo".to_string()];
        assert_eq!(
            index_can_serve_pattern(pattern, &sigs),
            index_can_serve_pattern(&padded, &sigs)
        );
        // Structural match: padding- and BOM-invariant, full-vector equality.
        let src = "fn f() { foo(1); }";
        assert_eq!(
            match_pattern(Language::Rust, src, pattern).unwrap(),
            match_pattern(Language::Rust, src, &padded).unwrap(),
            "{pattern:?}"
        );
        let bom = format!("\u{feff}{pattern}");
        assert_eq!(
            match_pattern(Language::Rust, src, pattern).unwrap(),
            match_pattern(Language::Rust, src, &bom).unwrap(),
            "{pattern:?}"
        );
    }
    // Contrast: the ident gate does NOT trim — padded idents are rejected.
    for ident in ["foo", "_x1", "Ü"] {
        assert!(is_pattern_ident(ident), "{ident:?}");
        assert!(!is_pattern_ident(&format!("  {ident}  ")), "{ident:?}");
    }
    assert!(!is_pattern_ident(""));
    // Absorbed degenerate leg: no-panic sweep — every Option/bool entry
    // called twice on each degenerate pattern agrees with itself (never
    // panics, never drifts).
    for pattern in DEGENERATE_PATTERNS {
        assert_eq!(
            classify_native(pattern).is_some(),
            classify_native(pattern).is_some()
        );
        assert_eq!(
            cached_pattern_signatures(pattern),
            cached_pattern_signatures(pattern)
        );
        assert_eq!(
            candidate_kind_signatures(pattern),
            candidate_kind_signatures(pattern)
        );
        assert_eq!(
            required_pattern_literal(pattern),
            required_pattern_literal(pattern)
        );
        assert_eq!(
            needs_ast_grep_fallback(pattern),
            needs_ast_grep_fallback(pattern)
        );
        assert_eq!(
            pattern_is_keyword_literal_root(pattern),
            pattern_is_keyword_literal_root(pattern)
        );
        assert_eq!(is_pattern_ident(pattern), is_pattern_ident(pattern));
        assert_eq!(
            is_universal_root_pattern(Language::Rust, pattern),
            is_universal_root_pattern(Language::Rust, pattern)
        );
        assert_eq!(
            is_universal_root_pattern(Language::Python, pattern),
            is_universal_root_pattern(Language::Python, pattern)
        );
        assert_eq!(
            php_comment_transparent_operand_lane(pattern),
            php_comment_transparent_operand_lane(pattern)
        );
        assert_eq!(
            literal_trailing_comment_lane(Language::Rust, pattern),
            literal_trailing_comment_lane(Language::Rust, pattern)
        );
        assert_eq!(
            native_pattern_answerable(Language::Rust, pattern),
            native_pattern_answerable(Language::Rust, pattern)
        );
    }
}

// ── E4 end-to-end drills (from error_api_pass4.rs) ──
// E4 end-to-end error drills for `ast-sgrep-lang`.
//
// Where E1 inventories each failure site, E2 pins stage-to-stage
// propagation, and E3 asserts cross-entry relations, E4 runs FULL
// pipelines on hostile inputs end to end:
//
// ```text
// detect -> parse/extract -> match -> score -> rank
// ```
//
// Score/rank live downstream of this crate, so the drills run the small
// deterministic score+rank lane from the `ast-sgrep-testkit` pipeline kit
// over the match hits and assert the ranked output is sound: same multiset as the
// hits (nothing invented, nothing dropped), non-increasing scores with a
// total tie-break, spans pointing inside the source, and identical output
// across repetitions.
//
// One drill per input class (empty/garbage, unicode/BOM/NUL,
// unsupported-language, hostile-pattern, fallback-loud, depth-breach) plus
// one mixed hostile-corpus drill. Each drill asserts the documented
// end-to-end outcome for its class: completion without panic, `Ok` on all
// user-influenced input, and no silently-wrong ranked output.
//
// Assertions are discriminants only (`is_some` / `is_ok` / emptiness /
// bools / equality of data values) — never message text.
//

/// E4-LANG-02 (+E4-LANG-01): empty / whitespace-only / garbage source end to
/// end. One parameterized drill over source classes: every class shares the
/// identical documented outcome — detected, Ok, empty rows, clear flag,
/// empty rank — never a panic, never invented output.
/// INTENT: empty/whitespace/garbage × every lang: detected, Ok, empty rows, flag clear, empty rank.
/// KILLS: empty-invented-output, garbage-invented-output, garbage-Err.
/// ABSORBS: garbage_source_end_to_end_rank_empty (folded as the garbage source-class leg; identical documented outcome).
#[test]
fn empty_and_garbage_source_end_to_end_rank_empty() {
    let registry = ParserRegistry::new();
    let classes: [(&str, &[&str]); 2] = [
        ("empty", &["", "   \n\t  "]),
        (
            "garbage",
            &[
                "{{{{ !!!",
                "}}}[[[",
                "@@@###$$$",
                "\x00\x01\x02\x03",
                "\u{fffd}{{{",
            ],
        ),
    ];
    for lang in Language::all() {
        let table_ext = Language::SOURCE_EXTENSIONS
            .iter()
            .find(|(_, l)| *l == *lang)
            .map(|(e, _)| *e)
            .unwrap_or(lang.as_str());
        let path = format!("n.{table_ext}");
        for (class, sources) in classes {
            for source in sources {
                let out = run_pipeline(&registry, &path, source, "foo($$$)");
                assert!(out.is_some(), "{lang} {class} {source:?}");
                let out = out.unwrap();
                assert!(out.rows_empty, "{lang} {class} {source:?}");
                assert!(!out.depth_truncated, "{lang} {class} {source:?}");
                assert!(out.ranked.is_empty(), "{lang} {class} {source:?}");
            }
        }
    }
}

// NOTE: `garbage_source_end_to_end_rank_empty` (former E4-LANG-01) was MERGED
// per the errorapi catalog into `empty_and_garbage_source_end_to_end_rank_empty`
// above: its header stated the identical documented outcome, so it survives
// as the garbage source-class leg of the one parameterized drill.

/// E4-LANG-03: unicode / BOM / NUL source end to end. Documented outcome is
/// totality, not emptiness: detect Ok, parse Ok, match Ok, and the ranked
/// output is sound (spans in-source) and deterministic across repetitions.
/// INTENT: unicode/BOM/NUL: total pipeline, spans in-source, identical repeat rank.
/// KILLS: span-out-of-source, nondeterministic-rank.
/// ABSORBS: none (drill pin; nothing merged).
#[test]
fn unicode_bom_nul_source_end_to_end_sound_and_stable() {
    let registry = ParserRegistry::new();
    let sources = [
        "💥💥💥",
        "µµµ µµµ",
        "\u{feff}",
        "\x00\x00",
        "fn föö() { bår(); }",
        "💥 fn foo() {} 💥",
    ];
    for source in sources {
        let first = run_pipeline(&registry, "n.rs", source, "foo");
        assert!(first.is_some(), "{source:?}");
        let first = first.unwrap();
        assert_spans_in_source(&first.ranked, source);
        // Determinism: the whole pipeline repeated yields identical rank.
        let second = run_pipeline(&registry, "n.rs", source, "foo").unwrap();
        assert_eq!(first.ranked, second.ranked, "{source:?}");
        assert_eq!(first.rows_empty, second.rows_empty);
        assert_eq!(first.depth_truncated, second.depth_truncated);
    }
    // Same totality contract in a second grammar.
    for source in sources {
        let out = run_pipeline(&registry, "n.py", source, "foo");
        assert!(out.is_some(), "{source:?}");
        let out = out.unwrap();
        assert_spans_in_source(&out.ranked, source);
        assert_eq!(
            out.ranked,
            run_pipeline(&registry, "n.py", source, "foo")
                .unwrap()
                .ranked
        );
    }
}

/// E4-LANG-04: unsupported-language inputs short-circuit with no ranked
/// output. Detection yields None, so no parse, no match, and no rank stage
/// ever runs — never a silent default language, never fabricated hits.
/// INTENT: unsupported inputs short-circuit None: no parse/match/rank + control.
/// KILLS: silent-default-language, fabricated-hits.
/// ABSORBS: none (drill pin; nothing merged).
#[test]
fn unsupported_language_short_circuits_with_no_ranked_output() {
    let registry = ParserRegistry::new();
    let cases = [
        ("n.fortran", "print(1)"),
        ("q.cobol", "fn foo() {}"),
        ("x.zzz", "{{{{ garbage"),
        ("Makefile", "all: foo"),
        ("n.fortran", ""),
    ];
    for (path, content) in cases {
        assert!(detect_language(Path::new(path), Some(content)).is_none());
        assert!(run_pipeline(&registry, path, content, "foo($$$)").is_none());
        assert!(run_pipeline(&registry, path, content, "").is_none());
    }
    // Positive control: a supported sibling of the same content runs.
    assert!(run_pipeline(&registry, "n.rs", "fn foo() {}", "foo($$$)").is_some());
}

/// E4-LANG-05: hostile patterns on real source end to end. Match stays Ok
/// for every hostile shape, and the ranked output is sound (exactly the
/// hits, ordered) and deterministic — never Err, never dropped hits.
/// INTENT: hostile patterns on real files: match Ok, rank sound + deterministic.
/// KILLS: dropped-hits, unsound-rank.
/// ABSORBS: none (drill pin; nothing merged).
#[test]
fn hostile_pattern_on_real_source_stays_sound() {
    let registry = ParserRegistry::new();
    let patterns = [
        "$%%%",
        "(((",
        "}}}",
        "a..b",
        "$A.$B.$C($D)",
        "fn fn fn",
        "@@@",
        "???",
        "<$A>",
        "foo(",
    ];
    let files = [
        ("n.rs", "fn foo() { foo(1); }"),
        ("n.py", "def foo():\n    foo(1)\n"),
        ("n.js", "function foo(){foo(1);}"),
    ];
    for (path, source) in files {
        for pattern in patterns {
            let first = run_pipeline(&registry, path, source, pattern);
            assert!(first.is_some(), "{path} {pattern:?}");
            let first = first.unwrap();
            assert!(!first.depth_truncated);
            // Soundness against the raw match hits.
            let lang = detect_language(Path::new(path), Some(source)).unwrap();
            let hits = match_pattern(lang, source, pattern).unwrap();
            assert_rank_sound(&first.ranked, &hits);
            assert_spans_in_source(&first.ranked, source);
            let second = run_pipeline(&registry, path, source, pattern).unwrap();
            assert_eq!(first.ranked, second.ranked, "{path} {pattern:?}");
        }
    }
}

/// E4-LANG-06: fallback-loud patterns end to end. Shapes the search ingress
/// refuses stay match-closed on the native lane: Ok + empty rank, never
/// fabricated hits, with a native-shape control answering on the same file.
/// INTENT: fallback-loud patterns rank empty end-to-end + native control answers.
/// KILLS: fabricated-hits-on-loud-shape.
/// ABSORBS: none (drill pin; nothing merged).
#[test]
fn fallback_loud_pattern_end_to_end_match_closed() {
    let registry = ParserRegistry::new();
    let source = "fn f() { greet(x); }";
    for pattern in ["greet($Ü)", "if ($COND) { $A; $B }"] {
        assert!(needs_ast_grep_fallback(pattern), "{pattern:?}");
        let out = run_pipeline(&registry, "n.rs", source, pattern).unwrap();
        assert!(out.ranked.is_empty(), "{pattern:?}");
        assert!(!out.depth_truncated);
    }
    // Positive control: the same file answers a native shape.
    let out = run_pipeline(&registry, "n.rs", source, "greet($$$)").unwrap();
    assert!(!out.ranked.is_empty());
}

/// E4-LANG-07: depth breach end to end. The cap propagates LOUD through the
/// full pipeline: extraction stays Ok with `depth_truncated` set, match
/// stays Ok, and the ranked output is sound and deterministic. The shallow
/// control keeps the flag clear.
/// INTENT: deep pipeline: loud flag + sound deterministic rank; shallow control clear.
/// KILLS: silent-breach, unsound-rank-on-deep.
/// ABSORBS: none (drill pin; nothing merged).
#[test]
fn depth_breach_end_to_end_loud_flag_with_sound_rank() {
    let registry = ParserRegistry::new();
    let deep_src = format!(
        "fn f() {{ let x = {}1{}; }}",
        "(".repeat(300),
        ")".repeat(300)
    );
    let out = run_pipeline(&registry, "n.rs", &deep_src, "f").unwrap();
    assert!(out.depth_truncated);
    let lang = detect_language(Path::new("n.rs"), Some(&deep_src)).unwrap();
    let hits = match_pattern(lang, &deep_src, "f").unwrap();
    assert_rank_sound(&out.ranked, &hits);
    assert_spans_in_source(&out.ranked, &deep_src);
    assert_eq!(
        out.ranked,
        run_pipeline(&registry, "n.rs", &deep_src, "f")
            .unwrap()
            .ranked
    );
    // Shallow control: same pipeline, flag clear.
    let shallow =
        run_pipeline(&registry, "n.rs", "fn f() { let x = (1 + (2 * 3)); }", "f").unwrap();
    assert!(!shallow.depth_truncated);
}

/// E4-LANG-08: mixed hostile-corpus drill. One pipeline run per file over a
/// corpus mixing every hostile class; the corpus-level rank (files ordered
/// by hit count desc, path asc) is deterministic across full runs, and the
/// union of ranked hits equals the union of raw hits (no invented or
/// dropped output anywhere in the corpus).
/// INTENT: corpus mixing all hostile classes: per-class outcomes, union equality, corpus-rank determinism.
/// KILLS: invented/dropped-corpus-hits, nondeterministic-corpus-rank.
/// ABSORBS: none (drill pin; nothing merged).
#[test]
fn mixed_hostile_corpus_drill() {
    let registry = ParserRegistry::new();
    let deep_src = format!(
        "fn f() {{ let x = {}1{}; }}",
        "(".repeat(300),
        ")".repeat(300)
    );
    let corpus = [
        ("a.rs", "fn foo() { foo(1); foo(2); }"),
        ("b.py", "def foo():\n    foo(1)\n"),
        ("c.js", "function foo(){foo(1);}"),
        ("empty.rs", ""),
        ("garbage.rs", "{{{{ !!! @@@"),
        ("unicode.rs", "fn föö() { bår(); } 💥"),
        ("nul.rs", "fn foo() {}\x00\x00"),
        ("deep.rs", deep_src.as_str()),
        ("skip.fortran", "fn foo() { foo(1); }"),
        ("Makefile", "all: foo"),
    ];
    let run_corpus = |pattern: &str| {
        let mut per_file: Vec<(String, Vec<PatternMatch>)> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        for (path, content) in &corpus {
            match run_pipeline(&registry, path, content, pattern) {
                Some(out) => {
                    assert_spans_in_source(&out.ranked, content);
                    per_file.push((path.to_string(), out.ranked));
                }
                None => skipped.push(path.to_string()),
            }
        }
        // Corpus rank: hit count desc, path asc (total order).
        per_file.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
        (per_file, skipped)
    };
    for pattern in ["foo($$$)", "foo"] {
        let (ranked, skipped) = run_corpus(pattern);
        // Documented per-class outcomes inside the corpus.
        assert_eq!(
            skipped,
            vec!["skip.fortran".to_string(), "Makefile".to_string()]
        );
        assert_eq!(ranked.len(), corpus.len() - skipped.len());
        // Empty and garbage members rank empty; valid members answer.
        for (path, hits) in &ranked {
            match path.as_str() {
                "empty.rs" | "garbage.rs" => assert!(hits.is_empty(), "{pattern} {path}"),
                "a.rs" | "b.py" | "c.js" => assert!(!hits.is_empty(), "{pattern} {path}"),
                _ => {}
            }
        }
        // No invented/dropped hits: corpus union equals raw-hit union.
        let mut ranked_total = 0usize;
        for (path, hits) in &ranked {
            ranked_total += hits.len();
            let content = corpus.iter().find(|(p, _)| p == path).unwrap().1;
            let lang = detect_language(Path::new(path), Some(content)).unwrap();
            let raw = match_pattern(lang, content, pattern).unwrap();
            assert_rank_sound(hits, &raw);
        }
        let mut raw_total = 0usize;
        for (path, content) in &corpus {
            if let Some(lang) = detect_language(Path::new(path), Some(content)) {
                raw_total += match_pattern(lang, content, pattern).unwrap().len();
            }
        }
        assert_eq!(ranked_total, raw_total);
        // Whole-corpus determinism across full runs.
        let (ranked_again, skipped_again) = run_corpus(pattern);
        assert_eq!(skipped, skipped_again);
        assert_eq!(ranked, ranked_again);
    }
}

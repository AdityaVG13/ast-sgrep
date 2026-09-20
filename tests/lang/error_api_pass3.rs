//! E3 negative-path metamorphic tests for `ast-sgrep-lang`.
//!
//! Where E1 inventories each failure site and E2 pins stage-to-stage
//! propagation, E3 asserts RELATIONS that must hold across entry points,
//! repetitions, and threshold boundaries:
//!
//! - Cross-entry consistency: the same invalid input through different
//!   entry points yields a consistent failure discriminant — never `Ok` in
//!   one and `Err` in another for the same contract, never `Some` in one
//!   index view and `None` in another where the contract links them.
//! - Repetition determinism: every pure query repeated returns the equal
//!   discriminant (no hash-order, no global-state drift).
//! - Degenerate-input totality: empty / whitespace / NUL / unicode / BOM
//!   inputs always produce the documented outcome — never a panic, never a
//!   silently-wrong value.
//! - Threshold-boundary consistency: just-below vs just-above the
//!   documented caps behave per contract. The lang crate has no rankers;
//!   its thresholds are `MAX_EXTRACTION_DEPTH` (256) and the degenerate
//!   `;`-root count (1 vs 2+), and the "across all rankers" leg is covered
//!   across all languages and all consuming entry points instead.
//!
//! Assertions are discriminants only (`is_some` / `is_ok` / emptiness /
//! bools / equality of data values) — never message text.

use ast_sgrep_lang::{
    cached_pattern_signatures, candidate_kind_signatures, classify_native, detect_language,
    index_can_serve_pattern, is_pattern_ident, is_universal_root_pattern,
    literal_trailing_comment_lane, match_literal_pattern, match_pattern, native_pattern_answerable,
    needs_ast_grep_fallback, pattern_is_keyword_literal_root, php_comment_transparent_operand_lane,
    required_pattern_literal, Language, ParserRegistry,
};
use std::path::Path;

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

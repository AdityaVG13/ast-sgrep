//! Pass 3 (oracle-foundry, Mission 2): L3 metamorphic / differential /
//! adversarial oracles for the lang surface pass 1–2 did not cover.
//!
//! Pass 1–2 own: dot/cosine/normalize basics, top_by incl heap-vs-sort,
//! embed roundtrip, split_ident, tokenize hand-sets, expand_concepts trigger
//! precision, keyword roots, match_pattern literal/trivia. This pass owns:
//! similarity symmetry/bounds, normalization idempotence, ranking stability
//! under permutation/ties, tokenize order-invariance, expand superset,
//! embed determinism/empty-zero, Language parse/extension/detect tables,
//! is_pattern_ident admission, classify trim/modifier invariance, signature
//! serve consistency, literal-lane differential + match monotonicity,
//! connector carves, and adversarial vectors/deep-nesting.
//!
//! Expectations are hand-computed; errors assert discriminants
//! (`is_err`/`is_none`/emptiness), never message text.

use ast_sgrep_embed::{
    cosine_similarity, dot_similarity, expand_concepts, normalize_vec, normalize_vec_in_place,
    tokenize, top_by_similarity, top_k_flat_similarity, SemanticLocalEmbedding, SEMANTIC_DIM,
};
use ast_sgrep_lang::{
    cached_pattern_signatures, candidate_kind_signatures, classify_native, detect_language,
    index_can_serve_pattern, is_pattern_ident, match_literal_pattern, match_pattern,
    needs_ast_grep_fallback, required_pattern_literal, structural_term_signatures, Language,
    ParserRegistry,
};

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-6
}

// ─── Metamorphic: similarity symmetry and cosine bounds ─────────────────────

#[test]
fn similarity_symmetric_and_cosine_bounded() {
    // Hand dot: 2.5*-4 + -1*2 + 0.5*8 = -10 - 2 + 4 = -8 (exact).
    let a = [2.5f32, -1.0, 0.5];
    let b = [-4.0f32, 2.0, 8.0];
    assert_eq!(dot_similarity(&a, &b), -8.0);
    assert_eq!(dot_similarity(&a, &b), dot_similarity(&b, &a));
    // Cosine is symmetric and bounded; self-similarity of a nonzero vector
    // is 1. Hand: cos([3,4],[4,3]) = 24/25 = 0.96.
    assert!(approx(cosine_similarity(&[3.0, 4.0], &[4.0, 3.0]), 0.96));
    let pairs: &[(&[f32], &[f32])] = &[
        (&[3.0, 4.0], &[4.0, 3.0]),
        (&[1.0, 1.0], &[-1.0, -1.0]),
        (&[2.5, -1.0, 0.5], &[-4.0, 2.0, 8.0]),
        (&[0.0, 5.0], &[0.0, -7.0]),
    ];
    for (x, y) in pairs {
        let xy = cosine_similarity(x, y);
        let yx = cosine_similarity(y, x);
        assert!(approx(xy, yx), "asymmetric {x:?} {y:?}: {xy} vs {yx}");
        assert!(xy.abs() <= 1.0 + 1e-6, "out of bounds: {xy}");
    }
    assert!(approx(cosine_similarity(&a, &a), 1.0));
    assert!(approx(cosine_similarity(&[1.0, 1.0], &[-1.0, -1.0]), -1.0));
}

// ─── Metamorphic + differential: normalization ──────────────────────────────

#[test]
fn normalize_idempotent_and_inplace_agrees() {
    // Hand: norm([3,4,-12]) = 13, so n = [3/13,4/13,-12/13].
    let v = [3.0f32, 4.0, -12.0];
    let once = normalize_vec(&v);
    assert!(approx(once[0], 3.0 / 13.0) && approx(once[2], -12.0 / 13.0));
    // Idempotence: normalizing a normalized vector is a fixed point.
    let twice = normalize_vec(&once);
    for (x, y) in once.iter().zip(twice.iter()) {
        assert!(approx(*x, *y), "not idempotent: {once:?} vs {twice:?}");
    }
    // Unit norm of the fixed point.
    let norm: f32 = once.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!(approx(norm, 1.0), "norm={norm}");
    // Differential: in-place and out-of-place paths agree bit-exactly.
    let mut inplace = v.to_vec();
    normalize_vec_in_place(&mut inplace);
    assert_eq!(inplace, once);
    // Fixed points of the degenerate classes are exact.
    assert_eq!(normalize_vec(&[0.0, 0.0]), vec![0.0, 0.0]);
    assert_eq!(normalize_vec(&normalize_vec(&[0.0, 0.0])), vec![0.0, 0.0]);
    assert_eq!(normalize_vec(&[f32::INFINITY, -3.0]), vec![0.0, -1.0]);
}

// ─── Metamorphic: ranking stability under permutation and ties ──────────────

#[test]
fn ranking_invariant_under_input_permutation_and_ties() {
    // Hand-sorted: score desc, ties by ascending index.
    let expected = vec![(0, 0.9), (1, 0.9), (2, 0.9), (3, 0.5), (4, 0.2)];
    let base = vec![(4, 0.2), (2, 0.9), (0, 0.9), (3, 0.5), (1, 0.9)];
    let mut reversed = base.clone();
    reversed.reverse();
    let mut rotated = base.clone();
    rotated.rotate_left(2);
    for perm in [base, reversed, rotated] {
        assert_eq!(top_by_similarity(perm, 10, None), expected);
    }
    // Limit beyond the corpus returns every survivor without padding.
    assert_eq!(
        top_by_similarity(vec![(1, 0.5), (0, 0.5)], 100, None),
        vec![(0, 0.5), (1, 0.5)]
    );
    // All-tie input still resolves to ascending index.
    assert_eq!(
        top_by_similarity(vec![(2, 0.5), (0, 0.5), (1, 0.5)], 10, None),
        vec![(0, 0.5), (1, 0.5), (2, 0.5)]
    );
}

// ─── Metamorphic: tokenize order-invariance and determinism ─────────────────

#[test]
fn tokenize_order_invariant_and_deterministic() {
    assert_eq!(tokenize("zebra apple"), tokenize("apple zebra"));
    assert_eq!(tokenize("zebra apple"), vec!["apple", "zebra"]);
    // Layout around tokens is invisible.
    assert_eq!(tokenize("  zebra   apple  "), vec!["apple", "zebra"]);
    // Hand: whole lowercased plus camel parts, sorted.
    assert_eq!(tokenize("HelloWorld"), vec!["hello", "helloworld", "world"]);
    // Determinism: repeated calls agree bit-exactly.
    assert_eq!(tokenize("FooBar_baz qux"), tokenize("FooBar_baz qux"));
}

// ─── Metamorphic: expand_concepts superset + empty ──────────────────────────

#[test]
fn expand_concepts_superset_and_empty() {
    // Empty query: tokens and parts are empty, so the format is one space.
    assert_eq!(expand_concepts(""), " ");
    // Expansion only ADDS terms: every query token survives in the output.
    for query in ["throttle inbound", "zxqy qwerty", "remember query embeddings"] {
        let expanded = expand_concepts(query);
        for token in tokenize(query) {
            assert!(
                tokenize(&expanded).contains(&token),
                "lost {token:?} in {expanded:?}"
            );
        }
        assert_eq!(expand_concepts(query), expand_concepts(query));
    }
    // Hand trigger: "throttle" fires the rate group terms.
    let expanded = expand_concepts("throttle inbound");
    for token in ["rate", "limit", "quota"] {
        assert!(expanded.contains(token), "{expanded:?}");
    }
}

// ─── Adversarial + differential: embed determinism and the empty zero ───────

#[test]
fn embed_text_deterministic_zero_for_empty() {
    let embedder = SemanticLocalEmbedding;
    // Empty text yields no tokens and no trigrams: the exact zero vector.
    assert_eq!(embedder.embed_text(""), vec![0.0; SEMANTIC_DIM]);
    // Determinism and shape on a real query.
    let a = embedder.embed_text("credential renewal");
    assert_eq!(a.len(), SEMANTIC_DIM);
    assert_eq!(a, embedder.embed_text("credential renewal"));
    assert!(a.iter().all(|x| x.is_finite()));
    // Normalized embeddings have unit self-similarity.
    assert!((dot_similarity(&a, &a) - 1.0).abs() < 1e-5);
    // Differential: the provider method agrees with the free function.
    let b = embedder.embed_text("sanitize user input");
    assert_eq!(embedder.similarity(&a, &b), dot_similarity(&a, &b));
}

// ─── Differential: Language parse / extension / detect tables ───────────────

#[test]
fn language_parse_roundtrip_and_extension_table() {
    // Every stored id parses back to itself and canonicalizes to itself.
    for lang in Language::all() {
        assert_eq!(Language::parse(lang.as_str()), Some(*lang));
        assert_eq!(
            Language::canonical_filter(Some(lang.as_str())).as_deref(),
            Some(lang.as_str())
        );
    }
    // Hand alias table (case/whitespace-tolerant). Extension spellings
    // resolve through from_extension; name spellings only through parse.
    let ext_aliases: &[(&str, Language)] = &[
        ("RS", Language::Rust),
        ("HPP", Language::Cpp),
        ("Ts", Language::TypeScript),
        (" h ", Language::C),
    ];
    for (raw, lang) in ext_aliases {
        assert_eq!(Language::parse(raw), Some(*lang), "{raw:?}");
        assert_eq!(Language::from_extension(raw), Some(*lang), "{raw:?}");
    }
    let name_aliases: &[(&str, Language)] = &[
        ("golang", Language::Go),
        ("C#", Language::CSharp),
        ("c++", Language::Cpp),
        ("cuda", Language::Cpp),
    ];
    for (raw, lang) in name_aliases {
        assert_eq!(Language::parse(raw), Some(*lang), "{raw:?}");
        assert_eq!(Language::from_extension(raw), None, "{raw:?}");
    }
    // Differential: detect_language agrees with from_extension on every
    // indexed extension, in both cases.
    for (ext, lang) in Language::SOURCE_EXTENSIONS {
        let rel = format!("n.{ext}");
        assert_eq!(
            detect_language(std::path::Path::new(&rel), None),
            Some(*lang),
            "detect({ext})"
        );
        assert_eq!(
            detect_language(std::path::Path::new(&rel), None),
            Language::from_extension(ext)
        );
        assert_eq!(
            Language::from_extension(&ext.to_ascii_uppercase()),
            Some(*lang)
        );
    }
    // Shebang/content sniffing when no extension matches.
    let no_ext = std::path::Path::new("Makefile");
    assert_eq!(
        detect_language(no_ext, Some("package main\n")),
        Some(Language::Go)
    );
    assert_eq!(
        detect_language(no_ext, Some("#!/usr/bin/env python\n")),
        Some(Language::Python)
    );
    assert_eq!(
        detect_language(no_ext, Some("#!/usr/bin/ruby\n")),
        Some(Language::Ruby)
    );
    assert_eq!(
        detect_language(no_ext, Some("<?php echo 1;")),
        Some(Language::Php)
    );
    // Unknown extension and unrecognized content stay silent None.
    assert_eq!(
        detect_language(std::path::Path::new("x.zzz"), None),
        None
    );
    assert_eq!(detect_language(no_ext, Some("just text")), None);
    assert_eq!(Language::parse(""), None);
    assert_eq!(Language::parse("fortran"), None);
}

// ─── Hand table: is_pattern_ident admission ─────────────────────────────────

#[test]
fn is_pattern_ident_admission_table() {
    // Unicode alphabetic/alphanumeric code points are admitted.
    for ident in ["foo", "_", "_a1", "A", "αβ", "café"] {
        assert!(is_pattern_ident(ident), "{ident:?}");
    }
    for ident in ["", "1a", "a-b", "a b", "foo(", "$A", "::", "->"] {
        assert!(!is_pattern_ident(ident), "{ident:?}");
    }
}

// ─── Metamorphic: classify trim- and modifier-invariance ────────────────────

#[test]
fn classify_native_trim_and_modifier_invariance() {
    // Surrounding trivia never changes classification.
    assert_eq!(
        classify_native("  fn $NAME($$$)  "),
        classify_native("fn $NAME($$$)")
    );
    assert!(classify_native("fn $NAME($$$)").is_some());
    // Declaration modifiers strip to the bare spelling.
    assert_eq!(
        classify_native("pub fn $NAME"),
        classify_native("fn $NAME")
    );
    assert_eq!(
        classify_native("export function $NAME"),
        classify_native("function $NAME")
    );
    assert_eq!(
        classify_native("pub(crate) fn $NAME"),
        classify_native("fn $NAME")
    );
    // Loud rejects: case-sensitive prefixes, garbage meta heads, empty.
    assert!(classify_native("Fn $NAME").is_none());
    assert!(classify_native("fn $3").is_none());
    assert!(classify_native("").is_none());
}

// ─── Hand consistency: signature derivation vs index-serve ──────────────────

#[test]
fn signature_serve_consistency() {
    // Ident and concrete decl shapes are index-exact.
    assert_eq!(
        cached_pattern_signatures("foo"),
        Some(vec!["foo".to_string()])
    );
    assert!(index_can_serve_pattern("foo", &["foo".to_string()]));
    assert_eq!(
        cached_pattern_signatures("fn greet"),
        Some(vec!["decl:fn:greet".to_string()])
    );
    assert!(index_can_serve_pattern("fn greet", &["decl:fn:greet".to_string()]));
    // Kind-only shapes always need native confirmation.
    let kinds = vec![
        "kind:function_item".to_string(),
        "kind:function_definition".to_string(),
        "kind:impl_definition".to_string(),
        "kind:named_lambda_expression".to_string(),
    ];
    assert_eq!(cached_pattern_signatures("fn $NAME"), Some(kinds.clone()));
    assert!(!index_can_serve_pattern("fn $NAME", &kinds));
    assert!(!index_can_serve_pattern("fn $NAME() { $$$BODY }", &[]));
    // Statement keywords escape ident-serve even with ident rows.
    assert!(!index_can_serve_pattern("return", &["return".to_string()]));
    assert!(!index_can_serve_pattern("debugger", &["debugger".to_string()]));
    assert_eq!(cached_pattern_signatures(""), Some(vec![]));
    // Prefilter literals: exact ident, longest token, meta-name hole, if head.
    assert_eq!(required_pattern_literal("foo"), Some("foo".to_string()));
    assert_eq!(
        required_pattern_literal("fn greet"),
        Some("greet".to_string())
    );
    assert_eq!(required_pattern_literal("fn $NAME"), None);
    assert_eq!(
        required_pattern_literal("if $C { $B }"),
        Some("if".to_string())
    );
    assert_eq!(required_pattern_literal(""), None);
    // Braced decl templates narrow by kind but never serve exactly.
    assert_eq!(
        candidate_kind_signatures("fn $N($$$) { $$$ }"),
        Some(kinds)
    );
    assert_eq!(candidate_kind_signatures(""), None);
    // Structural boost keys stay byte-identical.
    assert_eq!(
        structural_term_signatures("foo"),
        [
            "call-name:foo".to_string(),
            "call:foo".to_string(),
            "decl:fn:foo".to_string(),
            "decl:def:foo".to_string(),
            "decl:function:foo".to_string(),
            "foo".to_string(),
        ]
    );
}

// ─── Differential: literal lanes agree; matches grow monotonically ──────────

#[test]
fn literal_differential_and_match_monotonicity() {
    let source = "fn foo() { foo(); }\n";
    let unified = match_pattern(Language::Rust, source, "foo").expect("unified");
    let direct = match_literal_pattern(Language::Rust, source, "foo").expect("direct");
    assert!(!unified.is_empty());
    assert_eq!(unified, direct);
    // Empty source: both lanes agree on the honest empty.
    assert!(
        match_pattern(Language::Rust, "", "foo")
            .expect("empty unified")
            .is_empty()
    );
    assert!(
        match_literal_pattern(Language::Rust, "", "foo")
            .expect("empty direct")
            .is_empty()
    );
    // Monotonicity: appending another occurrence strictly grows the hit set.
    let one = match_pattern(Language::Rust, "fn foo() {}\n", "foo").expect("one");
    let two = match_pattern(Language::Rust, "fn foo() {}\nfn foo() {}\n", "foo").expect("two");
    assert!(!one.is_empty());
    assert!(two.len() > one.len(), "{} vs {}", two.len(), one.len());
    // Adversarial unicode ident: total (no error) and deterministic.
    let uni = "fn αβ() {}\n";
    let first = match_pattern(Language::Rust, uni, "αβ").expect("unicode ok");
    let second = match_pattern(Language::Rust, uni, "αβ").expect("unicode ok");
    assert_eq!(first, second);
    assert!(!first.is_empty(), "unicode ident must match");
}

// ─── Hand carves: connectors match nothing; $-less never falls back ─────────

#[test]
fn connector_carve_and_fallback_class() {
    // Bare connectors are lenient-parse garbage: silent empty even where the
    // bytes occur in the source.
    let arrow = match_pattern(Language::Rust, "fn f() -> i32 { 1 }\n", "->").expect("arrow");
    assert!(arrow.is_empty());
    let scope = match_pattern(Language::Rust, "use a::b;\nfn f() {}\n", "::").expect("scope");
    assert!(scope.is_empty());
    // The $-less class never needs the external engine, whatever the bytes.
    for pattern in [
        "",
        "   ",
        "αβ",
        "->",
        "::",
        ";",
        "foo(1, 2)",
        "fn foo() { }",
        "if (x) { y; }",
    ] {
        assert!(
            !needs_ast_grep_fallback(pattern),
            "{pattern:?} must stay in-process"
        );
    }
}

// ─── Adversarial: nonfinite/huge vectors and deep-nesting extraction ────────

#[test]
fn adversarial_vectors_and_deep_extraction() {
    // SIMD-length all-NaN vectors collapse to zero, never NaN.
    let nan70 = vec![f32::NAN; 70];
    let one70 = vec![1.0f32; 70];
    assert_eq!(dot_similarity(&nan70, &one70), 0.0);
    assert_eq!(cosine_similarity(&nan70, &one70), 0.0);
    // A NaN query scores every row exactly 0.0 (skipped pairs, zero norms
    // guarded) and the unthresholded lane keeps both rows, tie by index.
    assert_eq!(
        top_k_flat_similarity(&[f32::NAN, f32::NAN], &[1.0, 0.0, 0.0, 1.0], 2, 5, None),
        vec![(0, 0.0), (1, 0.0)]
    );
    // Huge dim stays finite and exact-class: 100k ones sum to 100000.
    let huge = vec![1.0f32; 100_000];
    let total = dot_similarity(&huge, &huge);
    assert!((total - 100_000.0).abs() < 1.0, "total={total}");
    // Extraction: shallow file is complete; 300-deep parens breach the
    // 256 cap (loud truncation flag, still Ok); empty is Ok-empty.
    let registry = ParserRegistry::new();
    let shallow = registry.parse(Language::Rust, "fn f() {}\n").expect("shallow");
    assert!(!shallow.depth_truncated);
    assert_eq!(shallow.symbols.len(), 1);
    assert_eq!(shallow.symbols[0].name, "f");
    let deep_src = format!("fn f() {{ let x = {}1{}; }}\n", "(".repeat(300), ")".repeat(300));
    let deep = registry.parse(Language::Rust, &deep_src).expect("deep ok");
    assert!(deep.depth_truncated, "300-deep parens must truncate");
    let empty = registry.parse(Language::Rust, "").expect("empty ok");
    assert!(empty.symbols.is_empty());
    assert!(!empty.depth_truncated);
}

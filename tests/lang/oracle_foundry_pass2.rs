//! Pass 2 (oracle-foundry, Mission 2): L2 mutation-discriminating oracles for
//! embed math/rerank/tokenizer/concept contracts and lang pattern gates.
//!
//! Each test names the mutant class it kills: strict-vs-inclusive threshold
//! flips, heap-vs-sort ranker divergence, div-by-zero guard removal,
//! NaN-poisoning, trigger-group drops, and literal-gate mistables.
//! Expectations are hand-computed; errors assert discriminants.

use ast_sgrep_embed::{
    cosine_similarity, dot_similarity, expand_concepts, normalize_vec, normalize_vec_in_place,
    split_ident, tokenize, top_by_similarity, top_k_flat_similarity, top_k_similarity,
    MIN_SIMILARITY,
};
use ast_sgrep_lang::{match_pattern, pattern_is_keyword_literal_root, Language};

#[test]
fn threshold_is_strict_nextafter_not_gte() {
    // Kills: `>` flipped to `>=` (equal scores would pass), the min == 0.0
    // denormal step dropped, and Inf slipping through the finite filter.
    assert!(top_by_similarity(vec![(0, 0.0)], 10, Some(0.0)).is_empty());
    assert_eq!(
        top_by_similarity(vec![(0, f32::MIN_POSITIVE)], 10, Some(0.0)).len(),
        1
    );
    assert!(top_by_similarity(vec![(0, -1.0)], 10, Some(-1.0)).is_empty());
    assert_eq!(
        top_by_similarity(vec![(0, -0.5)], 10, Some(-1.0)).len(),
        1
    );
    assert!(top_by_similarity(vec![(0, MIN_SIMILARITY)], 10, Some(MIN_SIMILARITY)).is_empty());
    assert!(top_by_similarity(vec![(0, f32::INFINITY)], 10, None).is_empty());
}

#[test]
fn heap_and_sort_rankers_agree_on_adversarial_corpus() {
    // Kills: heap-vs-sort divergence (NaN/Inf filtering, tie-break direction,
    // exclusive-threshold mismatch) in either ranker. Both must produce the
    // hand-sorted order: score desc, ties by ascending index.
    let corpus = vec![
        (5, 0.9),
        (2, 0.9),
        (7, f32::NAN),
        (3, 0.5),
        (9, f32::INFINITY),
        (1, 0.7),
        (4, -2.0),
        (6, 0.5001),
    ];
    let expected = vec![(2, 0.9), (5, 0.9), (1, 0.7)];
    assert_eq!(top_by_similarity(corpus.clone(), 3, Some(0.5)), expected);
    assert_eq!(
        top_k_similarity(corpus, 3, Some(0.5)),
        expected
    );
}

#[test]
fn top_k_flat_guards_division_and_shape() {
    // Kills: `checked_div` replaced by `/ dim` (panics on dim 0 instead of
    // returning empty), and shape-mismatch guards dropped.
    assert!(top_k_flat_similarity(&[1.0], &[1.0, 0.0], 0, 5, None).is_empty());
    assert!(top_k_flat_similarity(&[1.0, 0.0, 0.0], &[1.0, 0.0], 2, 5, None).is_empty());
    assert!(top_k_flat_similarity(&[1.0, 0.0], &[], 2, 5, None).is_empty());
    assert!(top_k_flat_similarity(&[1.0, 0.0], &[1.0, 0.0], 2, 0, None).is_empty());
    let ranked = top_k_flat_similarity(&[1.0, 0.0], &[1.0, 0.0, 0.0, 1.0], 2, 5, None);
    assert_eq!(ranked.len(), 2);
    assert_eq!(ranked[0].0, 0);
    assert!((ranked[0].1 - 1.0).abs() < 1e-6, "got {ranked:?}");
    assert_eq!(ranked[1].0, 1);
}

#[test]
fn normalize_zero_and_nonfinite_have_no_nan() {
    // Kills: non-finite zeroing removed (inf/inf -> NaN) and the zero-norm
    // guard dropped (0/0 -> NaN instead of the zero vector).
    assert_eq!(normalize_vec(&[0.0, 0.0]), vec![0.0, 0.0]);
    let poisoned = normalize_vec(&[f32::INFINITY, f32::NEG_INFINITY, f32::NAN]);
    assert_eq!(poisoned, vec![0.0, 0.0, 0.0]);
    assert!(!poisoned.iter().any(|x| x.is_nan()));
    let mut inplace = vec![1.0, f32::NAN];
    normalize_vec_in_place(&mut inplace);
    assert_eq!(inplace, vec![1.0, 0.0]);
}

#[test]
fn dot_simd_and_scalar_paths_agree() {
    // Kills: SIMD (>= 64 lanes) vs scalar divergence; hand sums are exact.
    let ones = vec![1.0f32; 64];
    let twos = vec![2.0f32; 64];
    assert_eq!(dot_similarity(&ones, &twos), 128.0);
    assert_eq!(dot_similarity(&vec![1.0f32; 63], &vec![1.0f32; 63]), 63.0);
    let alt: Vec<f32> = (0..100).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect();
    assert_eq!(dot_similarity(&alt, &vec![1.0f32; 100]), 0.0);
}

#[test]
fn cosine_skips_nonfinite_pairs_and_normalizes() {
    // Kills: NaN-poisoning (fold without the finite guard), zero-norm NaN,
    // and unnormalized-dot (parallel [1,1]/[2,2] must be 1.0, not 6.0).
    assert_eq!(cosine_similarity(&[f32::NAN, 1.0], &[0.0, 1.0]), 1.0);
    assert_eq!(cosine_similarity(&[f32::INFINITY, 0.0], &[1.0, 1.0]), 0.0);
    let parallel = cosine_similarity(&[1.0, 1.0], &[2.0, 2.0]);
    assert!((parallel - 1.0).abs() < 1e-6, "got {parallel}");
}

#[test]
fn independent_dot_oracle_agrees() {
    // Deliberately different accumulation (f64 fold) over dyadic-exact
    // vectors, plus hand totals. Kills zip-truncation and sign mutants.
    let cases: &[(&[f32], &[f32], f64)] = &[
        (&[1.5, -2.25, 3.0], &[4.0, 0.5, -1.0], 1.875),
        (&[0.5; 8], &[0.25; 8], 1.0),
    ];
    for (a, b, expected) in cases {
        assert_eq!(dot_similarity(a, b) as f64, *expected);
        let folded: f64 = a
            .iter()
            .zip(b.iter())
            .map(|(x, y)| f64::from(*x) * f64::from(*y))
            .sum();
        assert!((folded - expected).abs() < 1e-9, "folded={folded}");
    }
}

#[test]
fn split_ident_camel_and_acronym_tables() {
    // Kills: per-capital splits (HTTPStatusCode), digit-edge boundary flips,
    // and separator handling swaps.
    assert_eq!(split_ident("HTTPStatusCode"), vec!["httpstatus", "code"]);
    assert_eq!(split_ident("refreshToken"), vec!["refresh", "token"]);
    assert_eq!(split_ident("a1B2"), vec!["a1", "b2"]);
    assert_eq!(split_ident("FooBAR"), vec!["foo", "bar"]);
    assert_eq!(split_ident("A"), vec!["a"]);
}

#[test]
fn tokenize_dedups_and_sorts() {
    // Kills: dedup-set replaced by plain push, and the final sort dropped
    // (HashSet order would flake the sorted assertion).
    assert_eq!(tokenize("foo foo"), vec!["foo"]);
    assert_eq!(tokenize("zebra apple"), vec!["apple", "zebra"]);
    assert_eq!(tokenize("ab-CD"), vec!["ab", "cd"]);
}

#[test]
fn expand_concepts_trigger_precision() {
    // Kills: combine->conjunction group dropped, and channels merged into
    // the rrf/fusion group (steals AND queries for ranking).
    let query = "combine two search channels in a single query";
    let expanded = expand_concepts(query);
    let tokens = tokenize(&expanded);
    assert!(tokens.contains(&"conjunction".to_string()), "{expanded:?}");
    assert!(!tokens.contains(&"rrf".to_string()), "{expanded:?}");
    assert!(!tokens.contains(&"fusion".to_string()), "{expanded:?}");
    let evicted = expand_concepts("eviction");
    for token in ["prune", "cache", "stale"] {
        assert!(evicted.contains(token), "{evicted:?}");
    }
    assert_eq!(expand_concepts("zxqy qwerty"), "zxqy qwerty qwerty zxqy");
}

#[test]
fn keyword_literal_roots_match_exact_table() {
    // Kills: matches!-arm drops/additions, trim removal, case-folding.
    for word in [
        "null", "true", "false", "True", "False", "None", "this", "super", " null ",
    ] {
        assert!(pattern_is_keyword_literal_root(word), "{word:?}");
    }
    for word in ["", "undefined", "self", "nul", "NULL", "Truex", "none"] {
        assert!(!pattern_is_keyword_literal_root(word), "{word:?}");
    }
}

#[test]
fn match_pattern_literal_and_trivia_edges() {
    // Kills: literal-lane removal (no hits for a present ident), trim
    // removal (whitespace-only would error/misbehave), BOM-strip removal,
    // and error-on-absent (missing ident must be Ok-empty, not Err).
    let source = "fn foo() { foo(); }\n";
    assert!(!match_pattern(Language::Rust, source, "foo").expect("literal").is_empty());
    assert!(match_pattern(Language::Rust, source, "   ").expect("ws").is_empty());
    assert!(!match_pattern(Language::Rust, source, "\u{feff}foo").expect("bom").is_empty());
    assert!(match_pattern(Language::Rust, source, "absent_ident").expect("absent").is_empty());
}

//! Consolidated oracle suite: embed similarity kernels (dot / cosine /
//! normalize) plus the text-embedding differential.
//!
//! Replaces `oracle_foundry_pass{1,2,3}.rs` similarity legs. Expectations are
//! hand-computed; errors assert discriminants, never messages.

use ast_sgrep_embed::{
    cosine_similarity, dot_similarity, normalize_vec, normalize_vec_in_place,
    SemanticLocalEmbedding, SEMANTIC_DIM,
};
use ast_sgrep_testkit::approx_eq;

/// INTENT: dot and cosine equal hand values on exact, degenerate, SIMD-lane,
/// and adversarial inputs; both are symmetric and cosine is bounded.
///
/// KILLS: degenerate-guard-removal (empty/mismatch/Inf/NaN→0), zero-norm-NaN,
/// sign/zip-truncation, NaN-poisoning-fold, unnormalized-dot, SIMD-vs-scalar
/// lane-boundary divergence, asymmetric-accumulation, unbounded-cosine.
///
/// ABSORBS: pass1::dot_similarity_matches_hand_products,
/// pass1::cosine_similarity_matches_hand_angles,
/// pass2::independent_dot_oracle_agrees,
/// pass2::cosine_skips_nonfinite_pairs_and_normalizes,
/// pass2::dot_simd_and_scalar_paths_agree,
/// pass3::similarity_symmetric_and_cosine_bounded.
#[test]
fn dot_and_cosine_kernels_match_hand_values() {
    // Hand dot products; degenerate inputs collapse to zero, never NaN/panic.
    assert_eq!(dot_similarity(&[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0]), 32.0);
    assert_eq!(dot_similarity(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
    assert_eq!(dot_similarity(&[], &[]), 0.0);
    assert_eq!(dot_similarity(&[1.0], &[1.0, 2.0]), 0.0);
    assert_eq!(dot_similarity(&[f32::INFINITY], &[1.0]), 0.0);
    assert_eq!(dot_similarity(&[f32::NAN], &[1.0]), 0.0);
    // Dyadic-exact hand totals plus a deliberately different f64-fold leg
    // (the fold half alone is BEHAVIOR-ONLY; the hand totals carry the kill).
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
    // SIMD (≥64 lanes) vs scalar: exact sums incl the 63/64 boundary.
    let ones = vec![1.0f32; 64];
    let twos = vec![2.0f32; 64];
    assert_eq!(dot_similarity(&ones, &twos), 128.0);
    assert_eq!(dot_similarity(&vec![1.0f32; 63], &vec![1.0f32; 63]), 63.0);
    let alt: Vec<f32> = (0..100)
        .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
        .collect();
    assert_eq!(dot_similarity(&alt, &vec![1.0f32; 100]), 0.0);
    // Hand dot: 2.5*-4 + -1*2 + 0.5*8 = -8 (exact); dot is symmetric.
    let a = [2.5f32, -1.0, 0.5];
    let b = [-4.0f32, 2.0, 8.0];
    assert_eq!(dot_similarity(&a, &b), -8.0);
    assert_eq!(dot_similarity(&a, &b), dot_similarity(&b, &a));

    // Hand cosine angles; degenerate inputs collapse to 0.0.
    assert_eq!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]), 0.0);
    assert!(approx_eq(
        cosine_similarity(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]),
        1.0
    ));
    assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    assert_eq!(cosine_similarity(&[], &[]), 0.0);
    assert_eq!(cosine_similarity(&[1.0], &[1.0, 1.0]), 0.0);
    // NaN pairs are skipped (not poisoning), zero norms guarded, and the
    // result is normalized: parallel [1,1]/[2,2] is 1.0, not 6.0.
    assert_eq!(cosine_similarity(&[f32::NAN, 1.0], &[0.0, 1.0]), 1.0);
    assert_eq!(cosine_similarity(&[f32::INFINITY, 0.0], &[1.0, 1.0]), 0.0);
    let parallel = cosine_similarity(&[1.0, 1.0], &[2.0, 2.0]);
    assert!((parallel - 1.0).abs() < 1e-6, "got {parallel}");
    // Hand: cos([3,4],[4,3]) = 24/25 = 0.96; symmetric and bounded ±1.
    assert!(approx_eq(cosine_similarity(&[3.0, 4.0], &[4.0, 3.0]), 0.96));
    let pairs: &[(&[f32], &[f32])] = &[
        (&[3.0, 4.0], &[4.0, 3.0]),
        (&[1.0, 1.0], &[-1.0, -1.0]),
        (&[2.5, -1.0, 0.5], &[-4.0, 2.0, 8.0]),
        (&[0.0, 5.0], &[0.0, -7.0]),
    ];
    for (x, y) in pairs {
        let xy = cosine_similarity(x, y);
        let yx = cosine_similarity(y, x);
        assert!(approx_eq(xy, yx), "asymmetric {x:?} {y:?}: {xy} vs {yx}");
        assert!(xy.abs() <= 1.0 + 1e-6, "out of bounds: {xy}");
    }
    assert!(approx_eq(cosine_similarity(&a, &a), 1.0));
    assert!(approx_eq(
        cosine_similarity(&[1.0, 1.0], &[-1.0, -1.0]),
        -1.0
    ));
}

/// INTENT: normalize matches hand norms, is idempotent with unit norm,
/// bit-agrees across in-place/out-of-place paths, and maps every degenerate
/// class (zero, empty, all-nonfinite) to an exact NaN-free fixed point.
///
/// KILLS: nonfinite-zeroing-removal, zero-norm-guard-drop (0/0 and inf/inf
/// NaN), non-idempotent-normalize, in-place divergence, degenerate
/// fixed-point drift.
///
/// ABSORBS: pass1::normalize_vec_matches_hand_norms,
/// pass2::normalize_zero_and_nonfinite_have_no_nan,
/// pass3::normalize_idempotent_and_inplace_agrees.
///
/// DEDUP: the zero-vector fixed point was pinned in all three absorbed tests;
/// it is pinned ONCE here.
#[test]
fn normalize_is_idempotent_zero_safe_map() {
    // Hand: [3,4]→[0.6,0.8]; nonfinite components zeroed before norm.
    let n = normalize_vec(&[3.0, 4.0]);
    assert!(approx_eq(n[0], 0.6) && approx_eq(n[1], 0.8), "got {n:?}");
    assert_eq!(normalize_vec(&[f32::NAN, 1.0]), vec![0.0, 1.0]);
    assert_eq!(normalize_vec(&[]), Vec::<f32>::new());
    // Degenerate fixed points are exact and NaN-free (pinned once).
    assert_eq!(normalize_vec(&[0.0, 0.0]), vec![0.0, 0.0]);
    let poisoned = normalize_vec(&[f32::INFINITY, f32::NEG_INFINITY, f32::NAN]);
    assert_eq!(poisoned, vec![0.0, 0.0, 0.0]);
    assert!(!poisoned.iter().any(|x| x.is_nan()));
    let mut inplace = vec![1.0, f32::NAN];
    normalize_vec_in_place(&mut inplace);
    assert_eq!(inplace, vec![1.0, 0.0]);
    // Hand: norm([3,4,-12]) = 13, so n = [3/13,4/13,-12/13].
    let v = [3.0f32, 4.0, -12.0];
    let once = normalize_vec(&v);
    assert!(approx_eq(once[0], 3.0 / 13.0) && approx_eq(once[2], -12.0 / 13.0));
    // Idempotence: normalizing a normalized vector is a fixed point.
    let twice = normalize_vec(&once);
    for (x, y) in once.iter().zip(twice.iter()) {
        assert!(approx_eq(*x, *y), "not idempotent: {once:?} vs {twice:?}");
    }
    // Unit norm of the fixed point.
    let norm: f32 = once.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!(approx_eq(norm, 1.0), "norm={norm}");
    // Differential: in-place and out-of-place paths agree bit-exactly.
    let mut inplace = v.to_vec();
    normalize_vec_in_place(&mut inplace);
    assert_eq!(inplace, once);
    assert_eq!(normalize_vec(&normalize_vec(&[0.0, 0.0])), vec![0.0, 0.0]);
    assert_eq!(normalize_vec(&[f32::INFINITY, -3.0]), vec![0.0, -1.0]);
}

/// INTENT: the text embedder maps empty text to the exact zero vector and
/// real queries to deterministic finite unit-norm vectors, with the provider
/// method agreeing with the free function.
///
/// KILLS: empty-nonzero, nondeterminism, nonfinite components, non-unit-norm,
/// provider/free-function divergence.
///
/// ABSORBS: pass3::embed_text_deterministic_zero_for_empty.
#[test]
fn embed_text_is_deterministic_unit_zero_for_empty() {
    let embedder = SemanticLocalEmbedding;
    // Empty text yields no tokens and no trigrams: the exact zero vector.
    assert_eq!(embedder.embed_text(""), vec![0.0; SEMANTIC_DIM]);
    // Determinism and shape on a real query.
    let a = embedder.embed_text("credential renewal");
    assert_eq!(a.len(), SEMANTIC_DIM);
    assert_eq!(a, embedder.embed_text("credential renewal"));
    assert!(a.iter().all(|x| x.is_finite()));
    // Normalized embeddings have unit self-similarity (1e-5 band: 256 lanes).
    assert!((dot_similarity(&a, &a) - 1.0).abs() < 1e-5);
    // Differential: the provider method agrees with the free function.
    let b = embedder.embed_text("sanitize user input");
    assert_eq!(embedder.similarity(&a, &b), dot_similarity(&a, &b));
}

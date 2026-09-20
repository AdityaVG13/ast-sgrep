//! cosine_similarity contract suite: exactness + totality + metamorphic in one test.
//!
//! Absorbs the seven per-function cosine clauses from the N1/N2/N3 pass files.
//! Tolerance table: BIT-EXACT (`assert_eq!`) for SIMD-NaN/inf poison, scalar
//! skip pins, one-sided survivors, dyadic-exact ratios, scalar-lane symmetry;
//! 1e-6 (`approx_eq`) for extreme-magnitude ratios, SIMD-lane symmetry,
//! self-ceiling, and rescale invariance.

use ast_sgrep_embed::{cosine_similarity, dot_similarity, normalize_vec, top_k_flat_similarity};
use ast_sgrep_testkit::{approx_eq, lcg_vec};

/// INTENT: cosine_similarity total contract — SIMD lane poisons (no skip) while
/// scalar lane skips nonfinite pairs, f64-lane extreme-magnitude survival,
/// bit-exact scalar symmetry, approximate SIMD symmetry, self-maximality
/// ceiling, and positive-rescale score/ranking invariance.
/// KILLS: lane-divergence (SIMD-skip or scalar-poison), count-skipped-norm,
/// skip-guard-removal, f64-accumulation-removal, asymmetric-skip, SIMD
/// normalization-order, ceiling-violation (cross-beats-self),
/// magnitude-leak (non-invariance) mutants.
/// ABSORBS: cosine_simd_lane_does_not_skip_nonfinite (N1), cosine_skip_all_pairs_and_one_sided_survivor (N2),
/// cosine_extreme_magnitudes_avoid_dot_overflow_collapse (N2), cosine_symmetry_bit_exact_scalar_lane (N3),
/// cosine_symmetry_simd_lane_approx (N3), self_similarity_is_maximal (N3),
/// cosine_positive_rescale_preserves_scores_and_ranking (N3).
/// LINE-DROPS: self_maximal zero-vector arms (weaker vs N2 dot_zero/cosine_skip pins).
#[test]
fn cosine_contract() {
    // ── Clause cosine_simd_lane (N1): SIMD poisons, scalar skips ──
    // 64-dim takes the SIMD path: any NaN poisons dot/na/nb -> non-finite
    // -> 0.0. BIT-EXACT 0.0. (The scalar path would skip the pair instead;
    // this pins the deliberate lane divergence.)
    let mut a64 = vec![1.0f32; 64];
    a64[0] = f32::NAN;
    assert_eq!(cosine_similarity(&a64, &vec![1.0f32; 64]), 0.0);
    // 63-dim takes the scalar path: pair 0 is skipped ENTIRELY (neither
    // norm accumulates the survivor side), so 62 pairs of (1,1) give
    // dot=62, na=62, nb=62 -> 62/62 = 1.0. BIT-EXACT.
    let mut a63 = vec![1.0f32; 63];
    a63[0] = f32::NAN;
    assert_eq!(cosine_similarity(&a63, &vec![1.0f32; 63]), 1.0);
    // Non-trivial skip formula at boundary length: a=[NaN,3,4,0x60],
    // b=[5,4,3,0x60]. Pair 0 skipped; survivors give dot=12+12=24,
    // na=9+16=25, nb=16+9=25 -> 24/25 = 0.96 (1e-6; kills both the
    // NaN-poison mutant and the count-skipped-norm mutant, which would
    // give 24/sqrt(25*50) ~= 0.6788 instead).
    let mut a = vec![0.0f32; 63];
    let mut b = vec![0.0f32; 63];
    (a[0], a[1], a[2]) = (f32::NAN, 3.0, 4.0);
    (b[0], b[1], b[2]) = (5.0, 4.0, 3.0);
    let got = cosine_similarity(&a, &b);
    assert!(approx_eq(got, 0.96), "got {got}");
    // SIMD all-finite parallel vectors still normalize to 1.0:
    // dot=128, na=64, nb=256 -> 128/(8*16) = 1.0 (1e-6).
    let simd_one = cosine_similarity(&vec![1.0f32; 64], &vec![2.0f32; 64]);
    assert!(approx_eq(simd_one, 1.0), "got {simd_one}");

    // ── Clause cosine_skip (N2): all-skipped + one-sided survivors ──
    // Every pair skipped -> both norms are 0.0 -> honest 0.0. BIT-EXACT.
    assert_eq!(cosine_similarity(&[f32::NAN, f32::NAN], &[1.0, 2.0]), 0.0);
    assert_eq!(
        cosine_similarity(&[f32::INFINITY, f32::NEG_INFINITY], &[1.0, 2.0]),
        0.0
    );
    // One-sided survivors still score: pair 0 skipped, survivors give
    // dot=12, na=9, nb=16 -> 12/(3*4) = 1.0. BIT-EXACT.
    assert_eq!(cosine_similarity(&[f32::INFINITY, 3.0], &[4.0, 4.0]), 1.0);
    // -inf on the right skips the same way: dot=4, na=4, nb=4 -> 1.0.
    assert_eq!(
        cosine_similarity(&[1.0, 2.0], &[f32::NEG_INFINITY, 2.0]),
        1.0
    );
    // SIMD lane (64-dim): a single +inf poisons dot/na/nb -> 0.0.
    // BIT-EXACT. (Rides N1's pin that the SIMD path is taken here.)
    let mut ai64 = vec![1.0f32; 64];
    ai64[0] = f32::INFINITY;
    assert_eq!(cosine_similarity(&ai64, &vec![1.0f32; 64]), 0.0);

    // ── Clause cosine_extreme (N2): f64 lane survives dot collapse ──
    // The scalar cosine accumulates in f64, so magnitudes that collapse
    // dot_similarity (f32) to 0.0 still score ~1.0 here. 1e-6: the ratio
    // x/(sqrt(x)*sqrt(x)) is exact-class but not bit-promised.
    let got = cosine_similarity(&[1e30], &[1e30]);
    assert!(approx_eq(got, 1.0), "got {got}");
    assert_eq!(dot_similarity(&[1e30], &[1e30]), 0.0);
    let max = cosine_similarity(&[f32::MAX], &[f32::MAX]);
    assert!(approx_eq(max, 1.0), "got {max}");
    let anti = cosine_similarity(&[f32::MAX], &[-f32::MAX]);
    assert!(approx_eq(anti, -1.0), "got {anti}");
    let two = cosine_similarity(&[f32::MAX, f32::MAX], &[f32::MAX, f32::MAX]);
    assert!(approx_eq(two, 1.0), "got {two}");

    // ── Clause cos_sym_scalar (N3): bit-exact swap on scalar lane ──
    // The scalar fold skips non-finite pairs symmetrically and the f64
    // ratio is argument-order independent, so this is bit-identical.
    let mut seed = 0xC0FF_EE11_2345_6789;
    let mut pairs: Vec<(Vec<f32>, Vec<f32>)> = vec![
        (vec![3.0, 4.0], vec![1.0, 1.0]),
        (vec![f32::NAN, 3.0], vec![4.0, 4.0]),
        (vec![4.0, 4.0], vec![f32::NAN, 3.0]),
        (vec![f32::INFINITY, 3.0], vec![4.0, f32::NEG_INFINITY]),
        (vec![0.0, 0.0], vec![1.0, 2.0]),
        (vec![], vec![]),
        (vec![1.0], vec![1.0, 2.0]),
    ];
    for _ in 0..4 {
        pairs.push((lcg_vec(&mut seed, 9, 1.0), lcg_vec(&mut seed, 9, 1.0)));
    }
    pairs.push((lcg_vec(&mut seed, 63, 1.0), lcg_vec(&mut seed, 63, 1.0)));
    for (a, b) in &pairs {
        assert_eq!(cosine_similarity(a, b), cosine_similarity(b, a));
    }

    // ── Clause cos_sym_simd (N3): approx swap on SIMD lane ──
    // 64-dim takes the SIMD path; normalized ratios agree to 1e-6.
    let mut seed = 0x5EED_1234_ABCD_9876;
    for len in [64usize, 70, 128] {
        let a = lcg_vec(&mut seed, len, 1.0);
        let b = lcg_vec(&mut seed, len, 1.0);
        let ab = cosine_similarity(&a, &b);
        let ba = cosine_similarity(&b, &a);
        assert!(approx_eq(ab, ba), "len={len}: cosine {ab} vs {ba}");
    }

    // ── Clause self_maximal (N3): self is the ceiling ──
    // Cosine self-score is ~= 1.0 and no cross-score exceeds the 1.0 ceiling
    // (1e-6 rounding band); unit-vector dot self-scores behave the same, and
    // every finite dot self-score is non-negative.
    // LINE-DROP (catalog): the zero-vector arms (self/cross collapse to 0.0)
    // are deleted here as weaker vs the N2 dot_zero/cosine_skip pins.
    let mut seed = 0xA11C_E5E1_F00D_1234;
    let mut vecs: Vec<Vec<f32>> = vec![vec![3.0, 4.0], vec![1.0, 1.0], vec![-2.0, 0.5, 7.0]];
    for _ in 0..3 {
        vecs.push(lcg_vec(&mut seed, 8, 1.0));
    }
    vecs.push(lcg_vec(&mut seed, 64, 1.0));
    for a in &vecs {
        let me = cosine_similarity(a, a);
        assert!(approx_eq(me, 1.0), "self cosine {me} for {a:?}");
        for b in &vecs {
            let cross = cosine_similarity(a, b);
            assert!(cross <= 1.0 + 1e-6, "cross {cross} exceeds ceiling");
            assert!(cross <= me + 1e-6, "cross {cross} beats self {me}");
        }
        let u = normalize_vec(a);
        let udot = dot_similarity(&u, &u);
        assert!((udot - 1.0).abs() < 1e-6, "unit self-dot {udot}");
        for b in &vecs {
            let v = normalize_vec(b);
            assert!(dot_similarity(&u, &v) <= 1.0 + 1e-6);
        }
        assert!(dot_similarity(a, a) >= 0.0);
    }

    // ── Clause cosine_rescale (N3): positive rescale invariant ──
    // Cosine divides out magnitudes: per-row positive rescale leaves every
    // score ~= unchanged (1e-6) and the ranking order bit-identical.
    let query = vec![1.0, 0.2];
    let rows: Vec<Vec<f32>> = vec![
        vec![1.0, 0.0],
        vec![3.0, 4.0],
        vec![0.0, 1.0],
        vec![-1.0, 0.5],
        vec![1.0, 1.0],
    ];
    let factors = [0.25, 1.5, 3.0, 0.1, 7.0];
    let flat: Vec<f32> = rows.iter().flatten().copied().collect();
    let scaled_flat: Vec<f32> = rows
        .iter()
        .zip(factors)
        .flat_map(|(r, k)| r.iter().map(move |x| x * k))
        .collect();
    for (r, k) in rows.iter().zip(factors) {
        let plain = cosine_similarity(&query, r);
        let scaled_row: Vec<f32> = r.iter().map(|x| x * k).collect();
        assert!(approx_eq(plain, cosine_similarity(&query, &scaled_row)));
    }
    // Query-side rescale is invariant too.
    let big_q: Vec<f32> = query.iter().map(|x| x * 4.0).collect();
    for r in &rows {
        assert!(approx_eq(
            cosine_similarity(&query, r),
            cosine_similarity(&big_q, r)
        ));
    }
    // Ranking order is bit-identical under row rescale (scores are ~= but
    // well separated, so no adjacent pair can flip).
    let base = top_k_flat_similarity(&query, &flat, 2, 5, None);
    let scaled = top_k_flat_similarity(&query, &scaled_flat, 2, 5, None);
    assert_eq!(
        base.iter().map(|(i, _)| i).collect::<Vec<_>>(),
        scaled.iter().map(|(i, _)| i).collect::<Vec<_>>()
    );
    for (a, b) in base.iter().zip(scaled.iter()) {
        assert!(approx_eq(a.1, b.1), "scores {a:?} vs {b:?}");
    }
}

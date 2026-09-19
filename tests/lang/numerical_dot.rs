//! dot_similarity contract suite: exactness + totality + metamorphic in one test.
//!
//! Absorbs the six per-function dot clauses from the N1/N2/N3 pass files.
//! Tolerance table: BIT-EXACT (`assert_eq!` / `to_bits`) for overflow collapse,
//! dyadic sums, zero/subnormal lanes, signed-zero payloads, MAX split,
//! cancellation, scalar-lane symmetry, rescale order; 1e-5 inline for the
//! 64/70-dim SIMD-lane swap agreement (f32 accumulation band).

use ast_sgrep_embed::dot_similarity;
use ast_sgrep_testkit::{fold_rank_scored, lcg_vec};

/// INTENT: dot_similarity total contract — overflow collapse, dyadic exactness,
/// zero/subnormal/signed-zero lanes, extreme-magnitude split, exact
/// cancellation, bit-exact scalar symmetry, approximate SIMD symmetry, and
/// positive-preserve / negative-reverse rescale order.
/// KILLS: overflow-collapse-guard-removal, zero/subnormal-collapse,
/// sign-payload, overflow-boundary, cancellation-collapse, argument-order
/// (non-commutative-fold scalar + SIMD), rescale-sign/order mutants.
/// ABSORBS: dot_overflow_and_dyadic_lanes (N1), dot_zero_subnormal_and_signed_zero_lanes_are_exact (N2),
/// dot_extreme_magnitudes_split_finite_from_overflow (N2), dot_symmetry_bit_exact_scalar_lane (N3),
/// dot_symmetry_simd_lane_approx (N3), dot_positive_rescale_preserves_order_negative_reverses (N3).
/// LINE-DROPS: none (the dot-control line in cosine_extreme stays as a cross-ref there, not duplicated here).
#[test]
fn dot_contract() {
    // ── Clause dot_overflow (N1): overflow collapses, dyadic exact ──
    // Scalar lane (len 2): 1e38*1e38 = inf per product -> sum inf -> 0.0.
    assert_eq!(dot_similarity(&[1e38, 1e38], &[1e38, 1e38]), 0.0);
    // SIMD-width lane (len 64): true sum 6.4e61 unrepresentable in f32
    // under either accumulation (f32 inf, or f64 6.4e61 -> `as f32` inf),
    // so both the simd and the scalar-fallback arm collapse to 0.0.
    assert_eq!(
        dot_similarity(&vec![1e30f32; 64], &vec![1e30f32; 64]),
        0.0
    );
    // Dyadic-exact: 0.25+0.25+0.25+0.25 = 1.0, no rounding. BIT-EXACT.
    assert_eq!(
        dot_similarity(&[0.5, 0.5, 0.5, 0.5], &[0.5, 0.5, 0.5, 0.5]),
        1.0
    );

    // ── Clause dot_zero_subnormal (N2): zero/subnormal/signed-zero ──
    // Zero vectors are honest zeros, not NaN. BIT-EXACT.
    assert_eq!(dot_similarity(&[0.0, 0.0], &[0.0, 0.0]), 0.0);
    assert_eq!(dot_similarity(&[0.0, 0.0], &[1.0, 2.0]), 0.0);
    // MIN_POSITIVE^2 underflows to 0.0 (finite collapse). BIT-EXACT.
    assert_eq!(
        dot_similarity(&[f32::MIN_POSITIVE], &[f32::MIN_POSITIVE]),
        0.0
    );
    // MIN_POSITIVE * 1.0 is exactly representable. BIT-EXACT.
    assert_eq!(dot_similarity(&[f32::MIN_POSITIVE], &[1.0]), f32::MIN_POSITIVE);
    // Signed-zero payloads survive the finite gate; pin the BITS.
    assert_eq!(
        dot_similarity(&[-0.0], &[1.0]).to_bits(),
        (-0.0f32).to_bits()
    );
    assert_eq!(dot_similarity(&[-0.0], &[-0.0]).to_bits(), 0.0f32.to_bits());

    // ── Clause dot_extreme (N2): MAX split + cancellation ──
    // MAX * 1.0 is finite and exact; MAX * 2.0 overflows to inf -> 0.0.
    // BIT-EXACT both arms (documents the f32-accumulation boundary).
    assert_eq!(dot_similarity(&[f32::MAX], &[1.0]), f32::MAX);
    assert_eq!(dot_similarity(&[f32::MAX], &[2.0]), 0.0);
    assert_eq!(dot_similarity(&[f32::MAX], &[f32::MAX]), 0.0);
    // Exact cancellation at extreme magnitude: 1e30 + -1e30 = 0.0.
    // BIT-EXACT (kills the nonfinite-collapse mutant: this 0.0 is a real
    // sum, not an overflow guard).
    assert_eq!(dot_similarity(&[1e30, -1e30], &[1.0, 1.0]), 0.0);
    assert_eq!(dot_similarity(&[f32::MAX, f32::MAX], &[1.0, -1.0]), 0.0);

    // ── Clause dot_sym_scalar (N3): bit-exact swap on scalar lane ──
    // f32 mul is commutative and the scalar fold order is fixed, so swapping
    // arguments is bit-identical — including hostile lanes (all collapse to
    // the same 0.0 on both sides).
    let mut seed = 0x1234_5678_9ABC_DEF0;
    let mut pairs: Vec<(Vec<f32>, Vec<f32>)> = vec![
        (vec![3.0, -4.0, 0.5], vec![1.0, 2.0, -7.0]),
        (vec![0.0, -0.0], vec![-0.0, 0.0]),
        (vec![f32::MIN_POSITIVE, 1.0], vec![1.0, f32::MIN_POSITIVE]),
        (vec![1e30, -1e30], vec![1.0, 1.0]),
        (vec![f32::INFINITY, 1.0], vec![1.0, 2.0]),
        (vec![f32::NAN], vec![1.0]),
        (vec![], vec![]),
        (vec![1.0], vec![1.0, 2.0]),
    ];
    for _ in 0..4 {
        pairs.push((lcg_vec(&mut seed, 7, 1.0), lcg_vec(&mut seed, 7, 1.0)));
    }
    pairs.push((lcg_vec(&mut seed, 63, 1.0), lcg_vec(&mut seed, 63, 1.0)));
    for (a, b) in &pairs {
        assert_eq!(dot_similarity(a, b), dot_similarity(b, a));
    }

    // ── Clause dot_sym_simd (N3): approx swap on SIMD lane ──
    // 64-dim takes the SIMD path; accumulation order may differ from scalar,
    // but swapping arguments must agree to 1e-5 (vectors scaled so |dot|<=1).
    let mut seed = 0x0BAD_F00D_DEAD_BEEF;
    for _ in 0..4 {
        let a = lcg_vec(&mut seed, 64, 0.125);
        let b = lcg_vec(&mut seed, 64, 0.125);
        let ab = dot_similarity(&a, &b);
        let ba = dot_similarity(&b, &a);
        assert!((ab - ba).abs() < 1e-5, "dot {ab} vs {ba}");
    }
    let a = lcg_vec(&mut seed, 70, 0.125);
    let b = lcg_vec(&mut seed, 70, 0.125);
    assert!((dot_similarity(&a, &b) - dot_similarity(&b, &a)).abs() < 1e-5);

    // ── Clause dot_rescale (N3): positive preserves, negative reverses ──
    // Well-separated dyadic dots: exact in f32, so scaling the query by k>0
    // keeps the order and scaling by k<0 reverses it — bit-exact.
    let aq = vec![1.0, 2.0];
    let corpus = vec![
        vec![3.0, 4.0],  // 11
        vec![1.0, 1.0],  // 3
        vec![0.0, 0.0],  // 0
        vec![-1.0, 0.0], // -1
    ];
    let order_of = |q: &[f32]| {
        let scored: Vec<(usize, f32)> = corpus
            .iter()
            .enumerate()
            .map(|(i, b)| (i, dot_similarity(q, b)))
            .collect();
        fold_rank_scored(&scored)
    };
    let idx_order = |q: &[f32]| order_of(q).iter().map(|(i, _)| *i).collect::<Vec<_>>();
    let base = idx_order(&aq);
    assert_eq!(base, vec![0, 1, 2, 3]);
    let pos: Vec<f32> = aq.iter().map(|x| x * 2.5).collect();
    assert_eq!(idx_order(&pos), base);
    let neg: Vec<f32> = aq.iter().map(|x| x * -2.0).collect();
    let mut reversed = base.clone();
    reversed.reverse();
    assert_eq!(idx_order(&neg), reversed);
}

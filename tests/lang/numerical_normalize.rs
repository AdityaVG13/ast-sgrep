//! normalize_vec contract suite: exactness + totality + metamorphic in one test.
//!
//! Absorbs the four per-function normalize clauses from the N1/N2/N3 pass files.
//! Tolerance table: BIT-EXACT (`assert_eq!` / `to_bits`) for overflow fill,
//! subnormal fill, dyadic units, in-place lanes, signed-zero fill bits, API
//! parity; 1e-6 inline for second-pass drift on general vectors.

use ast_sgrep_embed::{normalize_vec, normalize_vec_in_place};
use ast_sgrep_testkit::lcg_vec;

/// INTENT: normalize_vec total contract — overflow/subnormal zero-fill,
/// dyadic-exact units, in-place totality with +0.0 fill bits, approximate
/// idempotence on general vectors, and bit-exact in-place/out-of-place parity.
/// KILLS: finite-guard/zero-guard-removal, in-place-divergence, zero-fill-sign,
/// second-pass-divergence, API-divergence mutants.
/// ABSORBS: normalize_overflow_subnormal_and_dyadic_pins (N1),
/// normalize_in_place_totality_and_signed_zero_bits (N2), normalize_idempotent_twice_equals_once (N3),
/// normalize_in_place_agrees_with_out_of_place (N3).
/// LINE-DROPS: idempotent exact-lane arms (weaker vs N1/N2 normalize pins).
#[test]
fn normalize_contract() {
    // ── Clause normalize_overflow (N1): overflow/subnormal/dyadic ──
    // [1e38,1e38]: squares overflow f32 -> norm=inf -> not finite -> zeros.
    // Kills the finite-guard removal (inf/inf = NaN). BIT-EXACT.
    assert_eq!(normalize_vec(&[1e38, 1e38]), vec![0.0, 0.0]);
    assert_eq!(normalize_vec(&[f32::MAX, f32::MAX]), vec![0.0, 0.0]);
    // [MIN_POSITIVE]: square (1.4e-45)^2 underflows to 0.0 -> norm=0 ->
    // zeros. Kills the zero-guard removal (0/0 = NaN). BIT-EXACT.
    assert_eq!(normalize_vec(&[f32::MIN_POSITIVE]), vec![0.0]);
    // Dyadic-exact: norm divides evenly, no rounding. BIT-EXACT.
    assert_eq!(normalize_vec(&[5.0]), vec![1.0]);
    assert_eq!(normalize_vec(&[-2.0, 0.0]), vec![-1.0, 0.0]);

    // ── Clause normalize_inplace (N2): in-place totality + fill bits ──
    // Empty in place: no panic, stays empty.
    let mut empty: Vec<f32> = vec![];
    normalize_vec_in_place(&mut empty);
    assert!(empty.is_empty());
    // Overflow in place: norm=inf -> fill zeros (matches out-of-place).
    let mut huge = vec![1e38, 1e38];
    normalize_vec_in_place(&mut huge);
    assert_eq!(huge, vec![0.0, 0.0]);
    // Non-finite zeroing in place: [inf,-3] -> [0,-1]. BIT-EXACT.
    let mut mixed = vec![f32::INFINITY, -3.0];
    normalize_vec_in_place(&mut mixed);
    assert_eq!(mixed, vec![0.0, -1.0]);
    // Zero-norm fill writes +0.0 even for -0.0 inputs; pin the BITS
    // (assert_eq alone cannot see the sign).
    let filled = normalize_vec(&[0.0, -0.0]);
    assert_eq!(filled.len(), 2);
    assert_eq!(filled[0].to_bits(), 0.0f32.to_bits());
    assert_eq!(filled[1].to_bits(), 0.0f32.to_bits());
    assert_eq!(normalize_vec(&[-0.0])[0].to_bits(), 0.0f32.to_bits());

    // ── Clause normalize_idempotent (N3): twice ~= once ──
    // General vectors: the second pass divides by ~= 1.0, so per-element
    // drift stays under 1e-6.
    // LINE-DROP (catalog): the exact-lane arms (dyadic/zero/nonfinite/
    // overflow/empty idempotence) are deleted here as weaker vs the
    // N1/N2 normalize pins above.
    let mut seed = 0x1DE4_907E_5EED_00CB;
    let general: Vec<Vec<f32>> = vec![
        vec![3.0, 4.0, 0.0],
        vec![0.1, -0.2, 0.3, -0.4],
        lcg_vec(&mut seed, 16, 1.0),
        lcg_vec(&mut seed, 70, 1.0),
    ];
    for v in &general {
        let once = normalize_vec(v);
        let twice = normalize_vec(&once);
        assert_eq!(once.len(), twice.len());
        for (x, y) in once.iter().zip(twice.iter()) {
            assert!((x - y).abs() < 1e-6, "drift {x} vs {y} for {v:?}");
        }
    }

    // ── Clause normalize_parity (N3): in-place == out-of-place ──
    // The two APIs share one code path; parity must be bit-exact on general
    // and hostile inputs alike (catches any future divergence).
    let mut seed = 0x9E37_79B9_7F4A_7C15;
    let mut vecs: Vec<Vec<f32>> = vec![
        vec![3.0, -4.0, 0.0],
        vec![f32::INFINITY, -3.0, f32::NAN],
        vec![0.0, -0.0],
        vec![1e38, 1e38],
        Vec::new(),
    ];
    vecs.push(lcg_vec(&mut seed, 70, 1.0));
    for v in &vecs {
        let mut inplace = v.clone();
        normalize_vec_in_place(&mut inplace);
        assert_eq!(inplace, normalize_vec(v));
    }
}

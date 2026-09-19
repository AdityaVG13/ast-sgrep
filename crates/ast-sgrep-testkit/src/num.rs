//! Scalar numeric helpers shared by the scoring-similarity suites.
//!
//! # Contract
//!
//! - [`approx_eq`] is a pure function: absolute epsilon only, no relative
//!   component, no panics. `NaN` inputs compare `false` (never equal).
//! - Tolerance bands are suite-pinned, not testkit-pinned: bit-exact
//!   (`assert_eq!` / `to_bits`) for orderings/indices/strings/guard outcomes,
//!   1e-6 here for cosine/hand-formula scores, 1e-5 inline for 256-lane
//!   embedding self-dots. This module owns only the shared 1e-6 band.

/// Absolute-epsilon float equality: `(a - b).abs() < 1e-6`.
///
/// Units: absolute `f32` epsilon (1e-6), shared by the lang numerical and
/// oracle suites for cosine/hand-formula score pins. Pure; `NaN` yields false.
pub fn approx_eq(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-6
}

/// INTENT: independent f64-fold oracle for excerpt ranking — a deliberately
/// different accumulation from the pipeline's `dot_similarity`, with the hand
/// rule (score desc, ties by ascending index) applied explicitly over
/// caller-supplied `(index, score)` pairs. Pure; `NaN` scores compare equal
/// and fall back to index order.
pub fn fold_rank_scored(order: &[(usize, f32)]) -> Vec<(usize, f32)> {
    let mut ranked = order.to_vec();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    ranked
}

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
//! - Deterministic fixtures ([`lcg_vec`], [`chunk_row`]) and the rank-order
//!   projection ([`ranked_indices`]) are fixed-value constructors shared by
//!   the lang numerical suites; every vector is reproducible from its seed
//!   with no prod RNG dependency.

use ast_sgrep_embed::SemanticChunkRow;

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

/// INTENT: deterministic LCG vector fixture — `len` lanes uniform in
/// `[-scale, scale]` from a caller-owned seed (Knuth MMIX constants,
/// upper-33-bits projection). Fixed seeds make every vector reproducible
/// without a prod RNG dependency. Pure constructor (seed advances).
pub fn lcg_vec(seed: &mut u64, len: usize, scale: f32) -> Vec<f32> {
    (0..len)
        .map(|_| {
            *seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((*seed >> 33) as f32 / u32::MAX as f32 * 2.0 - 1.0) * scale
        })
        .collect()
}

/// INTENT: rank-order projection — ranked `(index, score)` pairs down to
/// index order for exact-order asserts. Pure projection.
pub fn ranked_indices(ranked: &[(usize, f32)]) -> Vec<usize> {
    ranked.iter().map(|(i, _)| *i).collect()
}

/// INTENT: chunk-row fixture — the 6-tuple [`SemanticChunkRow`] the
/// chunk ranker consumes, with fixed non-embedding fields (`f.rs`,
/// span 0..1, `sym`/`ex`) so suites vary only the embedding under test.
/// Pure constructor.
pub fn chunk_row(emb: Vec<f32>) -> SemanticChunkRow {
    ("f.rs".to_string(), 0, 1, "sym".to_string(), "ex".to_string(), emb)
}

//! Similarity ranking primitives.
//!
//! # Finite-score invariants (`ast-sgrep-0pla`)
//!
//! - [`Scored`] only admits finite `f32` similarities via [`Scored::new`].
//! - Because NaN/±∞ never enter `Scored`, derived [`PartialEq`] and blank [`Eq`]
//!   agree with [`Ord`]: [`PartialEq`] never sees NaN≠NaN, and [`Ord`] never
//!   needs a NaN==NaN special case.
//! - Public rankers ([`top_k_similarity`], [`top_k_flat_similarity`],
//!   [`top_by_similarity`]) drop non-finite input scores before heap/sort.
//! - Cosine / L2 helpers treat non-finite residuals as zero contribution so
//!   ANN normalization cannot poison downstream ranking with NaN.

use rayon::prelude::*;
use simsimd::SpatialSimilarity;
use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
pub const MIN_SIMILARITY: f32 = 0.08;
pub const PARALLEL_CHUNK_THRESHOLD: usize = 64;
fn exceeds_threshold(sim: f32, min: f32) -> bool {
    if !sim.is_finite() || !min.is_finite() {
        return false;
    }
    let next = if min == 0.0 {
        f32::from_bits(1)
    } else if min > 0.0 {
        f32::from_bits(min.to_bits() + 1)
    } else {
        f32::from_bits(min.to_bits() - 1)
    };
    sim > next
}
const SIMD_DOT_THRESHOLD: usize = 64;

/// Heap entry for top-k ranking. Similarity is finite by construction.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Scored {
    idx: usize,
    sim: f32,
}

impl Scored {
    /// Reject non-finite similarities so [`Eq`]/[`Ord`] stay coherent.
    fn new(idx: usize, sim: f32) -> Option<Self> {
        sim.is_finite().then_some(Self { idx, sim })
    }
}

impl Eq for Scored {}

impl PartialOrd for Scored {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Scored {
    fn cmp(&self, other: &Self) -> Ordering {
        // Both sims are finite (Scored::new), so partial_cmp always returns Some.
        self.sim
            .partial_cmp(&other.sim)
            .unwrap_or(Ordering::Equal)
            .then_with(|| other.idx.cmp(&self.idx))
    }
}

fn score_order(left: f32, right: f32) -> Ordering {
    // Public sort paths only retain finite scores; treat any residual non-finite
    // as less than every finite value (and equal to each other) without claiming
    // NaN==NaN for Eq on Scored.
    match (left.is_finite(), right.is_finite()) {
        (true, true) => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
        (false, false) => Ordering::Equal,
        (false, true) => Ordering::Less,
        (true, false) => Ordering::Greater,
    }
}

fn cmp_hits_desc(left: &(usize, f32), right: &(usize, f32)) -> Ordering {
    score_order(right.1, left.1).then_with(|| left.0.cmp(&right.0))
}

pub fn dot_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    if a.len() >= SIMD_DOT_THRESHOLD {
        if let Some(d) = f32::dot(a, b) {
            let d = d as f32;
            return if d.is_finite() { d } else { 0.0 };
        }
    }
    let sum: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    if sum.is_finite() {
        sum
    } else {
        0.0
    }
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    if a.len() >= SIMD_DOT_THRESHOLD {
        if let (Some(dot), Some(na), Some(nb)) = (f32::dot(a, b), f32::dot(a, a), f32::dot(b, b)) {
            if !dot.is_finite() || !na.is_finite() || !nb.is_finite() || na <= 0.0 || nb <= 0.0 {
                return 0.0;
            }
            let score = (dot / (na.sqrt() * nb.sqrt())) as f32;
            if score.is_finite() {
                return score;
            }
            return 0.0;
        }
    }
    let (dot, na, nb) =
        a.iter()
            .zip(b)
            .fold((0.0_f64, 0.0_f64, 0.0_f64), |(dot, na, nb), (&l, &r)| {
                if !l.is_finite() || !r.is_finite() {
                    return (dot, na, nb);
                }
                let l = f64::from(l);
                let r = f64::from(r);
                (l.mul_add(r, dot), l.mul_add(l, na), r.mul_add(r, nb))
            });
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    let score = (dot / (na.sqrt() * nb.sqrt())) as f32;
    if score.is_finite() {
        score
    } else {
        0.0
    }
}

pub fn top_k_similarity(
    scored: impl IntoIterator<Item = (usize, f32)>,
    limit: usize,
    min_similarity: Option<f32>,
) -> Vec<(usize, f32)> {
    if limit == 0 {
        return vec![];
    }
    let mut heap = BinaryHeap::new();
    for (idx, sim) in scored {
        if min_similarity.is_none_or(|min| exceeds_threshold(sim, min)) {
            push_top_k(&mut heap, limit, idx, sim);
        }
    }
    heap_to_sorted(heap)
}

pub fn top_k_flat_similarity(
    query_vec: &[f32],
    flat: &[f32],
    dim: usize,
    limit: usize,
    min_similarity: Option<f32>,
) -> Vec<(usize, f32)> {
    // Guard dim/limit before any `/ dim` — bare integer division panics on dim=0
    // (empty/corrupt embedding corpora must return no hits, not abort).
    let n = flat.len().checked_div(dim).unwrap_or(0);
    if limit == 0 || n == 0 || dim == 0 || query_vec.len() != dim {
        return vec![];
    }
    let push_sim = |heap: &mut BinaryHeap<Reverse<Scored>>, i: usize| {
        let sim = cosine_similarity(query_vec, &flat[i * dim..(i + 1) * dim]);
        if min_similarity.is_none_or(|min| exceeds_threshold(sim, min)) {
            push_top_k(heap, limit, i, sim);
        }
    };
    if n < PARALLEL_CHUNK_THRESHOLD {
        let mut heap = BinaryHeap::new();
        for i in 0..n {
            push_sim(&mut heap, i);
        }
        return heap_to_sorted(heap);
    }
    let heap = (0..n)
        .into_par_iter()
        .fold(BinaryHeap::new, |mut heap, i| {
            push_sim(&mut heap, i);
            heap
        })
        .reduce(BinaryHeap::new, |mut left, right| {
            for Reverse(s) in right {
                push_top_k(&mut left, limit, s.idx, s.sim);
            }
            left
        });
    heap_to_sorted(heap)
}

fn push_top_k(heap: &mut BinaryHeap<Reverse<Scored>>, limit: usize, idx: usize, sim: f32) {
    let Some(scored) = Scored::new(idx, sim) else {
        return;
    };
    heap.push(Reverse(scored));
    if heap.len() > limit {
        heap.pop();
    }
}

fn heap_to_sorted(heap: BinaryHeap<Reverse<Scored>>) -> Vec<(usize, f32)> {
    let mut out: Vec<(usize, f32)> = heap.into_iter().map(|Reverse(s)| (s.idx, s.sim)).collect();
    out.sort_by(cmp_hits_desc);
    out
}

pub fn top_by_similarity(
    mut scored: Vec<(usize, f32)>,
    limit: usize,
    min_similarity: Option<f32>,
) -> Vec<(usize, f32)> {
    if limit == 0 {
        return vec![];
    }
    scored.retain(|(_, sim)| {
        sim.is_finite() && min_similarity.is_none_or(|min| exceeds_threshold(*sim, min))
    });
    scored.sort_by(cmp_hits_desc);
    scored.truncate(limit);
    scored
}

/// L2-normalize in place. Non-finite components are zeroed first so a NaN
/// residual cannot poison the whole vector (ANN/IVF path).
pub fn normalize_vec_in_place(vec: &mut [f32]) {
    for x in vec.iter_mut() {
        if !x.is_finite() {
            *x = 0.0;
        }
    }
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm.is_finite() && norm > 0.0 {
        for x in vec {
            *x /= norm;
        }
    } else {
        vec.fill(0.0);
    }
}

pub fn normalize_vec(vec: &[f32]) -> Vec<f32> {
    let mut out = vec.to_vec();
    normalize_vec_in_place(&mut out);
    out
}

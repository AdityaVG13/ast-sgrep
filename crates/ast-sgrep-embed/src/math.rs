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

#[cfg(test)]
mod contract_tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn cosine_similarity_is_scale_invariant() {
        assert!(
            (cosine_similarity(&[1.0, 2.0], &[3.0, 4.0])
                - cosine_similarity(&[10.0, 20.0], &[1.5, 2.0]))
            .abs()
                <= f32::EPSILON
        );
    }

    #[test]
    fn similarity_rankers_filter_non_finite_scores() {
        assert_eq!(
            top_k_similarity([(0, f32::NAN), (1, 0.5)], 2, None),
            vec![(1, 0.5)]
        );
        // NaN components in flat rows are ignored; residual may be a finite 0.0
        // score which is dropped by the minimum-similarity gate.
        assert_eq!(
            top_k_flat_similarity(
                &[1.0, 0.0],
                &[f32::NAN, 0.0, 0.5, 0.0],
                2,
                2,
                Some(MIN_SIMILARITY)
            ),
            vec![(1, 1.0)]
        );
        assert_eq!(
            top_by_similarity(vec![(0, f32::NAN), (1, f32::INFINITY), (2, 0.4)], 3, None),
            vec![(2, 0.4)]
        );
    }

    #[test]
    fn scored_constructor_rejects_non_finite() {
        assert!(Scored::new(0, 0.5).is_some());
        assert!(Scored::new(0, f32::NAN).is_none());
        assert!(Scored::new(0, f32::INFINITY).is_none());
        assert!(Scored::new(0, f32::NEG_INFINITY).is_none());
    }

    #[test]
    fn scored_eq_ord_agree_on_finite_domain() {
        let a = Scored::new(1, 0.2).unwrap();
        let b = Scored::new(2, 0.2).unwrap();
        let c = Scored::new(0, 0.9).unwrap();
        assert_eq!(a.cmp(&b), Ordering::Greater); // higher idx loses ties → Reverse heap
        assert_eq!((a == b), (a.cmp(&b) == Ordering::Equal));
        assert_eq!((a == c), (a.cmp(&c) == Ordering::Equal));
        // Total order: no NaN equality loophole
        let mut set = BTreeSet::new();
        set.insert(a);
        set.insert(b);
        set.insert(c);
        assert_eq!(set.len(), 3);
    }

    #[test]
    fn normalize_vec_canonicalizes_nan_residuals() {
        let out = normalize_vec(&[1.0, f32::NAN, 0.0]);
        assert!(out.iter().all(|x| x.is_finite()));
        let norm: f32 = out.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5 || norm == 0.0);
        let all_nan = normalize_vec(&[f32::NAN, f32::NAN]);
        assert_eq!(all_nan, vec![0.0, 0.0]);
    }

    #[test]
    fn cosine_ignores_nan_components() {
        let score = cosine_similarity(&[1.0, f32::NAN], &[1.0, 0.0]);
        assert!(score.is_finite());
        assert!((score - 1.0).abs() < 1e-5);
    }

    #[test]
    fn minimum_similarity_uses_stable_ulp_boundary() {
        let min = 0.5_f32;
        let one = f32::from_bits(min.to_bits() + 1);
        let two = f32::from_bits(min.to_bits() + 2);
        assert!(top_k_similarity([(0, one)], 1, Some(min)).is_empty());
        assert_eq!(top_k_similarity([(0, two)], 1, Some(min)), vec![(0, two)]);
        assert!(top_by_similarity(vec![(0, one)], 1, Some(min)).is_empty());
        assert_eq!(
            top_by_similarity(vec![(0, two)], 1, Some(min)),
            vec![(0, two)]
        );
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;

    #[test]
    fn scored_heap_never_admits_nan_across_seeded_inputs() {
        // Lightweight property micro-harness (g799) without pulling proptest into
        // the default lib build graph for embed.
        let seeds: &[f32] = &[
            0.0,
            -0.0,
            1.0,
            -1.0,
            f32::MIN_POSITIVE,
            f32::MAX,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            0.08,
            0.0799999,
        ];
        for (i, &sim) in seeds.iter().enumerate() {
            let out = top_k_similarity([(i, sim), (i + 100, 0.5)], 2, None);
            assert!(out.iter().all(|(_, s)| s.is_finite()));
            assert!(!out.iter().any(|(idx, _)| *idx == i) || sim.is_finite());
            let scored = Scored::new(i, sim);
            assert_eq!(scored.is_some(), sim.is_finite());
        }
        let mixed: Vec<_> = seeds.iter().enumerate().map(|(i, s)| (i, *s)).collect();
        let ranked = top_by_similarity(mixed, 8, None);
        assert!(ranked.iter().all(|(_, s)| s.is_finite()));
        for window in ranked.windows(2) {
            let ord = score_order(window[0].1, window[1].1);
            assert!(
                matches!(ord, Ordering::Greater | Ordering::Equal),
                "expected non-ascending scores, got {:?} then {:?}",
                window[0].1,
                window[1].1
            );
        }
    }

    #[test]
    fn normalize_then_rank_rejects_nan_query_residuals() {
        let q = normalize_vec(&[f32::NAN, 1.0, f32::INFINITY]);
        assert!(q.iter().all(|x| x.is_finite()));
        let flat = {
            let mut v = vec![1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0];
            normalize_vec_in_place(&mut v[0..3]);
            normalize_vec_in_place(&mut v[3..6]);
            v
        };
        let hits = top_k_flat_similarity(&q, &flat, 3, 2, Some(MIN_SIMILARITY));
        assert!(hits.iter().all(|(_, s)| s.is_finite()));
    }

    /// Product edge paths: empty corpus, zero dim, limit 0 / max, dim mismatch.
    /// Must return empty — never panic (div-by-zero on dim=0 was a real crash).
    #[test]
    fn top_k_flat_edge_paths_return_empty_without_panic() {
        let row = [1.0f32, 0.0, 0.0];
        let flat = {
            let mut v = vec![1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0];
            normalize_vec_in_place(&mut v[0..3]);
            normalize_vec_in_place(&mut v[3..6]);
            v
        };
        // empty corpus
        assert!(top_k_flat_similarity(&row, &[], 3, 5, Some(MIN_SIMILARITY)).is_empty());
        // zero dim (empty and non-empty flat) — must not divide-by-zero
        assert!(top_k_flat_similarity(&[], &[], 0, 5, None).is_empty());
        assert!(top_k_flat_similarity(&[], &[1.0, 2.0], 0, 5, None).is_empty());
        // limit 0
        assert!(top_k_flat_similarity(&row, &flat, 3, 0, Some(MIN_SIMILARITY)).is_empty());
        // query dim mismatch
        assert!(top_k_flat_similarity(&[1.0, 0.0], &flat, 3, 5, None).is_empty());
        // max limit: still ranks without OOM on tiny corpus
        let hits = top_k_flat_similarity(&row, &flat, 3, usize::MAX, None);
        assert_eq!(hits.len(), 2);
        assert!(hits[0].1 >= hits[1].1);
    }
}

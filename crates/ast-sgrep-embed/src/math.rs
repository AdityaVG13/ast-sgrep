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
#[derive(Clone, Copy, PartialEq)]
struct Scored {
    idx: usize,
    sim: f32,
}
impl Eq for Scored {}
impl PartialOrd for Scored {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Scored {
    fn cmp(&self, other: &Self) -> Ordering {
        score_order(self.sim, other.sim).then_with(|| other.idx.cmp(&self.idx))
    }
}
fn score_order(left: f32, right: f32) -> Ordering {
    match (left.is_nan(), right.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
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
            return d as f32;
        }
    }
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (dot, na, nb) =
        a.iter()
            .zip(b)
            .fold((0.0_f64, 0.0_f64, 0.0_f64), |(dot, na, nb), (&l, &r)| {
                let l = f64::from(l);
                let r = f64::from(r);
                (l.mul_add(r, dot), l.mul_add(l, na), r.mul_add(r, nb))
            });
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    (dot / (na.sqrt() * nb.sqrt())) as f32
}
pub fn cosine_scores_for<'a>(
    query_vec: &[f32],
    rows: impl Iterator<Item = (usize, &'a [f32])>,
) -> Vec<(usize, f32)> {
    rows.filter(|(_, emb)| emb.len() == query_vec.len())
        .map(|(idx, emb)| (idx, cosine_similarity(query_vec, emb)))
        .collect()
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
        if sim.is_finite() && min_similarity.is_none_or(|min| exceeds_threshold(sim, min)) {
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
    let n = flat.len() / dim;
    if limit == 0 || n == 0 || dim == 0 || query_vec.len() != dim {
        return vec![];
    }
    if n < PARALLEL_CHUNK_THRESHOLD {
        let mut heap = BinaryHeap::new();
        for i in 0..n {
            let sim = cosine_similarity(query_vec, &flat[i * dim..(i + 1) * dim]);
            if sim.is_finite() && min_similarity.is_none_or(|min| exceeds_threshold(sim, min)) {
                push_top_k(&mut heap, limit, i, sim);
            }
        }
        return heap_to_sorted(heap);
    }
    let heap = (0..n)
        .into_par_iter()
        .fold(BinaryHeap::new, |mut heap, i| {
            let sim = cosine_similarity(query_vec, &flat[i * dim..(i + 1) * dim]);
            if sim.is_finite() && min_similarity.is_none_or(|min| exceeds_threshold(sim, min)) {
                push_top_k(&mut heap, limit, i, sim);
            }
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
    heap.push(Reverse(Scored { idx, sim }));
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
    if let Some(min) = min_similarity {
        scored.retain(|(_, sim)| *sim > min);
    }
    scored.sort_by(cmp_hits_desc);
    scored.truncate(limit);
    scored
}
#[cfg(test)]
mod contract_tests {
    use super::*;
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
        assert_eq!(
            top_k_flat_similarity(&[1.0, 0.0], &[f32::NAN, 0.0, 0.5, 0.0], 2, 2, None),
            vec![(1, 1.0)]
        );
    }
    #[test]
    fn minimum_similarity_uses_stable_ulp_boundary() {
        let min = 0.5_f32;
        let one = f32::from_bits(min.to_bits() + 1);
        let two = f32::from_bits(min.to_bits() + 2);
        assert!(top_k_similarity([(0, one)], 1, Some(min)).is_empty());
        assert_eq!(top_k_similarity([(0, two)], 1, Some(min)), vec![(0, two)]);
    }
}

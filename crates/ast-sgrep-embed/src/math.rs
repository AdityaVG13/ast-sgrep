use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use rayon::prelude::*;

pub const MIN_SIMILARITY: f32 = 0.08;
pub const PARALLEL_CHUNK_THRESHOLD: usize = 64;

/// A threshold is crossed only when the score is more than one representable
/// f32 step above it. This keeps products that merely round upward at the
/// boundary from changing an exclusive gate decision.
fn exceeds_similarity_threshold(sim: f32, min: f32) -> bool {
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
fn compare_hits_desc(left: &(usize, f32), right: &(usize, f32)) -> Ordering {
    score_order(right.1, left.1).then_with(|| left.0.cmp(&right.0))
}
pub fn dot_similarity(a: &[f32], b: &[f32]) -> f32 {
/// Returns the dot product of two equal-length vectors.
///
/// This is cosine similarity only when both inputs are already L2-normalized.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() { return 0.0; }
    let (dot, norm_a, norm_b) = a.iter().zip(b).fold(
        (0.0_f64, 0.0_f64, 0.0_f64),
        |(dot, norm_a, norm_b), (&left, &right)| {
            let left = f64::from(left);
            let right = f64::from(right);
            (
                left.mul_add(right, dot),
                left.mul_add(left, norm_a),
                right.mul_add(right, norm_b),
            )
        },
    );
    if norm_a == 0.0 || norm_b == 0.0 { return 0.0; }
    (dot / (norm_a.sqrt() * norm_b.sqrt())) as f32
}
pub fn cosine_scores_for<'a>(
    query_vec: &[f32],
    rows: impl Iterator<Item = (usize, &'a [f32])>,
) -> Vec<(usize, f32)> {
    rows.filter(|(_, emb)| emb.len() == query_vec.len())
        .map(|(idx, emb)| (idx, dot_similarity(query_vec, emb)))
        .collect()
}
pub fn top_k_similarity(
    scored: impl IntoIterator<Item = (usize, f32)>,
    limit: usize,
    min_similarity: Option<f32>,
) -> Vec<(usize, f32)> {
    if limit == 0 { return vec![]; }
    let mut heap = BinaryHeap::new();
    for (idx, sim) in scored {
        if sim.is_finite() && min_similarity.is_none_or(|min| sim >= min) {
        if sim.is_finite()
            && min_similarity.is_none_or(|min| exceeds_similarity_threshold(sim, min))
        {
            push_top_k(&mut heap, limit, idx, sim);
        }
    }
    heap_to_sorted_vec(heap)
}
pub fn top_k_flat_similarity(
    query_vec: &[f32],
    flat: &[f32],
    dim: usize,
    limit: usize,
    min_similarity: Option<f32>,
) -> Vec<(usize, f32)> {
    let n = flat.len() / dim;
    if limit == 0 || n == 0 || dim == 0 || query_vec.len() != dim { return vec![]; }
    if n < PARALLEL_CHUNK_THRESHOLD {
        let mut heap = BinaryHeap::new();
        for i in 0..n {
            let sim = dot_similarity(query_vec, &flat[i * dim..(i + 1) * dim]);
            if min_similarity.is_none_or(|min| sim >= min) {
            let sim = cosine_similarity(query_vec, &flat[i * dim..(i + 1) * dim]);
            if sim.is_finite() && min_similarity.is_none_or(|min| sim > min) {
            if min_similarity.is_none_or(|min| exceeds_similarity_threshold(sim, min)) {
                push_top_k(&mut heap, limit, i, sim);
            }
        }
        return heap_to_sorted_vec(heap);
    }
    let heap = (0..n)
        .into_par_iter()
        .fold(BinaryHeap::new, |mut heap, i| {
            let sim = dot_similarity(query_vec, &flat[i * dim..(i + 1) * dim]);
            if min_similarity.is_none_or(|min| sim >= min) {
            let sim = cosine_similarity(query_vec, &flat[i * dim..(i + 1) * dim]);
            if sim.is_finite() && min_similarity.is_none_or(|min| sim > min) {
            if min_similarity.is_none_or(|min| exceeds_similarity_threshold(sim, min)) {
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
    heap_to_sorted_vec(heap)
}
fn push_top_k(heap: &mut BinaryHeap<Reverse<Scored>>, limit: usize, idx: usize, sim: f32) {
    heap.push(Reverse(Scored { idx, sim }));
    if heap.len() > limit {
        heap.pop();
    }
}
fn heap_to_sorted_vec(heap: BinaryHeap<Reverse<Scored>>) -> Vec<(usize, f32)> {
    let mut out: Vec<(usize, f32)> = heap.into_iter().map(|Reverse(s)| (s.idx, s.sim)).collect();
    out.sort_by(compare_hits_desc);
    out
}
pub fn top_by_similarity(
    mut scored: Vec<(usize, f32)>,
    limit: usize,
    min_similarity: Option<f32>,
) -> Vec<(usize, f32)> {
    if limit == 0 { return vec![]; }
    if let Some(min) = min_similarity {
        scored.retain(|(_, sim)| *sim >= min);
        scored.retain(|(_, sim)| exceeds_similarity_threshold(*sim, min));
    }
    scored.retain(|(_, sim)| {
        sim.is_finite() && min_similarity.is_none_or(|min| *sim > min)
    });
    scored.sort_by(compare_hits_desc);
    scored.truncate(limit);
    scored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_similarity_is_scale_equivariant() {
        let a = [1.0, -2.0, 3.0];
        let b = [4.0, 5.0, -6.0];

        assert_eq!(dot_similarity(&a, &b), -24.0);
        assert_eq!(dot_similarity(&[2.0, -4.0, 6.0], &b), -48.0);
    }

    #[test]
    fn minimum_similarity_is_inclusive_across_rankers() {
        let scored = vec![(0, MIN_SIMILARITY)];

        assert_eq!(
            top_k_similarity(scored.clone(), 1, Some(MIN_SIMILARITY)),
            scored
        );
        assert_eq!(
            top_by_similarity(scored.clone(), 1, Some(MIN_SIMILARITY)),
            scored
        );
        assert_eq!(
            top_k_flat_similarity(
                &[MIN_SIMILARITY],
                &[1.0],
                1,
                1,
                Some(MIN_SIMILARITY),
            ),
            scored
    fn rounded_boundary_product_does_not_cross_minimum() {
        let rounded = 0.1_f32 * 0.8_f32;
        assert!(rounded > MIN_SIMILARITY);
        assert!(top_k_similarity([(0, rounded)], 1, Some(MIN_SIMILARITY)).is_empty());
    }

    #[test]
    fn score_beyond_boundary_band_crosses_minimum() {
        let one_ulp = f32::from_bits(MIN_SIMILARITY.to_bits() + 1);
        let two_ulps = f32::from_bits(MIN_SIMILARITY.to_bits() + 2);
        assert!(top_by_similarity(vec![(0, one_ulp)], 1, Some(MIN_SIMILARITY)).is_empty());
        assert_eq!(
            top_by_similarity(vec![(0, two_ulps)], 1, Some(MIN_SIMILARITY)),
            vec![(0, two_ulps)]
        );
    }
}

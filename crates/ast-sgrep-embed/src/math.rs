//! Shared vector math for embedding backends.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use rayon::prelude::*;
use simsimd::SpatialSimilarity;

/// Minimum similarity for semantic chunk hits.
pub const MIN_SIMILARITY: f32 = 0.08;

/// Vectors below this count use serial scoring (thread overhead dominates).
pub const PARALLEL_CHUNK_THRESHOLD: usize = 64;

/// Use SIMD dot product for vectors at least this long.
const SIMD_DOT_THRESHOLD: usize = 64;

#[derive(Clone, Copy, PartialEq)]
struct Scored {
    idx: usize,
    sim: f32,
}

impl Eq for Scored {}

impl PartialOrd for Scored {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.sim.partial_cmp(&other.sim)
    }
}

impl Ord for Scored {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap_or(Ordering::Equal)
    }
}

/// Dot product of two equal-length unit-ish embedding vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    if a.len() >= SIMD_DOT_THRESHOLD {
        if let Some(d) = f32::dot(a, b) {
            return d as f32;
        }
    }
    scalar_dot(a, b)
}

fn scalar_dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Score indexed rows by cosine similarity to a query vector.
pub fn cosine_scores_for<'a>(
    query_vec: &[f32],
    rows: impl Iterator<Item = (usize, &'a [f32])>,
) -> Vec<(usize, f32)> {
    rows.filter(|(_, emb)| emb.len() == query_vec.len())
        .map(|(idx, emb)| (idx, cosine_similarity(query_vec, emb)))
        .collect()
}

/// Score a flattened, row-major matrix of normalized vectors.
pub fn cosine_scores_flat(query_vec: &[f32], flat: &[f32], dim: usize) -> Vec<(usize, f32)> {
    let n = flat.len() / dim;
    if n == 0 || dim == 0 || query_vec.len() != dim {
        return Vec::new();
    }
    if n < PARALLEL_CHUNK_THRESHOLD {
        return (0..n)
            .map(|i| {
                let start = i * dim;
                (i, cosine_similarity(query_vec, &flat[start..start + dim]))
            })
            .collect();
    }
    (0..n)
        .into_par_iter()
        .map(|i| {
            let start = i * dim;
            (i, cosine_similarity(query_vec, &flat[start..start + dim]))
        })
        .collect()
}

/// Select top-k by similarity using a bounded heap (O(n log k), no full sort).
pub fn top_k_similarity(
    scored: impl IntoIterator<Item = (usize, f32)>,
    limit: usize,
    min_similarity: Option<f32>,
) -> Vec<(usize, f32)> {
    if limit == 0 {
        return Vec::new();
    }
    let mut heap = BinaryHeap::new();
    for (idx, sim) in scored {
        if min_similarity.is_some_and(|min| sim <= min) {
            continue;
        }
        push_top_k(&mut heap, limit, idx, sim);
    }
    heap_to_sorted_vec(heap)
}

/// Score a flat matrix and return top-k matches without sorting all candidates.
pub fn top_k_flat_similarity(
    query_vec: &[f32],
    flat: &[f32],
    dim: usize,
    limit: usize,
    min_similarity: Option<f32>,
) -> Vec<(usize, f32)> {
    let n = flat.len() / dim;
    if limit == 0 || n == 0 || dim == 0 || query_vec.len() != dim {
        return Vec::new();
    }
    if n < PARALLEL_CHUNK_THRESHOLD {
        return top_k_flat_serial(query_vec, flat, dim, limit, min_similarity);
    }
    let heap = (0..n)
        .into_par_iter()
        .fold(
            || BinaryHeap::new(),
            |mut heap, i| {
                let start = i * dim;
                let sim = cosine_similarity(query_vec, &flat[start..start + dim]);
                if min_similarity.is_none_or(|min| sim > min) {
                    push_top_k(&mut heap, limit, i, sim);
                }
                heap
            },
        )
        .reduce(
            || BinaryHeap::new(),
            |mut left, right| {
                merge_top_k(&mut left, right, limit);
                left
            },
        );
    heap_to_sorted_vec(heap)
}

fn top_k_flat_serial(
    query_vec: &[f32],
    flat: &[f32],
    dim: usize,
    limit: usize,
    min_similarity: Option<f32>,
) -> Vec<(usize, f32)> {
    let n = flat.len() / dim;
    let mut heap = BinaryHeap::new();
    for i in 0..n {
        let start = i * dim;
        let sim = cosine_similarity(query_vec, &flat[start..start + dim]);
        if min_similarity.is_some_and(|min| sim <= min) {
            continue;
        }
        push_top_k(&mut heap, limit, i, sim);
    }
    heap_to_sorted_vec(heap)
}

fn push_top_k(heap: &mut BinaryHeap<Reverse<Scored>>, limit: usize, idx: usize, sim: f32) {
    heap.push(Reverse(Scored { idx, sim }));
    if heap.len() > limit {
        heap.pop();
    }
}

fn merge_top_k(target: &mut BinaryHeap<Reverse<Scored>>, other: BinaryHeap<Reverse<Scored>>, limit: usize) {
    for Reverse(scored) in other {
        push_top_k(target, limit, scored.idx, scored.sim);
    }
}

fn heap_to_sorted_vec(heap: BinaryHeap<Reverse<Scored>>) -> Vec<(usize, f32)> {
    let mut out: Vec<(usize, f32)> = heap
        .into_iter()
        .map(|Reverse(s)| (s.idx, s.sim))
        .collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
    out
}

/// Sort by similarity descending, take `limit`, optionally filter by minimum score.
pub fn top_by_similarity(
    scored: Vec<(usize, f32)>,
    limit: usize,
    min_similarity: Option<f32>,
) -> Vec<(usize, f32)> {
    top_k_similarity(scored, limit, min_similarity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_k_matches_full_sort() {
        let scored: Vec<(usize, f32)> = (0..100)
            .map(|i| (i, (i as f32 * 0.01).sin().abs()))
            .collect();
        let limit = 7;
        let heap = top_k_similarity(scored.clone(), limit, None);
        let sorted = top_by_similarity(scored, limit, None);
        assert_eq!(heap, sorted);
    }

    #[test]
    fn top_k_flat_respects_min_similarity() {
        let dim = 4;
        let flat = vec![
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
        ];
        let query = [1.0, 0.0, 0.0, 0.0];
        let hits = top_k_flat_similarity(&query, &flat, dim, 2, Some(0.5));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, 0);
    }
}

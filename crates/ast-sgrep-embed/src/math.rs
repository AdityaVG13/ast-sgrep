//! Shared vector math for embedding backends.

use rayon::prelude::*;
use simsimd::SpatialSimilarity;

/// Minimum similarity for semantic chunk hits.
pub const MIN_SIMILARITY: f32 = 0.08;

/// Vectors below this count use serial scoring (thread overhead dominates).
pub const PARALLEL_CHUNK_THRESHOLD: usize = 64;

/// Use SIMD dot product for vectors at least this long.
const SIMD_DOT_THRESHOLD: usize = 64;

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

/// Sort by similarity descending, take `limit`, optionally filter by minimum score.
pub fn top_by_similarity(
    mut scored: Vec<(usize, f32)>,
    limit: usize,
    min_similarity: Option<f32>,
) -> Vec<(usize, f32)> {
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(limit)
        .filter(|(_, sim)| min_similarity.is_none_or(|min| *sim > min))
        .collect()
}

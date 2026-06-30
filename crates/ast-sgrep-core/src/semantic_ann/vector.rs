//! Vector normalization and flat-layout helpers for semantic ANN.

use ast_sgrep_embed::{cosine_scores_for, top_by_similarity, SemanticChunkRow, MIN_SIMILARITY};

pub(crate) fn flatten_vectors(chunks: &[SemanticChunkRow], dim: usize) -> Vec<f32> {
    let mut flat = Vec::with_capacity(chunks.len() * dim);
    for c in chunks {
        let mut v = c.5.clone();
        normalize_vec_in_place(&mut v);
        flat.extend_from_slice(&v);
    }
    flat
}

pub(crate) fn normalize_flat(vectors: &[f32], dim: usize) -> Vec<f32> {
    let n = vectors.len() / dim;
    let mut out = vectors.to_vec();
    for i in 0..n {
        let start = i * dim;
        let row = &mut out[start..start + dim];
        normalize_vec_in_place(row);
    }
    out
}

pub(crate) fn rows_from_flat(flat: &[f32], dim: usize) -> Vec<Vec<f32>> {
    let n = flat.len() / dim;
    (0..n)
        .map(|i| {
            let start = i * dim;
            flat[start..start + dim].to_vec()
        })
        .collect()
}

pub(crate) fn cluster_count(n: usize) -> usize {
    let sqrt = (n as f64).sqrt() as usize;
    sqrt.clamp(16, 256)
}

pub(crate) fn normalize_vec_in_place(vec: &mut [f32]) {
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in vec.iter_mut() {
            *x /= norm;
        }
    }
}

pub(crate) fn normalize_vec(vec: &[f32]) -> Vec<f32> {
    let mut out = vec.to_vec();
    normalize_vec_in_place(&mut out);
    out
}

pub(crate) fn brute_force_flat(
    flat: &[f32],
    dim: usize,
    query: &[f32],
    limit: usize,
) -> Vec<(usize, f32)> {
    let n = flat.len() / dim;
    let q = normalize_vec(query);
    top_by_similarity(
        cosine_scores_for(
            &q,
            (0..n).map(|i| {
                let start = i * dim;
                (i, &flat[start..start + dim])
            }),
        ),
        limit,
        Some(MIN_SIMILARITY),
    )
}

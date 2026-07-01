//! Vector normalization and brute-force semantic search.

use rayon::prelude::*;

use ast_sgrep_embed::{
    top_k_flat_similarity, SemanticChunkRow, MIN_SIMILARITY, PARALLEL_CHUNK_THRESHOLD,
};

pub fn flatten_vectors_for_search(chunks: &[SemanticChunkRow], dim: usize) -> Vec<f32> {
    let mut flat = vec![0.0f32; chunks.len() * dim];
    if chunks.len() >= PARALLEL_CHUNK_THRESHOLD {
        flat.par_chunks_mut(dim)
            .zip(chunks.par_iter())
            .for_each(|(row, chunk)| {
                row.copy_from_slice(&chunk.5);
                normalize_vec_in_place(row);
            });
    } else {
        for (i, chunk) in chunks.iter().enumerate() {
            let start = i * dim;
            flat[start..start + dim].copy_from_slice(&chunk.5);
            normalize_vec_in_place(&mut flat[start..start + dim]);
        }
    }
    flat
}

pub fn flatten_vectors(chunks: &[SemanticChunkRow], dim: usize) -> Vec<f32> {
    flatten_vectors_for_search(chunks, dim)
}

pub fn normalize_flat(vectors: &[f32], dim: usize) -> Vec<f32> {
    let n = vectors.len() / dim;
    let mut out = vectors.to_vec();
    for i in 0..n {
        let start = i * dim;
        let row = &mut out[start..start + dim];
        normalize_vec_in_place(row);
    }
    out
}

pub fn rows_from_flat(flat: &[f32], dim: usize) -> Vec<Vec<f32>> {
    let n = flat.len() / dim;
    (0..n)
        .map(|i| {
            let start = i * dim;
            flat[start..start + dim].to_vec()
        })
        .collect()
}

pub fn normalize_vec_in_place(vec: &mut [f32]) {
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in vec.iter_mut() {
            *x /= norm;
        }
    }
}

pub fn normalize_vec(vec: &[f32]) -> Vec<f32> {
    let mut out = vec.to_vec();
    normalize_vec_in_place(&mut out);
    out
}

pub fn brute_force_flat(flat: &[f32], dim: usize, query: &[f32], limit: usize) -> Vec<(usize, f32)> {
    let q = normalize_vec(query);
    top_k_flat_similarity(&q, flat, dim, limit, Some(MIN_SIMILARITY))
}

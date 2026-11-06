#![forbid(unsafe_code)]

//! Embedding for ast-sgrep: one native static engine (trained token
//! table baked into the binary, WordPiece + mean-pool, 256-d), plus the
//! text utilities and vector math the search layer shares. No backend
//! selection, no model download, no feature flags.

mod accent_table;
mod embedder;
mod math;
mod static_engine;
pub mod text;

pub use embedder::{embed_text, embed_texts};
pub use math::{
    cosine_similarity, dot_similarity, normalize_vec, normalize_vec_in_place, top_by_similarity,
    top_k_flat_similarity, top_k_similarity, MIN_SIMILARITY, PARALLEL_CHUNK_THRESHOLD,
};
pub use static_engine::{table_is_materialized, StaticEmbedder, STATIC_DIM, STATIC_MODEL_ID};
pub use text::{expand_concepts, split_ident, tokenize};
pub fn embed_to_bytes(vec: &[f32]) -> Vec<u8> {
    vec.iter().flat_map(|f| f.to_le_bytes()).collect()
}
pub fn embed_from_bytes(bytes: &[u8]) -> Result<Vec<f32>, &'static str> {
    if !bytes.len().is_multiple_of(4) {
        return Err("embedding byte length is not a multiple of 4");
    }
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect())
}
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticChunkRow {
    pub path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub symbol: String,
    pub text: String,
    pub embedding: Vec<f32>,
}

pub fn rank_chunk_indices_by_vector(
    query_vec: &[f32],
    chunks: &[SemanticChunkRow],
    limit: usize,
) -> Vec<(usize, f32)> {
    top_by_similarity(
        chunks
            .iter()
            .enumerate()
            .filter(|(_, row)| row.embedding.len() == query_vec.len())
            .map(|(idx, row)| (idx, cosine_similarity(query_vec, &row.embedding)))
            .collect(),
        limit,
        Some(MIN_SIMILARITY),
    )
}

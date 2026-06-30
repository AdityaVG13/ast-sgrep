//! Embedding plugins for ast-sgrep: semantic local, Ollama, and cloud APIs.

mod cloud;
mod ollama;
mod provider;
mod semantic;

pub use cloud::{embed_via_api, rank_by_vector, CloudEmbeddingConfig};
pub use ollama::{embed_via_ollama, OllamaEmbeddingConfig};
pub use provider::{
    embed_query, embed_with_chain, default_semantic_dim, EmbedBackendKind, EmbedPreference,
    EmbedResult,
};
pub use semantic::{expand_concepts, tokenize, SemanticLocalEmbedding, SEMANTIC_DIM};

/// Legacy dimension constant — prefer [`SEMANTIC_DIM`] for local semantic embeddings.
pub const EMBED_DIM: usize = SEMANTIC_DIM;

/// Trait for embedding providers (plugin interface).
pub trait EmbeddingProvider: Send + Sync {
    fn embed_text(&self, text: &str) -> Vec<f32>;
    fn similarity(&self, a: &[f32], b: &[f32]) -> f32;
}

impl EmbeddingProvider for SemanticLocalEmbedding {
    fn embed_text(&self, text: &str) -> Vec<f32> {
        SemanticLocalEmbedding::embed_text(self, text)
    }

    fn similarity(&self, a: &[f32], b: &[f32]) -> f32 {
        SemanticLocalEmbedding::similarity(self, a, b)
    }
}

/// Serialize embedding to bytes for SQLite storage.
pub fn embed_to_bytes(vec: &[f32]) -> Vec<u8> {
    vec.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Deserialize embedding from bytes.
pub fn embed_from_bytes(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// A semantic chunk candidate for ranking.
pub type SemanticChunkRow = (
    String,  // file
    u32,     // line_start
    u32,     // line_end
    String,  // symbol
    String,  // excerpt
    Vec<f32>, // vector
);

/// Rank semantic symbol chunks by cosine similarity to the query text.
pub fn rank_semantic_chunks(
    query: &str,
    chunks: &[SemanticChunkRow],
    limit: usize,
    stored_backend: Option<&str>,
    stored_dim: usize,
    preference: EmbedPreference,
) -> Vec<(f32, String, u32, u32, String, String)> {
    if chunks.is_empty() {
        return Vec::new();
    }
    let query_result = embed_query(query, stored_backend, stored_dim, preference);
    rank_chunks_by_vector(&query_result.vector, chunks, limit)
}

/// Rank chunks using a precomputed query embedding vector.
pub fn rank_chunks_by_vector(
    query_vec: &[f32],
    chunks: &[SemanticChunkRow],
    limit: usize,
) -> Vec<(f32, String, u32, u32, String, String)> {
    let mut scored: Vec<(f32, String, u32, u32, String, String)> = chunks
        .iter()
        .filter(|(_, _, _, _, _, emb)| emb.len() == query_vec.len())
        .map(|(file, line_start, line_end, symbol, excerpt, emb)| {
            let sim = cosine_similarity(query_vec, emb);
            (
                sim,
                file.clone(),
                *line_start,
                *line_end,
                symbol.clone(),
                excerpt.clone(),
            )
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(limit)
        .filter(|(sim, _, _, _, _, _)| *sim > 0.08)
        .collect()
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_semantic_chunks_orders_by_similarity() {
        let embedder = SemanticLocalEmbedding;
        let chunk_a = (
            "a.rs".into(),
            1,
            3,
            "auth_refresh".into(),
            "fn auth_refresh() {}".into(),
            embedder.embed_text("auth refresh token"),
        );
        let chunk_b = (
            "b.rs".into(),
            10,
            12,
            "migrate_db".into(),
            "fn migrate_db() {}".into(),
            embedder.embed_text("database migration"),
        );
        let ranked = rank_semantic_chunks(
            "credential renewal",
            &[chunk_a, chunk_b],
            2,
            Some("semantic"),
            SEMANTIC_DIM,
            EmbedPreference::Semantic,
        );
        assert!(!ranked.is_empty());
        assert_eq!(ranked[0].4, "auth_refresh");
    }
}

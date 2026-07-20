mod embedder; mod math; mod neural; mod provider; mod remote; #[cfg(feature = "rerank")] mod rerank; mod semantic;

pub use embedder::{embedder_for, CloudEmbedder, CostHint, Embedder, HashedEmbedder, OllamaEmbedder}; pub use math::{
    cosine_scores_for, cosine_similarity, dot_similarity, top_by_similarity, top_k_flat_similarity, top_k_similarity, MIN_SIMILARITY, PARALLEL_CHUNK_THRESHOLD,
}; #[cfg(feature = "neural-embed")] pub use neural::NeuralEmbedder; pub use neural::{
    configured_model_id as neural_configured_model_id, default_cache_dir as neural_default_cache_dir, NeuralEmbeddingConfig, NeuralModel,
}; pub use provider::{
    default_semantic_dim, embed_batch_with_chain, embed_query, embed_with_chain, EmbedBackendKind, EmbedPreference, EmbedResult,
}; pub use remote::{embed_via_api, embed_via_ollama, CloudEmbeddingConfig, OllamaEmbeddingConfig}; #[cfg(feature = "rerank")] pub use rerank::{rerank, RerankScore};
pub use semantic::{expand_concepts, tokenize, SemanticLocalEmbedding, SEMANTIC_DIM};

pub trait EmbeddingProvider: Send + Sync {
    fn embed_text(&self, text: &str) -> Vec<f32>; fn similarity(&self, a: &[f32], b: &[f32]) -> f32;
} impl EmbeddingProvider for SemanticLocalEmbedding {
    fn embed_text(&self, text: &str) -> Vec<f32> { SemanticLocalEmbedding::embed_text(self, text) } fn similarity(&self, a: &[f32], b: &[f32]) -> f32 { SemanticLocalEmbedding::similarity(self, a, b) }
}

pub fn embed_to_bytes(vec: &[f32]) -> Vec<u8> { vec.iter().flat_map(|f| f.to_le_bytes()).collect() } pub fn embed_from_bytes(bytes: &[u8]) -> Result<Vec<f32>, &'static str> {
    if !bytes.len().is_multiple_of(4) { return Err("embedding byte length is not a multiple of 4"); } Ok(bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

pub type SemanticChunkRow = (String, u32, u32, String, String, Vec<f32>);

pub fn rank_chunk_indices_by_vector(query_vec: &[f32], chunks: &[SemanticChunkRow], limit: usize) -> Vec<(usize, f32)> {
    let qn = l2_norm(query_vec); top_by_similarity(
        chunks.iter().enumerate()
            .filter(|(_, (_, _, _, _, _, emb))| emb.len() == query_vec.len()) .map(|(idx, (_, _, _, _, _, emb))| {
                let d = qn * l2_norm(emb); (idx, if d > 0.0 { dot_similarity(query_vec, emb) / d } else { 0.0 })
            }).collect(),
        limit, Some(MIN_SIMILARITY),
    )
} fn l2_norm(v: &[f32]) -> f32 { v.iter().map(|x| x * x).sum::<f32>().sqrt() }

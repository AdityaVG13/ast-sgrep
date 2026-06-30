//! Embedding plugins for ast-sgrep: local (offline) and cloud (API).

mod cloud;

pub use cloud::{embed_via_api, rank_by_vector, CloudEmbeddingConfig};

/// Embedding vector dimension for local hash embeddings.
pub const EMBED_DIM: usize = 64;

/// Trait for embedding providers (plugin interface).
pub trait EmbeddingProvider: Send + Sync {
    fn embed_text(&self, text: &str) -> Vec<f32>;
    fn similarity(&self, a: &[f32], b: &[f32]) -> f32;
}

/// Local bag-of-words hash embedding — fully offline.
#[derive(Debug, Clone, Default)]
pub struct LocalEmbedding;

impl EmbeddingProvider for LocalEmbedding {
    fn embed_text(&self, text: &str) -> Vec<f32> {
        let mut vec = vec![0.0_f32; EMBED_DIM];
        for token in text.split(|c: char| !c.is_alphanumeric() && c != '_') {
            if token.len() < 2 {
                continue;
            }
            let hash = blake3::hash(token.to_lowercase().as_bytes());
            let bytes = hash.as_bytes();
            for i in 0..EMBED_DIM {
                let b = bytes[i % bytes.len()];
                vec[i] += if b & 1 == 0 { 1.0 } else { -1.0 };
            }
        }
        normalize(&mut vec);
        vec
    }

    fn similarity(&self, a: &[f32], b: &[f32]) -> f32 {
        cosine_similarity(a, b)
    }
}

fn normalize(vec: &mut [f32]) {
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in vec.iter_mut() {
            *x /= norm;
        }
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
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

/// Compute embedding for a line of text.
pub fn embed_line(text: &str) -> Vec<f32> {
    LocalEmbedding.embed_text(text)
}

/// Rank lines by semantic similarity to query.
pub fn rank_by_similarity(
    query: &str,
    lines: &[(String, u32, String, Vec<f32>)],
    limit: usize,
) -> Vec<(f32, String, u32, String)> {
    let provider = LocalEmbedding;
    let query_vec = provider.embed_text(query);
    let mut scored: Vec<(f32, String, u32, String)> = lines
        .iter()
        .map(|(file, line_no, content, emb)| {
            let sim = provider.similarity(&query_vec, emb);
            (sim, file.clone(), *line_no, content.clone())
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(limit)
        .filter(|(sim, _, _, _)| *sim > 0.05)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similar_texts_score_higher() {
        let p = LocalEmbedding;
        let a = p.embed_text("auth refresh token");
        let b = p.embed_text("refresh auth token");
        let c = p.embed_text("unrelated database schema");
        assert!(p.similarity(&a, &b) > p.similarity(&a, &c));
    }
}

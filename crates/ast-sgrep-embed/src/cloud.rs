//! Cloud embedding provider (OpenAI-compatible API).

#[cfg(feature = "cloud")]
use serde::{Deserialize, Serialize};

/// Configuration for cloud embedding APIs (OpenAI-compatible).
#[derive(Debug, Clone)]
pub struct CloudEmbeddingConfig {
    pub api_url: String,
    pub api_key: String,
    pub model: String,
}

impl CloudEmbeddingConfig {
    /// Load from environment: `ASGREP_EMBED_API_URL`, `ASGREP_EMBED_API_KEY`, `ASGREP_EMBED_MODEL`.
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("ASGREP_EMBED_API_KEY").ok()?;
        let api_url = std::env::var("ASGREP_EMBED_API_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1/embeddings".to_string());
        let model = std::env::var("ASGREP_EMBED_MODEL").unwrap_or_else(|_| "text-embedding-3-small".to_string());
        Some(Self {
            api_url,
            api_key,
            model,
        })
    }
}

#[derive(Serialize)]
#[cfg(feature = "cloud")]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a str,
}

#[derive(Deserialize)]
#[cfg(feature = "cloud")]
struct EmbedResponse {
    data: Vec<EmbedData>,
}

#[derive(Deserialize)]
#[cfg(feature = "cloud")]
struct EmbedData {
    embedding: Vec<f32>,
}

/// Fetch embedding vector from a cloud API (OpenAI-compatible JSON format).
#[cfg(feature = "cloud")]
pub fn embed_via_api(text: &str, config: &CloudEmbeddingConfig) -> Result<Vec<f32>, String> {
    let body = EmbedRequest {
        model: &config.model,
        input: text,
    };
    let json = serde_json::to_string(&body).map_err(|e| e.to_string())?;
    let response = ureq::post(&config.api_url)
        .set("Authorization", &format!("Bearer {}", config.api_key))
        .set("Content-Type", "application/json")
        .send_string(&json)
        .map_err(|e| e.to_string())?;
    let parsed: EmbedResponse = response.into_json().map_err(|e| e.to_string())?;
    parsed
        .data
        .into_iter()
        .next()
        .map(|d| d.embedding)
        .ok_or_else(|| "empty embedding response".to_string())
}

#[cfg(not(feature = "cloud"))]
pub fn embed_via_api(_text: &str, _config: &CloudEmbeddingConfig) -> Result<Vec<f32>, String> {
    Err("cloud embedding feature not enabled; rebuild with --features cloud".to_string())
}

/// Rank lines using a precomputed query embedding vector.
pub fn rank_by_vector(
    query_vec: &[f32],
    lines: &[(String, u32, String, Vec<f32>)],
    limit: usize,
) -> Vec<(f32, String, u32, String)> {
    let mut scored: Vec<(f32, String, u32, String)> = lines
        .iter()
        .map(|(file, line_no, content, emb)| {
            let sim = cosine_similarity(query_vec, emb);
            (sim, file.clone(), *line_no, content.clone())
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().take(limit).collect()
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len().min(b.len());
    if len == 0 {
        return 0.0;
    }
    a.iter()
        .zip(b.iter())
        .take(len)
        .map(|(x, y)| x * y)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_by_vector_orders_by_similarity() {
        let q = vec![1.0, 0.0];
        let lines = vec![
            ("a.rs".into(), 1, "x".into(), vec![1.0, 0.0]),
            ("b.rs".into(), 2, "y".into(), vec![0.0, 1.0]),
        ];
        let ranked = rank_by_vector(&q, &lines, 2);
        assert!(ranked[0].0 > ranked[1].0);
    }
}

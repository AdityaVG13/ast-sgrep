//! Ollama local embedding API (OpenAI-compatible `/v1/embeddings` or native).

#[cfg(feature = "cloud")]
use serde::{Deserialize, Serialize};

/// Configuration for Ollama embedding API.
#[derive(Debug, Clone)]
pub struct OllamaEmbeddingConfig {
    pub api_url: String,
    pub model: String,
}

impl OllamaEmbeddingConfig {
    /// Load from `ASGREP_OLLAMA_URL` (default `http://127.0.0.1:11434`) and
    /// `ASGREP_OLLAMA_MODEL` (default `nomic-embed-text`).
    pub fn from_env() -> Option<Self> {
        if std::env::var("ASGREP_NO_OLLAMA").ok().as_deref() == Some("1") {
            return None;
        }
        let explicit = std::env::var("ASGREP_OLLAMA_EMBED").ok().as_deref() == Some("1");
        let url_set = std::env::var("ASGREP_OLLAMA_URL").is_ok();
        if !explicit && !url_set {
            return None;
        }
        let api_url = std::env::var("ASGREP_OLLAMA_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
        let model = std::env::var("ASGREP_OLLAMA_MODEL")
            .unwrap_or_else(|_| "nomic-embed-text".to_string());
        Some(Self { api_url, model })
    }

    fn embeddings_endpoint(&self) -> String {
        let base = self.api_url.trim_end_matches('/');
        format!("{base}/api/embeddings")
    }
}

#[derive(Serialize)]
#[cfg(feature = "cloud")]
struct OllamaEmbedRequest<'a> {
    model: &'a str,
    prompt: &'a str,
}

#[derive(Deserialize)]
#[cfg(feature = "cloud")]
struct OllamaEmbedResponse {
    embedding: Vec<f32>,
}

/// Fetch embedding from a running Ollama instance.
#[cfg(feature = "cloud")]
pub fn embed_via_ollama(text: &str, config: &OllamaEmbeddingConfig) -> Result<Vec<f32>, String> {
    let body = OllamaEmbedRequest {
        model: &config.model,
        prompt: text,
    };
    let json = serde_json::to_string(&body).map_err(|e| e.to_string())?;
    let response = ureq::post(&config.embeddings_endpoint())
        .set("Content-Type", "application/json")
        .send_string(&json)
        .map_err(|e| e.to_string())?;
    let parsed: OllamaEmbedResponse = response.into_json().map_err(|e| e.to_string())?;
    if parsed.embedding.is_empty() {
        return Err("empty ollama embedding response".to_string());
    }
    Ok(parsed.embedding)
}

#[cfg(not(feature = "cloud"))]
pub fn embed_via_ollama(_text: &str, _config: &OllamaEmbeddingConfig) -> Result<Vec<f32>, String> {
    Err("ollama embedding requires cloud feature (ureq)".to_string())
}

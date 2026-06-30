//! Embedding provider chain: cloud → Ollama → semantic local.

use crate::cloud::{embed_via_api, CloudEmbeddingConfig};
use crate::ollama::{embed_via_ollama, OllamaEmbeddingConfig};
use crate::semantic::{SemanticLocalEmbedding, SEMANTIC_DIM};

/// Which backend produced stored embeddings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedBackendKind {
    Cloud,
    Ollama,
    Semantic,
}

impl EmbedBackendKind {
    pub fn as_meta_str(self) -> &'static str {
        match self {
            EmbedBackendKind::Cloud => "cloud",
            EmbedBackendKind::Ollama => "ollama",
            EmbedBackendKind::Semantic => "semantic",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cloud" => Some(EmbedBackendKind::Cloud),
            "ollama" => Some(EmbedBackendKind::Ollama),
            "semantic" | "local" => Some(EmbedBackendKind::Semantic),
            _ => None,
        }
    }
}

/// Preferred embedding backend for indexing and search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmbedPreference {
    #[default]
    Auto,
    Cloud,
    Ollama,
    Semantic,
}

/// Result of embedding a single text chunk.
#[derive(Debug, Clone)]
pub struct EmbedResult {
    pub vector: Vec<f32>,
    pub backend: EmbedBackendKind,
}

/// Embed text using the provider chain.
pub fn embed_with_chain(text: &str, preference: EmbedPreference) -> EmbedResult {
    match preference {
        EmbedPreference::Cloud => {
            if let Some(vec) = try_cloud(text) {
                return EmbedResult {
                    vector: vec,
                    backend: EmbedBackendKind::Cloud,
                };
            }
            if let Some(vec) = try_ollama(text) {
                return EmbedResult {
                    vector: vec,
                    backend: EmbedBackendKind::Ollama,
                };
            }
        }
        EmbedPreference::Ollama => {
            if let Some(vec) = try_ollama(text) {
                return EmbedResult {
                    vector: vec,
                    backend: EmbedBackendKind::Ollama,
                };
            }
        }
        EmbedPreference::Semantic | EmbedPreference::Auto => {}
    }

    if matches!(preference, EmbedPreference::Auto) {
        if let Some(vec) = try_cloud(text) {
            return EmbedResult {
                vector: vec,
                backend: EmbedBackendKind::Cloud,
            };
        }
        if let Some(vec) = try_ollama(text) {
            return EmbedResult {
                vector: vec,
                backend: EmbedBackendKind::Ollama,
            };
        }
    }

    let embedder = SemanticLocalEmbedding;
    EmbedResult {
        vector: embedder.embed_text(text),
        backend: EmbedBackendKind::Semantic,
    }
}

/// Embed a query vector, respecting stored index backend when possible.
pub fn embed_query(
    text: &str,
    stored_backend: Option<&str>,
    stored_dim: usize,
    preference: EmbedPreference,
) -> EmbedResult {
    if let Some(backend) = stored_backend.and_then(EmbedBackendKind::parse) {
        let result = match backend {
            EmbedBackendKind::Cloud => try_cloud(text).map(|vector| EmbedResult {
                vector,
                backend: EmbedBackendKind::Cloud,
            }),
            EmbedBackendKind::Ollama => try_ollama(text).map(|vector| EmbedResult {
                vector,
                backend: EmbedBackendKind::Ollama,
            }),
            EmbedBackendKind::Semantic => None,
        };
        if let Some(r) = result {
            if stored_dim == 0 || r.vector.len() == stored_dim {
                return r;
            }
        }
    }
    embed_with_chain(text, preference)
}

fn try_cloud(text: &str) -> Option<Vec<f32>> {
    let config = CloudEmbeddingConfig::from_env()?;
    embed_via_api(text, &config).ok()
}

fn try_ollama(text: &str) -> Option<Vec<f32>> {
    let config = OllamaEmbeddingConfig::from_env()?;
    embed_via_ollama(text, &config).ok()
}

/// Default local semantic dimension (used when no neural backend is available).
pub fn default_semantic_dim() -> usize {
    SEMANTIC_DIM
}

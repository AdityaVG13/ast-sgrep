use anyhow::Result;

fn is_boolish_true(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .as_deref()
        .is_some_and(is_boolish_true)
}

fn env_allows_neural_fallback() -> bool {
    env_flag("ASGREP_NEURAL_FALLBACK")
}

use crate::semantic::{SemanticLocalEmbedding, SEMANTIC_DIM};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostHint {
    LocalCheap,
    LocalCompute,
}

pub trait Embedder: Send + Sync {
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let mut out = self.embed_batch(&[text])?;
        Ok(out.pop().unwrap_or_default())
    }
    fn dim(&self) -> usize;
    fn model_id(&self) -> &str;
    fn cost_hint(&self) -> CostHint;
}

pub struct HashedEmbedder {
    inner: SemanticLocalEmbedding,
    model_id: String,
}

impl Default for HashedEmbedder {
    fn default() -> Self {
        Self {
            inner: SemanticLocalEmbedding,
            // `-xof` marks full-rank feature hashing (not period-32 blake3 tiling).
            model_id: format!("hashed-{SEMANTIC_DIM}-xof"),
        }
    }
}

impl Embedder for HashedEmbedder {
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.inner.embed_text(t)).collect())
    }
    fn dim(&self) -> usize {
        SEMANTIC_DIM
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn cost_hint(&self) -> CostHint {
        CostHint::LocalCheap
    }
}

pub fn embedder_for(kind: EmbedBackendKind) -> Option<Box<dyn Embedder>> {
    match kind {
        EmbedBackendKind::Neural => neural_embedder(),
        EmbedBackendKind::Semantic => Some(Box::new(HashedEmbedder::default())),
    }
}

#[cfg(feature = "neural-embed")]
impl Embedder for std::sync::Arc<crate::neural::NeuralEmbedder> {
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        crate::neural::NeuralEmbedder::embed_batch(self, texts)
    }
    fn dim(&self) -> usize {
        crate::neural::NeuralEmbedder::dim(self)
    }
    fn model_id(&self) -> &str {
        crate::neural::NeuralEmbedder::model_id(self)
    }
    fn cost_hint(&self) -> CostHint {
        CostHint::LocalCompute
    }
}

#[cfg(feature = "neural-embed")]
fn neural_embedder() -> Option<Box<dyn Embedder>> {
    use crate::neural::{NeuralEmbedder, NeuralEmbeddingConfig};
    use std::collections::HashMap;
    use std::sync::{Arc, LazyLock, Mutex};
    type Cache = HashMap<NeuralEmbeddingConfig, Option<Arc<NeuralEmbedder>>>;
    static INSTANCES: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(HashMap::new()));
    let config = NeuralEmbeddingConfig::configured();
    let mut instances = INSTANCES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(cached) = instances.get(&config) {
        return cached.clone().map(|arc| Box::new(arc) as Box<dyn Embedder>);
    }
    let cached = match NeuralEmbedder::new(config.clone()) {
        Ok(embedder) => Some(Arc::new(embedder)),
        Err(err) => {
            if env_allows_neural_fallback() {
                eprintln!(
                    "asgrep: neural embedder unavailable; ASGREP_NEURAL_FALLBACK=1 acknowledged hashed fallback: {err}"
                );
            } else {
                eprintln!(
                    "asgrep: neural embedder unavailable (set ASGREP_NEURAL_FALLBACK=1 to acknowledge hashed fallback): {err}"
                );
            }
            None
        }
    };
    instances.insert(config, cached.clone());
    cached.map(|arc| Box::new(arc) as Box<dyn Embedder>)
}

#[cfg(not(feature = "neural-embed"))]
fn neural_embedder() -> Option<Box<dyn Embedder>> {
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedBackendKind {
    Neural,
    Semantic,
}

impl EmbedBackendKind {
    pub fn as_meta_str(self) -> &'static str {
        match self {
            Self::Neural => "neural",
            Self::Semantic => "semantic-v2",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "neural" | "fastembed" => Some(Self::Neural),
            "semantic-v2" | "semantic" | "local" => Some(Self::Semantic),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmbedPreference {
    #[default]
    Auto,
    Neural,
    Semantic,
}

#[derive(Debug, Clone)]
pub struct EmbedResult {
    pub vector: Vec<f32>,
    pub backend: EmbedBackendKind,
}

pub fn embed_with_chain(text: &str, preference: EmbedPreference) -> EmbedResult {
    for kind in chain_kinds(preference) {
        if let Some(vector) = try_backend(kind, text) {
            return EmbedResult {
                vector,
                backend: kind,
            };
        }
    }
    if matches!(preference, EmbedPreference::Neural) && !env_allows_neural_fallback() {
        eprintln!(
            "asgrep: neural preference requested but neural embedder unavailable; \
             refusing silent hashed swap -- set ASGREP_NEURAL_FALLBACK=1 to acknowledge"
        );
    }
    EmbedResult {
        vector: try_backend(EmbedBackendKind::Semantic, text)
            .expect("local semantic embedder is always available and infallible"),
        backend: EmbedBackendKind::Semantic,
    }
}

pub fn embed_batch_with_chain(texts: &[&str], preference: EmbedPreference) -> Vec<EmbedResult> {
    if texts.is_empty() {
        return vec![];
    }
    for kind in chain_kinds(preference) {
        if let Some(vectors) = try_backend_batch(kind, texts) {
            return vectors
                .into_iter()
                .map(|v| EmbedResult {
                    vector: v,
                    backend: kind,
                })
                .collect();
        }
    }
    try_backend_batch(EmbedBackendKind::Semantic, texts)
        .expect("local semantic embedder is always available and infallible")
        .into_iter()
        .map(|v| EmbedResult {
            vector: v,
            backend: EmbedBackendKind::Semantic,
        })
        .collect()
}

fn chain_kinds(preference: EmbedPreference) -> Vec<EmbedBackendKind> {
    match preference {
        EmbedPreference::Neural => vec![EmbedBackendKind::Neural],
        EmbedPreference::Semantic => vec![],
        EmbedPreference::Auto => {
            if crate::neural::NeuralEmbeddingConfig::from_env().is_some() {
                vec![EmbedBackendKind::Neural]
            } else {
                vec![]
            }
        }
    }
}

pub fn embed_query(
    text: &str,
    stored_backend: Option<&str>,
    stored_dim: usize,
    preference: EmbedPreference,
) -> Result<EmbedResult, String> {
    if let Some(stored) = stored_backend {
        if stored == "cloud" || stored == "ollama" {
            return Err(
                "stored embedding backend was an HTTP provider (cloud/ollama); those backends were removed -- reindex with hashed or neural: asgrep reindex"
                    .into(),
            );
        }
        let backend = EmbedBackendKind::parse(stored)
            .ok_or_else(|| format!("unknown stored embedding backend {stored:?}"))?;
        return match try_backend(backend, text).map(|vector| EmbedResult { vector, backend }) {
            Some(r) if stored_dim == 0 || r.vector.len() == stored_dim => Ok(r),
            Some(r) => Err(format!(
                "stored embedding backend '{}' (dim {}) does not match active backend '{}' (dim {}); reindex the store with 'asgrep index --force-reindex'",
                backend.as_meta_str(),
                stored_dim,
                pref_str(preference),
                r.vector.len()
            )),
            None => Err(format!(
                "stored embedding backend '{}' is not available; switch backends or reindex with 'asgrep index --force-reindex' using '{}'",
                backend.as_meta_str(),
                pref_str(preference)
            )),
        };
    }
    Ok(embed_with_chain(text, preference))
}

fn pref_str(p: EmbedPreference) -> &'static str {
    match p {
        EmbedPreference::Auto => "auto",
        EmbedPreference::Neural => "neural",
        EmbedPreference::Semantic => "semantic",
    }
}

fn try_backend(kind: EmbedBackendKind, text: &str) -> Option<Vec<f32>> {
    embedder_for(kind)?.embed(text).ok()
}

fn try_backend_batch(kind: EmbedBackendKind, texts: &[&str]) -> Option<Vec<Vec<f32>>> {
    embedder_for(kind)?.embed_batch(texts).ok()
}

pub fn configured_backend_model_id(kind: EmbedBackendKind, dim: usize) -> Option<String> {
    match kind {
        EmbedBackendKind::Semantic => Some(format!("semantic:hashed-v2:{dim}")),
        EmbedBackendKind::Neural => {
            Some(format!("neural:{}", crate::neural::configured_model_id()))
        }
    }
}

pub fn default_semantic_dim() -> usize {
    SEMANTIC_DIM
}



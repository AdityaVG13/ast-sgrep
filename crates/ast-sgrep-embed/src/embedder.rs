use anyhow::{anyhow, Result}; use crate::provider::EmbedBackendKind; use crate::remote::{embed_via_api, embed_via_ollama, CloudEmbeddingConfig, OllamaEmbeddingConfig}; use crate::semantic::{SemanticLocalEmbedding, SEMANTIC_DIM};

#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum CostHint { LocalCheap, LocalCompute, Network }

pub trait Embedder: Send + Sync {
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>; fn embed(&self, text: &str) -> Result<Vec<f32>> { Ok(self.embed_batch(&[text])?.pop().unwrap_or_default()) } fn dim(&self) -> usize; fn model_id(&self) -> &str;
    fn cost_hint(&self) -> CostHint;
}

pub struct HashedEmbedder { inner: SemanticLocalEmbedding, model_id: String } impl Default for HashedEmbedder {
    fn default() -> Self { Self { inner: SemanticLocalEmbedding, model_id: format!("hashed-{SEMANTIC_DIM}") } }
} impl Embedder for HashedEmbedder {
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> { Ok(texts.iter().map(|t| self.inner.embed_text(t)).collect()) } fn dim(&self) -> usize { SEMANTIC_DIM } fn model_id(&self) -> &str { &self.model_id }
    fn cost_hint(&self) -> CostHint { CostHint::LocalCheap }
}

macro_rules! net_embedder {
    ($name:ident, $cfg:ty, $prefix:literal, $call:ident) => {
        pub struct $name { config: $cfg, model_id: String } impl $name {
            pub fn new(config: $cfg) -> Self {
                let model_id = format!(concat!($prefix, ":{}"), config.model); Self { config, model_id }
            }
        } impl Embedder for $name {
            fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
                texts.iter().map(|t| $call(t, &self.config).map_err(|e| anyhow!(e))).collect()
            } fn dim(&self) -> usize { 0 } fn model_id(&self) -> &str { &self.model_id } fn cost_hint(&self) -> CostHint { CostHint::Network }
        }
    };
} net_embedder!(OllamaEmbedder, OllamaEmbeddingConfig, "ollama", embed_via_ollama); net_embedder!(CloudEmbedder, CloudEmbeddingConfig, "cloud", embed_via_api);

pub fn embedder_for(kind: EmbedBackendKind) -> Option<Box<dyn Embedder>> {
    match kind {
        EmbedBackendKind::Cloud => CloudEmbeddingConfig::from_env().map(|c| Box::new(CloudEmbedder::new(c)) as Box<dyn Embedder>),
        EmbedBackendKind::Ollama => OllamaEmbeddingConfig::from_env().map(|c| Box::new(OllamaEmbedder::new(c)) as Box<dyn Embedder>),
        EmbedBackendKind::Neural => neural_embedder(), EmbedBackendKind::Semantic => Some(Box::new(HashedEmbedder::default())),
    }
}

#[cfg(feature = "neural-embed")] impl Embedder for std::sync::Arc<crate::neural::NeuralEmbedder> {
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> { crate::neural::NeuralEmbedder::embed_batch(self, texts) } fn dim(&self) -> usize { crate::neural::NeuralEmbedder::dim(self) }
    fn model_id(&self) -> &str { crate::neural::NeuralEmbedder::model_id(self) } fn cost_hint(&self) -> CostHint { CostHint::LocalCompute }
}

#[cfg(feature = "neural-embed")] fn neural_embedder() -> Option<Box<dyn Embedder>> {
    use crate::neural::{NeuralEmbedder, NeuralEmbeddingConfig}; use std::collections::HashMap; use std::sync::{Arc, LazyLock, Mutex}; type Cache = HashMap<NeuralEmbeddingConfig, Option<Arc<NeuralEmbedder>>>;
    static INSTANCES: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(HashMap::new())); let config = NeuralEmbeddingConfig::configured(); let mut g = INSTANCES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(c) = g.get(&config) { return c.clone().map(|a| Box::new(a) as Box<dyn Embedder>); } let cached = match NeuralEmbedder::new(config.clone()) {
        Ok(e) => Some(Arc::new(e)), Err(err) => { eprintln!("asgrep: neural embedder unavailable, falling back: {err}"); None }
    }; g.insert(config, cached.clone()); cached.map(|a| Box::new(a) as Box<dyn Embedder>)
} #[cfg(not(feature = "neural-embed"))] fn neural_embedder() -> Option<Box<dyn Embedder>> { None }

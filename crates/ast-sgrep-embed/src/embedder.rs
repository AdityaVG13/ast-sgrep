#[cfg(feature = "cloud")] use serde::{Deserialize, Serialize};
use anyhow::{anyhow, Result};
// ---- remote cloud/ollama ----
#[derive(Debug, Clone)]
pub struct CloudEmbeddingConfig { pub api_url: String, pub api_key: String, pub model: String }
impl CloudEmbeddingConfig {
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("ASGREP_EMBED_API_KEY").ok()?;
        let api_url = std::env::var("ASGREP_EMBED_API_URL").unwrap_or_else(|_| "https://api.openai.com/v1/embeddings".to_string());
        let model = std::env::var("ASGREP_EMBED_MODEL").unwrap_or_else(|_| "text-embedding-3-small".to_string());
        Some(Self { api_url, api_key, model })
    }
}
#[derive(Serialize)] #[cfg(feature = "cloud")]
struct EmbedRequest<'a> { model: &'a str, input: &'a str }
#[derive(Deserialize)] #[cfg(feature = "cloud")]
struct EmbedResponse { data: Vec<EmbedData> }
#[derive(Deserialize)] #[cfg(feature = "cloud")]
struct EmbedData { embedding: Vec<f32> }
#[cfg(feature = "cloud")]
pub fn embed_via_api(text: &str, config: &CloudEmbeddingConfig) -> Result<Vec<f32>, String> {
    let body = EmbedRequest { model: &config.model, input: text }; let json = serde_json::to_string(&body).map_err(|e| e.to_string())?;
    let response = ureq::post(&config.api_url).set("Authorization", &format!("Bearer {}", config.api_key))
        .set("Content-Type", "application/json").send_string(&json).map_err(|e| e.to_string())?;
    let parsed: EmbedResponse = response.into_json().map_err(|e| e.to_string())?;
    parsed.data.into_iter().next().map(|d| d.embedding).ok_or_else(|| "empty embedding response".to_string())
}
#[cfg(not(feature = "cloud"))]
pub fn embed_via_api(_text: &str, _config: &CloudEmbeddingConfig) -> Result<Vec<f32>, String> { Err("cloud embedding feature not enabled; rebuild with --features cloud".to_string()) }
#[derive(Debug, Clone)]
pub struct OllamaEmbeddingConfig { pub api_url: String, pub model: String }
impl OllamaEmbeddingConfig {
    pub fn from_env() -> Option<Self> {
        if std::env::var("ASGREP_NO_OLLAMA").ok().as_deref() == Some("1") { return None; }
        let explicit = std::env::var("ASGREP_OLLAMA_EMBED").ok().as_deref() == Some("1"); let url_set = std::env::var("ASGREP_OLLAMA_URL").is_ok();
        if !explicit && !url_set { return None; }
        let api_url = std::env::var("ASGREP_OLLAMA_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
        let model = std::env::var("ASGREP_OLLAMA_MODEL").unwrap_or_else(|_| "nomic-embed-text".to_string()); Some(Self { api_url, model })
    }
    #[cfg(feature = "cloud")]
    fn embeddings_endpoint(&self) -> String { format!("{}/api/embeddings", self.api_url.trim_end_matches('/')) }
}
#[derive(Serialize)] #[cfg(feature = "cloud")]
struct OllamaEmbedRequest<'a> { model: &'a str, prompt: &'a str }
#[derive(Deserialize)] #[cfg(feature = "cloud")]
struct OllamaEmbedResponse { embedding: Vec<f32> }
#[cfg(feature = "cloud")]
pub fn embed_via_ollama(text: &str, config: &OllamaEmbeddingConfig) -> Result<Vec<f32>, String> {
    let body = OllamaEmbedRequest { model: &config.model, prompt: text }; let json = serde_json::to_string(&body).map_err(|e| e.to_string())?;
    let response = ureq::post(&config.embeddings_endpoint()).set("Content-Type", "application/json")
        .send_string(&json).map_err(|e| e.to_string())?;
    let parsed: OllamaEmbedResponse = response.into_json().map_err(|e| e.to_string())?;
    if parsed.embedding.is_empty() { return Err("empty ollama embedding response".to_string()); }
    Ok(parsed.embedding)
}
#[cfg(not(feature = "cloud"))]
pub fn embed_via_ollama(_text: &str, _config: &OllamaEmbeddingConfig) -> Result<Vec<f32>, String> { Err("ollama embedding requires cloud feature (ureq)".to_string()) }
use crate::semantic::{SemanticLocalEmbedding, SEMANTIC_DIM};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostHint { LocalCheap, LocalCompute, Network }
pub trait Embedder: Send + Sync {
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
    fn embed(&self, text: &str) -> Result<Vec<f32>> { let mut out = self.embed_batch(&[text])?; Ok(out.pop().unwrap_or_default()) }
    fn dim(&self) -> usize;
    fn model_id(&self) -> &str;
    fn cost_hint(&self) -> CostHint;
}
pub struct HashedEmbedder { inner: SemanticLocalEmbedding, model_id: String }
impl Default for HashedEmbedder { fn default() -> Self { Self { inner: SemanticLocalEmbedding, model_id: format!("hashed-{SEMANTIC_DIM}") } } }
impl Embedder for HashedEmbedder {
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> { Ok(texts.iter().map(|t| self.inner.embed_text(t)).collect()) }
    fn dim(&self) -> usize { SEMANTIC_DIM }
    fn model_id(&self) -> &str { &self.model_id }
    fn cost_hint(&self) -> CostHint { CostHint::LocalCheap }
}
pub struct OllamaEmbedder { config: OllamaEmbeddingConfig, model_id: String }
impl OllamaEmbedder { pub fn new(config: OllamaEmbeddingConfig) -> Self { let model_id = format!("ollama:{}", config.model); Self { config, model_id } } }
impl Embedder for OllamaEmbedder {
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> { texts.iter().map(|t| embed_via_ollama(t, &self.config).map_err(|e| anyhow!(e))).collect() }
    fn dim(&self) -> usize { 0 }
    fn model_id(&self) -> &str { &self.model_id }
    fn cost_hint(&self) -> CostHint { CostHint::Network }
}
pub struct CloudEmbedder { config: CloudEmbeddingConfig, model_id: String }
impl CloudEmbedder { pub fn new(config: CloudEmbeddingConfig) -> Self { let model_id = format!("cloud:{}", config.model); Self { config, model_id } } }
impl Embedder for CloudEmbedder {
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> { texts.iter().map(|t| embed_via_api(t, &self.config).map_err(|e| anyhow!(e))).collect() }
    fn dim(&self) -> usize { 0 }
    fn model_id(&self) -> &str { &self.model_id }
    fn cost_hint(&self) -> CostHint { CostHint::Network }
}
pub fn embedder_for(kind: EmbedBackendKind) -> Option<Box<dyn Embedder>> {
    match kind {
        EmbedBackendKind::Cloud => CloudEmbeddingConfig::from_env().map(|c| Box::new(CloudEmbedder::new(c)) as Box<dyn Embedder>),
        EmbedBackendKind::Ollama => OllamaEmbeddingConfig::from_env().map(|c| Box::new(OllamaEmbedder::new(c)) as Box<dyn Embedder>),
        EmbedBackendKind::Neural => neural_embedder(), EmbedBackendKind::Semantic => Some(Box::new(HashedEmbedder::default())), }
}
#[cfg(feature = "neural-embed")]
impl Embedder for std::sync::Arc<crate::neural::NeuralEmbedder> {
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> { crate::neural::NeuralEmbedder::embed_batch(self, texts) }
    fn dim(&self) -> usize { crate::neural::NeuralEmbedder::dim(self) }
    fn model_id(&self) -> &str { crate::neural::NeuralEmbedder::model_id(self) }
    fn cost_hint(&self) -> CostHint { CostHint::LocalCompute }
}
#[cfg(feature = "neural-embed")]
fn neural_embedder() -> Option<Box<dyn Embedder>> {
    use std::collections::HashMap; use std::sync::{Arc, LazyLock, Mutex};
    use crate::neural::{NeuralEmbedder, NeuralEmbeddingConfig}; type Cache = HashMap<NeuralEmbeddingConfig, Option<Arc<NeuralEmbedder>>>;
    static INSTANCES: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(HashMap::new())); let config = NeuralEmbeddingConfig::configured();
    let mut instances = INSTANCES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(cached) = instances.get(&config) { return cached.clone().map(|arc| Box::new(arc) as Box<dyn Embedder>); }
    let cached = match NeuralEmbedder::new(config.clone()) { Ok(embedder) => Some(Arc::new(embedder)), Err(err) => { eprintln!("asgrep: neural embedder unavailable, falling back: {err}"); None } };
    instances.insert(config, cached.clone()); cached.map(|arc| Box::new(arc) as Box<dyn Embedder>)
}
#[cfg(not(feature = "neural-embed"))]
fn neural_embedder() -> Option<Box<dyn Embedder>> { None }
// ---- provider chain ----
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedBackendKind { Cloud, Ollama, Neural, Semantic }
impl EmbedBackendKind {
    pub fn as_meta_str(self) -> &'static str { match self { Self::Cloud => "cloud", Self::Ollama => "ollama", Self::Neural => "neural", Self::Semantic => "semantic" } }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cloud" => Some(Self::Cloud), "ollama" => Some(Self::Ollama),
            "neural" | "fastembed" => Some(Self::Neural), "semantic" | "local" => Some(Self::Semantic), _ => None, }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmbedPreference { #[default] Auto, Cloud, Ollama, Neural, Semantic }
#[derive(Debug, Clone)]
pub struct EmbedResult { pub vector: Vec<f32>, pub backend: EmbedBackendKind }
pub fn embed_with_chain(text: &str, preference: EmbedPreference) -> EmbedResult {
    for kind in chain_kinds(preference) { if let Some(vector) = try_backend(kind, text) { return EmbedResult { vector, backend: kind }; } }
    EmbedResult {
        vector: try_backend(EmbedBackendKind::Semantic, text)
            .expect("local semantic embedder is always available and infallible"), backend: EmbedBackendKind::Semantic, }
}
pub fn embed_batch_with_chain(texts: &[&str], preference: EmbedPreference) -> Vec<EmbedResult> {
    if texts.is_empty() { return vec![]; }
    for kind in chain_kinds(preference) { if let Some(vectors) = try_backend_batch(kind, texts) { return vectors.into_iter().map(|v| EmbedResult { vector: v, backend: kind }).collect(); } }
    try_backend_batch(EmbedBackendKind::Semantic, texts)
        .expect("local semantic embedder is always available and infallible")
        .into_iter().map(|v| EmbedResult { vector: v, backend: EmbedBackendKind::Semantic }).collect()
}
fn chain_kinds(preference: EmbedPreference) -> Vec<EmbedBackendKind> {
    let mut kinds = Vec::new(); if matches!(preference, EmbedPreference::Cloud | EmbedPreference::Auto) { kinds.push(EmbedBackendKind::Cloud); }
    if matches!(preference, EmbedPreference::Cloud | EmbedPreference::Ollama | EmbedPreference::Auto) { kinds.push(EmbedBackendKind::Ollama); }
    if matches!(preference, EmbedPreference::Neural)
        || (matches!(preference, EmbedPreference::Auto) && crate::neural::NeuralEmbeddingConfig::from_env().is_some())
    { kinds.push(EmbedBackendKind::Neural); }
    kinds
}
pub fn embed_query( text: &str, stored_backend: Option<&str>, stored_dim: usize, preference: EmbedPreference, ) -> Result<EmbedResult, String> {
    if let Some(backend) = stored_backend.and_then(EmbedBackendKind::parse) {
        return match try_backend(backend, text).map(|vector| EmbedResult { vector, backend }) {
            Some(r) if stored_dim == 0 || r.vector.len() == stored_dim => Ok(r), Some(r) => Err(format!(
                "stored embedding backend '{}' (dim {}) does not match active backend '{}' (dim {}); reindex the store with 'asgrep index --force-reindex'",
                backend.as_meta_str(), stored_dim, pref_str(preference), r.vector.len()
            )),
            None => Err(format!( "stored embedding backend '{}' is not available; switch backends or reindex with 'asgrep index --force-reindex' using '{}'",
                backend.as_meta_str(), pref_str(preference)
            )), };
    }
    Ok(embed_with_chain(text, preference))
}
fn pref_str(p: EmbedPreference) -> &'static str {
    match p {
        EmbedPreference::Auto => "auto", EmbedPreference::Cloud => "cloud", EmbedPreference::Ollama => "ollama",
        EmbedPreference::Neural => "neural", EmbedPreference::Semantic => "semantic", }
}
fn try_backend(kind: EmbedBackendKind, text: &str) -> Option<Vec<f32>> { embedder_for(kind)?.embed(text).ok() }
fn try_backend_batch(kind: EmbedBackendKind, texts: &[&str]) -> Option<Vec<Vec<f32>>> { embedder_for(kind)?.embed_batch(texts).ok() }
pub fn default_semantic_dim() -> usize { SEMANTIC_DIM }

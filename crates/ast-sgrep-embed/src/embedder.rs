use anyhow::{anyhow, Result};
#[cfg(feature = "cloud")]
use serde::{Deserialize, Serialize};

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

/// Allowlist env-driven embed HTTP endpoints against SSRF (j0x4 / 2lbz / rl1p.7).
///
/// Default hosts: `api.openai.com`, `api.azure.com`, loopback for Ollama.
/// Extra hosts: comma-separated `ASGREP_EMBED_URL_ALLOWLIST`.
pub fn embed_url_is_allowed(url: &str) -> Result<(), String> {
    let url = url.trim();
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| "embed URL missing scheme".to_string())?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "https" && scheme != "http" {
        return Err(format!("embed URL scheme {scheme:?} is not allowed"));
    }
    let authority = rest
        .split(|c| c == '/' || c == '?' || c == '#')
        .next()
        .unwrap_or("");
    if authority.is_empty() {
        return Err("embed URL missing host".to_string());
    }
    // Strip userinfo and port: [user@]host[:port]
    let hostport = authority.rsplit('@').next().unwrap_or(authority);
    let host = if hostport.starts_with('[') {
        hostport
            .trim_start_matches('[')
            .split(']')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
    } else {
        hostport
            .split(':')
            .next()
            .unwrap_or(hostport)
            .to_ascii_lowercase()
    };
    if host.is_empty() {
        return Err("embed URL missing host".to_string());
    }
    let mut allowed = vec![
        "api.openai.com".to_string(),
        "api.azure.com".to_string(),
        "127.0.0.1".to_string(),
        "localhost".to_string(),
        "::1".to_string(),
    ];
    if let Ok(extra) = std::env::var("ASGREP_EMBED_URL_ALLOWLIST") {
        for part in extra.split(',') {
            let host = part.trim().to_ascii_lowercase();
            if !host.is_empty() {
                allowed.push(host);
            }
        }
    }
    if allowed.iter().any(|h| h == &host) {
        if scheme == "http"
            && !matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1")
            && !env_flag("ASGREP_EMBED_ALLOW_INSECURE_HTTP")
        {
            return Err(
                "http embed URLs are limited to loopback unless ASGREP_EMBED_ALLOW_INSECURE_HTTP=1"
                    .into(),
            );
        }
        return Ok(());
    }
    Err(format!(
        "embed URL host {host:?} is not allowlisted; set ASGREP_EMBED_URL_ALLOWLIST"
    ))
}

// ---- remote cloud/ollama ----
#[derive(Debug, Clone)]
pub struct CloudEmbeddingConfig {
    pub api_url: String,
    pub api_key: String,
    pub model: String,
}
impl CloudEmbeddingConfig {
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("ASGREP_EMBED_API_KEY").ok()?;
        let api_url = std::env::var("ASGREP_EMBED_API_URL")
            .unwrap_or_else(|_| "https://api.openai.com/v1/embeddings".to_string());
        embed_url_is_allowed(&api_url).ok()?;
        let model = std::env::var("ASGREP_EMBED_MODEL")
            .unwrap_or_else(|_| "text-embedding-3-small".to_string());
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
#[cfg(feature = "cloud")]
pub fn embed_via_api(text: &str, config: &CloudEmbeddingConfig) -> Result<Vec<f32>, String> {
    embed_url_is_allowed(&config.api_url)?;
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
#[derive(Debug, Clone)]
pub struct OllamaEmbeddingConfig {
    pub api_url: String,
    pub model: String,
}
impl OllamaEmbeddingConfig {
    pub fn from_env() -> Option<Self> {
        if env_flag("ASGREP_NO_OLLAMA") {
            return None;
        }
        let explicit = env_flag("ASGREP_OLLAMA_EMBED");
        let url_set = std::env::var("ASGREP_OLLAMA_URL").is_ok();
        if !explicit && !url_set {
            return None;
        }
        let api_url = std::env::var("ASGREP_OLLAMA_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
        embed_url_is_allowed(&api_url).ok()?;
        let model =
            std::env::var("ASGREP_OLLAMA_MODEL").unwrap_or_else(|_| "nomic-embed-text".to_string());
        Some(Self { api_url, model })
    }
    #[cfg(feature = "cloud")]
    fn embeddings_endpoint(&self) -> String {
        format!("{}/api/embeddings", self.api_url.trim_end_matches('/'))
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
#[cfg(feature = "cloud")]
pub fn embed_via_ollama(text: &str, config: &OllamaEmbeddingConfig) -> Result<Vec<f32>, String> {
    embed_url_is_allowed(&config.api_url)?;
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
use crate::semantic::{SemanticLocalEmbedding, SEMANTIC_DIM};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostHint {
    LocalCheap,
    LocalCompute,
    Network,
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
pub struct OllamaEmbedder {
    config: OllamaEmbeddingConfig,
    model_id: String,
    dim: std::sync::OnceLock<usize>,
    embed_one: fn(&str, &OllamaEmbeddingConfig) -> Result<Vec<f32>, String>,
}
impl OllamaEmbedder {
    pub fn new(config: OllamaEmbeddingConfig) -> Self {
        Self::with_embed_fn(config, embed_via_ollama)
    }
    /// Construct with a custom single-text embedder (used in tests to avoid network).
    pub fn with_embed_fn(
        config: OllamaEmbeddingConfig,
        embed_one: fn(&str, &OllamaEmbeddingConfig) -> Result<Vec<f32>, String>,
    ) -> Self {
        let model_id = format!("ollama:{}", config.model);
        Self {
            config,
            model_id,
            dim: std::sync::OnceLock::new(),
            embed_one,
        }
    }
}
impl Embedder for OllamaEmbedder {
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let out: Result<Vec<Vec<f32>>> = texts
            .iter()
            .map(|t| (self.embed_one)(t, &self.config).map_err(|e| anyhow!(e)))
            .collect();
        let out = out?;
        if let Some(first) = out.first().filter(|v| !v.is_empty()) {
            let _ = self.dim.set(first.len());
        }
        Ok(out)
    }
    fn dim(&self) -> usize {
        // 0 until the first successful embed probes and caches the true dim.
        self.dim.get().copied().unwrap_or(0)
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn cost_hint(&self) -> CostHint {
        CostHint::Network
    }
}
pub struct CloudEmbedder {
    config: CloudEmbeddingConfig,
    model_id: String,
    dim: std::sync::OnceLock<usize>,
    embed_one: fn(&str, &CloudEmbeddingConfig) -> Result<Vec<f32>, String>,
}
impl CloudEmbedder {
    pub fn new(config: CloudEmbeddingConfig) -> Self {
        Self::with_embed_fn(config, embed_via_api)
    }
    /// Construct with a custom single-text embedder (used in tests to avoid network).
    pub fn with_embed_fn(
        config: CloudEmbeddingConfig,
        embed_one: fn(&str, &CloudEmbeddingConfig) -> Result<Vec<f32>, String>,
    ) -> Self {
        let model_id = format!("cloud:{}", config.model);
        Self {
            config,
            model_id,
            dim: std::sync::OnceLock::new(),
            embed_one,
        }
    }
}
impl Embedder for CloudEmbedder {
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let out: Result<Vec<Vec<f32>>> = texts
            .iter()
            .map(|t| (self.embed_one)(t, &self.config).map_err(|e| anyhow!(e)))
            .collect();
        let out = out?;
        if let Some(first) = out.first().filter(|v| !v.is_empty()) {
            let _ = self.dim.set(first.len());
        }
        Ok(out)
    }
    fn dim(&self) -> usize {
        self.dim.get().copied().unwrap_or(0)
    }
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn cost_hint(&self) -> CostHint {
        CostHint::Network
    }
}
pub fn embedder_for(kind: EmbedBackendKind) -> Option<Box<dyn Embedder>> {
    match kind {
        EmbedBackendKind::Cloud => CloudEmbeddingConfig::from_env()
            .map(|c| Box::new(CloudEmbedder::new(c)) as Box<dyn Embedder>),
        EmbedBackendKind::Ollama => OllamaEmbeddingConfig::from_env()
            .map(|c| Box::new(OllamaEmbedder::new(c)) as Box<dyn Embedder>),
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
            // Fail closed unless the operator explicitly opts into hashed fallback (2058).
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
// ---- provider chain ----
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedBackendKind {
    Cloud,
    Ollama,
    Neural,
    Semantic,
}
impl EmbedBackendKind {
    pub fn as_meta_str(self) -> &'static str {
        match self {
            Self::Cloud => "cloud",
            Self::Ollama => "ollama",
            Self::Neural => "neural",
            Self::Semantic => "semantic-v2",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cloud" => Some(Self::Cloud),
            "ollama" => Some(Self::Ollama),
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
    Cloud,
    Ollama,
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
             refusing silent hashed swap — set ASGREP_NEURAL_FALLBACK=1 to acknowledge"
        );
    }
    if matches!(preference, EmbedPreference::Cloud | EmbedPreference::Ollama)
        && !env_allows_embed_fallback()
    {
        eprintln!(
            "asgrep: {preference:?} preference unavailable; refusing silent hashed Semantic \
             — set ASGREP_EMBED_FALLBACK=1 to acknowledge (embed_fallback)"
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
    if matches!(preference, EmbedPreference::Cloud | EmbedPreference::Ollama)
        && !env_allows_embed_fallback()
    {
        eprintln!(
            "asgrep: {preference:?} preference unavailable for batch; refusing silent hashed Semantic \
             — set ASGREP_EMBED_FALLBACK=1 to acknowledge (embed_fallback)"
        );
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
    // Cloud preference must not silently try Ollama (9gfx).
    match preference {
        EmbedPreference::Cloud => vec![EmbedBackendKind::Cloud],
        EmbedPreference::Ollama => vec![EmbedBackendKind::Ollama],
        EmbedPreference::Neural => vec![EmbedBackendKind::Neural],
        EmbedPreference::Semantic => vec![],
        EmbedPreference::Auto => {
            let mut kinds = vec![EmbedBackendKind::Cloud, EmbedBackendKind::Ollama];
            if crate::neural::NeuralEmbeddingConfig::from_env().is_some() {
                kinds.push(EmbedBackendKind::Neural);
            }
            kinds
        }
    }
}
fn env_allows_embed_fallback() -> bool {
    env_flag("ASGREP_EMBED_FALLBACK")
}
pub fn embed_query(
    text: &str,
    stored_backend: Option<&str>,
    stored_dim: usize,
    preference: EmbedPreference,
) -> Result<EmbedResult, String> {
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
        EmbedPreference::Auto => "auto",
        EmbedPreference::Cloud => "cloud",
        EmbedPreference::Ollama => "ollama",
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
        EmbedBackendKind::Semantic => Some(format!("semantic:hashed-v1:{dim}")),
        EmbedBackendKind::Neural => {
            Some(format!("neural:{}", crate::neural::configured_model_id()))
        }
        EmbedBackendKind::Cloud => {
            CloudEmbeddingConfig::from_env().map(|config| format!("cloud:{}", config.model))
        }
        EmbedBackendKind::Ollama => {
            OllamaEmbeddingConfig::from_env().map(|config| format!("ollama:{}", config.model))
        }
    }
}

pub fn default_semantic_dim() -> usize {
    SEMANTIC_DIM
}

#[cfg(test)]
mod dim_probe_tests {
    use super::*;

    #[test]
    fn embed_url_allowlist_blocks_ssrf_targets() {
        assert!(embed_url_is_allowed("https://api.openai.com/v1/embeddings").is_ok());
        assert!(embed_url_is_allowed("http://127.0.0.1:11434/api/embeddings").is_ok());
        assert!(embed_url_is_allowed("http://169.254.169.254/latest/meta-data").is_err());
        assert!(embed_url_is_allowed("https://evil.example/exfil").is_err());
        assert!(embed_url_is_allowed("file:///etc/passwd").is_err());
    }

    fn stub_ollama(_text: &str, _cfg: &OllamaEmbeddingConfig) -> Result<Vec<f32>, String> {
        Ok(vec![0.25; 384])
    }

    fn stub_cloud(_text: &str, _cfg: &CloudEmbeddingConfig) -> Result<Vec<f32>, String> {
        Ok(vec![0.5; 1536])
    }

    #[test]
    fn ollama_dim_matches_embedded_vector_after_probe() {
        let embedder = OllamaEmbedder::with_embed_fn(
            OllamaEmbeddingConfig {
                api_url: "http://127.0.0.1:9".into(),
                model: "stub".into(),
            },
            stub_ollama,
        );
        assert_eq!(embedder.dim(), 0, "dim is 0 before first embed");
        let vector = Embedder::embed(&embedder, "hello").unwrap();
        assert_eq!(embedder.dim(), vector.len());
        assert_eq!(embedder.dim(), 384);
    }

    #[test]
    fn cloud_dim_matches_embedded_vector_after_probe() {
        let embedder = CloudEmbedder::with_embed_fn(
            CloudEmbeddingConfig {
                api_url: "http://127.0.0.1:9".into(),
                api_key: "test".into(),
                model: "stub".into(),
            },
            stub_cloud,
        );
        assert_eq!(embedder.dim(), 0, "dim is 0 before first embed");
        let vector = Embedder::embed(&embedder, "hello").unwrap();
        assert_eq!(embedder.dim(), vector.len());
        assert_eq!(embedder.dim(), 1536);
    }
}

#[cfg(test)]
mod preference_tests {
    use super::*;
    #[test]
    fn cloud_preference_excludes_ollama() {
        let kinds = chain_kinds(EmbedPreference::Cloud);
        assert_eq!(kinds, vec![EmbedBackendKind::Cloud]);
        assert!(!kinds.contains(&EmbedBackendKind::Ollama));
        assert!(!kinds.contains(&EmbedBackendKind::Semantic));
    }
    #[test]
    fn auto_may_include_cloud_and_ollama() {
        let kinds = chain_kinds(EmbedPreference::Auto);
        assert!(kinds.contains(&EmbedBackendKind::Cloud));
        assert!(kinds.contains(&EmbedBackendKind::Ollama));
    }
}

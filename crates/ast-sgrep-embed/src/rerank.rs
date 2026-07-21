use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use anyhow::{anyhow, Context, Result};
use fastembed::{RerankInitOptions, RerankerModel, TextRerank};
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RerankScore { pub index: usize, pub score: f32 }
#[derive(Debug, Clone, PartialEq, Eq)]
struct RerankConfig { model: RerankerModel, cache_dir: PathBuf, max_length: usize, intra_threads: usize }
impl std::hash::Hash for RerankConfig {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.model.to_string().hash(state); self.cache_dir.hash(state); self.max_length.hash(state); self.intra_threads.hash(state);
    }
}
impl RerankConfig {
    fn from_env() -> Result<Self> {
        Ok(Self {
            model: resolve_model(std::env::var("ASGREP_RERANK_MODEL").ok().as_deref())?,
            cache_dir: std::env::var_os("ASGREP_RERANK_CACHE_DIR").filter(|v| !v.is_empty())
                .map(PathBuf::from).unwrap_or_else(crate::neural_default_cache_dir), max_length: env_usize("ASGREP_RERANK_MAX_LENGTH", 128),
            intra_threads: env_usize("ASGREP_RERANK_INTRA_THREADS", 2), })
    }
}
fn env_usize(name: &str, default: usize) -> usize { std::env::var(name).ok().and_then(|v| v.parse().ok()).filter(|v| *v > 0).unwrap_or(default) }
fn resolve_model(value: Option<&str>) -> Result<RerankerModel> {
    match value.map(|v| v.trim()) {
        None | Some("") => Ok(RerankerModel::JINARerankerV1TurboEn), Some(t) => match t.to_ascii_lowercase().as_str() {
            "jina-v1-turbo-en" => Ok(RerankerModel::JINARerankerV1TurboEn), "bge-reranker-base" => Ok(RerankerModel::BGERerankerBase),
            "bge-reranker-v2-m3" => Ok(RerankerModel::BGERerankerV2M3), "jina-v2-base-multilingual" => Ok(RerankerModel::JINARerankerV2BaseMultiligual),
            _ => Err(anyhow!( "unknown reranker model {:?}. Accepted aliases: \
                 bge-reranker-base, bge-reranker-v2-m3, jina-v1-turbo-en, \
                 jina-v2-base-multilingual (CC-BY-NC-4.0, non-commercial)", t
            )), }, }
}
type SharedReranker = Arc<Mutex<TextRerank>>;
type CacheEntry = std::result::Result<SharedReranker, String>;
static RERANKERS: LazyLock<Mutex<HashMap<RerankConfig, CacheEntry>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
fn load(config: &RerankConfig) -> Result<SharedReranker> {
    let mut cache = RERANKERS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(entry) = cache.get(config) { return entry.clone().map_err(anyhow::Error::msg); }
    std::fs::create_dir_all(&config.cache_dir)
        .with_context(|| format!("creating reranker cache dir {:?}", config.cache_dir))?;
    let options = RerankInitOptions::new(config.model.clone())
        .with_cache_dir(config.cache_dir.clone()).with_show_download_progress(false)
        .with_max_length(config.max_length).with_intra_threads(config.intra_threads);
    let loaded = TextRerank::try_new(options)
        .map(|model| Arc::new(Mutex::new(model)))
        .map_err(|error| format!("failed to load local reranker: {error}"));
    cache.insert(config.clone(), loaded.clone()); loaded.map_err(anyhow::Error::msg)
}
pub fn rerank(query: &str, documents: &[String]) -> Result<Vec<RerankScore>> {
    if documents.is_empty() { return Ok(vec![]); }
    let model = load(&RerankConfig::from_env()?)?; let batch_size = env_usize("ASGREP_RERANK_BATCH_SIZE", 1).min(documents.len());
    let mut model = model.lock().map_err(|_| anyhow!("local reranker mutex poisoned"))?; let mut scores = Vec::with_capacity(documents.len());
    for (batch_index, batch) in documents.chunks(batch_size).enumerate() {
        let refs = batch.iter().map(String::as_str).collect::<Vec<_>>(); let results = model.rerank(query, &refs, false, Some(batch.len()))
            .map_err(|error| anyhow!("local rerank failed: {error}"))?;
        let offset = batch_index * batch_size; scores.extend(results.into_iter().map(|r| RerankScore { index: offset + r.index, score: r.score }));
    }
    Ok(scores)
}

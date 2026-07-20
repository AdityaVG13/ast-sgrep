//! Local cross-encoder rerank (feature `rerank`).

#[cfg(feature = "rerank")] mod real {
    use anyhow::{anyhow, Context, Result}; use fastembed::{RerankInitOptions, RerankerModel, TextRerank}; use std::collections::HashMap; use std::path::PathBuf; use std::sync::{Arc, LazyLock, Mutex};

    #[derive(Debug, Clone, Copy, PartialEq)] pub struct RerankScore {
        pub index: usize, pub score: f32,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Hash)] struct RerankConfig {
        model: String, cache_dir: PathBuf, max_length: usize, intra_threads: usize,
    }

    impl RerankConfig {
        fn from_env() -> Result<(Self, RerankerModel)> {
            let (key, model) = resolve_model(std::env::var("ASGREP_RERANK_MODEL").ok().as_deref())?; Ok((
                Self {
                    model: key.into(), cache_dir: std::env::var_os("ASGREP_RERANK_CACHE_DIR")
                        .filter(|v| !v.is_empty()) .map(PathBuf::from) .unwrap_or_else(crate::neural_default_cache_dir),
                    max_length: env_usize("ASGREP_RERANK_MAX_LENGTH", 128), intra_threads: env_usize("ASGREP_RERANK_INTRA_THREADS", 2),
                }, model,
            ))
        }
    }

    fn env_usize(name: &str, default: usize) -> usize {
        std::env::var(name)
            .ok() .and_then(|v| v.parse().ok()) .filter(|v| *v > 0) .unwrap_or(default)
    }

    fn resolve_model(value: Option<&str>) -> Result<(&'static str, RerankerModel)> {
        match value.map(|v| v.trim()).unwrap_or("") {
            "" | "jina-v1-turbo-en" => Ok(("jina-v1-turbo-en", RerankerModel::JINARerankerV1TurboEn)), "bge-reranker-base" => Ok(("bge-reranker-base", RerankerModel::BGERerankerBase)),
            "bge-reranker-v2-m3" => Ok(("bge-reranker-v2-m3", RerankerModel::BGERerankerV2M3)), "jina-v2-base-multilingual" => {
                Ok(("jina-v2-base-multilingual", RerankerModel::JINARerankerV2BaseMultiligual))
            } t => Err(anyhow!("unknown reranker model {t:?}")),
        }
    }

    type Shared = Arc<Mutex<TextRerank>>; static CACHE: LazyLock<Mutex<HashMap<RerankConfig, Result<Shared, String>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    fn load() -> Result<Shared> {
        let (cfg, model) = RerankConfig::from_env()?; let mut cache = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(e) = cache.get(&cfg) { return e.clone().map_err(anyhow::Error::msg); } std::fs::create_dir_all(&cfg.cache_dir)
            .with_context(|| format!("creating reranker cache dir {:?}", cfg.cache_dir))?;
        let loaded = TextRerank::try_new(
            RerankInitOptions::new(model)
                .with_cache_dir(cfg.cache_dir.clone()) .with_show_download_progress(false) .with_max_length(cfg.max_length) .with_intra_threads(cfg.intra_threads),
        ) .map(|m| Arc::new(Mutex::new(m))) .map_err(|e| format!("failed to load local reranker: {e}")); cache.insert(cfg, loaded.clone()); loaded.map_err(anyhow::Error::msg)
    }

    pub fn rerank(query: &str, documents: &[String]) -> Result<Vec<RerankScore>> {
        if documents.is_empty() { return Ok(vec![]); } let batch_size = env_usize("ASGREP_RERANK_BATCH_SIZE", 1).min(documents.len()); let mut model = load()?
            .lock() .map_err(|_| anyhow!("local reranker mutex poisoned"))?;
        let mut scores = Vec::with_capacity(documents.len()); for (bi, batch) in documents.chunks(batch_size).enumerate() {
            let refs: Vec<_> = batch.iter().map(String::as_str).collect(); let results = model
                .rerank(query, &refs, false, Some(batch.len())) .map_err(|e| anyhow!("local rerank failed: {e}"))?;
            let off = bi * batch_size; scores.extend(results.into_iter().map(|r| RerankScore {
                index: off + r.index, score: r.score,
            }));
        } Ok(scores)
    }
}

#[cfg(feature = "rerank")] pub use real::{rerank, RerankScore};

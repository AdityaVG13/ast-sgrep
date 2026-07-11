use std::path::PathBuf;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NeuralModel {
    AllMiniLmL6V2,
    AllMiniLmL6V2Q,
    BgeSmallEnV15,
}
impl NeuralModel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AllMiniLmL6V2 => "all-minilm-l6-v2",
            Self::AllMiniLmL6V2Q => "all-minilm-l6-v2-q",
            Self::BgeSmallEnV15 => "bge-small-en-v1.5",
        }
    }

    pub fn dim(self) -> usize {
        384
    }

    fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "all-minilm-l6-v2" | "minilm" | "all-minilm" | "all-minilm-l6" => Some(Self::AllMiniLmL6V2),
            "all-minilm-l6-v2-q" | "minilm-q" => Some(Self::AllMiniLmL6V2Q),
            "bge-small-en-v1.5" | "bge-small" | "bge" => Some(Self::BgeSmallEnV15),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NeuralEmbeddingConfig {
    pub model: NeuralModel,
    pub cache_dir: PathBuf,
    pub intra_threads: usize,
    pub coreml: bool,
}
impl NeuralEmbeddingConfig {
    pub fn from_env() -> Option<Self> {
        (std::env::var("ASGREP_NEURAL_EMBED").ok().as_deref() == Some("1")).then(Self::configured)
    }

    pub(crate) fn configured() -> Self {
        Self {
            model: std::env::var("ASGREP_NEURAL_MODEL")
                .ok()
                .and_then(|s| NeuralModel::parse(&s))
                .unwrap_or(NeuralModel::AllMiniLmL6V2Q),
            cache_dir: std::env::var("ASGREP_NEURAL_CACHE_DIR")
                .ok()
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(default_cache_dir),
            intra_threads: std::env::var("ASGREP_NEURAL_INTRA_THREADS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(2),
            coreml: std::env::var("ASGREP_NEURAL_COREML").ok().as_deref() == Some("1"),
        }
    }
}
pub fn configured_model_id() -> &'static str {
    NeuralEmbeddingConfig::configured().model.as_str()
}
pub fn default_cache_dir() -> PathBuf {
    cache_home().join("ast-sgrep").join("models")
}
fn cache_home() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        if !xdg.is_empty() { return PathBuf::from(xdg); }
    }
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join(".cache"))
        .unwrap_or_else(|_| PathBuf::from(".cache"))
}
#[cfg(feature = "neural-embed")]
pub use real::NeuralEmbedder;
#[cfg(feature = "neural-embed")]
mod real {
    use std::sync::Mutex;
    use anyhow::{anyhow, Context, Result};
    use fastembed::{EmbeddingModel, ExecutionProviderDispatch, InitOptions, TextEmbedding};
    use super::{NeuralEmbeddingConfig, NeuralModel};

    pub struct NeuralEmbedder {
        model: Mutex<TextEmbedding>,
        dim: usize,
        model_id: String,
    }

    impl NeuralEmbedder {
        pub fn new(config: NeuralEmbeddingConfig) -> Result<Self> {
            std::fs::create_dir_all(&config.cache_dir)
                .with_context(|| format!("creating neural model cache dir {:?}", config.cache_dir))?;
            let fastembed_model = match config.model {
                NeuralModel::AllMiniLmL6V2 => EmbeddingModel::AllMiniLML6V2,
                NeuralModel::AllMiniLmL6V2Q => EmbeddingModel::AllMiniLML6V2Q,
                NeuralModel::BgeSmallEnV15 => EmbeddingModel::BGESmallENV15,
            };
            let options = InitOptions::new(fastembed_model)
                .with_cache_dir(config.cache_dir.clone())
                .with_execution_providers(execution_providers(config.coreml))
                .with_show_download_progress(false)
                .with_max_length(32)
                .with_intra_threads(config.intra_threads);
            let model = TextEmbedding::try_new(options)
                .map_err(|e| anyhow!("failed to load neural embedding model: {e}"))?;
            Ok(Self {
                model: Mutex::new(model),
                dim: config.model.dim(),
                model_id: format!("fastembed:{}", config.model.as_str()),
            })
        }

        pub fn dim(&self) -> usize {
            self.dim
        }
        pub fn model_id(&self) -> &str {
            &self.model_id
        }
        pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
            self.model
                .lock()
                .map_err(|_| anyhow!("neural embedder mutex poisoned"))?
                .embed(texts, None)
                .map_err(|e| anyhow!("neural embed_batch failed: {e}"))
        }
    }

    #[cfg(target_os = "macos")]
    fn execution_providers(coreml_requested: bool) -> Vec<ExecutionProviderDispatch> {
        if coreml_requested {
            vec![ort::ep::CoreML::default().build()]
        } else {
            vec![]
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn execution_providers(_: bool) -> Vec<ExecutionProviderDispatch> {
        vec![]
    }
}

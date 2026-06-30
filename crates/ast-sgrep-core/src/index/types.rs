use std::path::PathBuf;

/// Embedding backend preference for symbol-level semantic chunks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EmbedBackend {
    /// Cloud API → Ollama → semantic local (default).
    #[default]
    Auto,
    Cloud,
    Ollama,
    /// Offline code-aware semantic embeddings only.
    Semantic,
}

impl EmbedBackend {
    pub fn to_preference(self) -> ast_sgrep_embed::EmbedPreference {
        match self {
            EmbedBackend::Auto => ast_sgrep_embed::EmbedPreference::Auto,
            EmbedBackend::Cloud => ast_sgrep_embed::EmbedPreference::Cloud,
            EmbedBackend::Ollama => ast_sgrep_embed::EmbedPreference::Ollama,
            EmbedBackend::Semantic => ast_sgrep_embed::EmbedPreference::Semantic,
        }
    }

    /// Parse backend name from config strings (`cloud`, `ollama`, `semantic`, `local`).
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "cloud" => EmbedBackend::Cloud,
            "ollama" => EmbedBackend::Ollama,
            "semantic" | "local" => EmbedBackend::Semantic,
            _ => EmbedBackend::Auto,
        }
    }

    /// Map CLI/LSP boolean flags to a single backend preference.
    pub fn from_flags(cloud: bool, ollama: bool, semantic_only: bool) -> Self {
        if cloud {
            EmbedBackend::Cloud
        } else if ollama {
            EmbedBackend::Ollama
        } else if semantic_only {
            EmbedBackend::Semantic
        } else {
            EmbedBackend::Auto
        }
    }
}

/// Options for indexing a repository.
#[derive(Debug, Clone)]
pub struct IndexOptions {
    pub root: PathBuf,
    pub index_path: Option<PathBuf>,
    pub lang_filter: Option<String>,
    pub respect_gitignore: bool,
    pub use_tantivy: bool,
    /// Index symbol-level semantic chunks (default on).
    pub embed_semantic: bool,
    pub embed_backend: EmbedBackend,
    pub force_reindex: bool,
    /// Override ANN threshold (`ASGREP_ANN_THRESHOLD` when unset).
    pub ann_threshold: Option<usize>,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            root: PathBuf::from("."),
            index_path: None,
            lang_filter: None,
            respect_gitignore: true,
            use_tantivy: false,
            embed_semantic: true,
            embed_backend: EmbedBackend::Auto,
            force_reindex: false,
            ann_threshold: None,
        }
    }
}

/// Statistics from an indexing run.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct IndexStats {
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub files_removed: usize,
    pub files_failed: usize,
    pub walk_errors: bool,
    pub symbols_extracted: usize,
    pub callers_extracted: usize,
    pub imports_extracted: usize,
}

/// Per-file indexing outcome.
#[derive(Debug, Clone, Copy, Default)]
pub struct FileIndexStats {
    pub symbols: usize,
    pub callers: usize,
    pub imports: usize,
    pub skipped: bool,
}

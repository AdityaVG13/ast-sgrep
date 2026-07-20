use ast_sgrep_core::{EmbedBackend, IndexOptions, SearchOptions}; use serde::Deserialize; use serde_json::Value;

#[derive(Debug, Clone, Default, Deserialize)] #[serde(rename_all = "camelCase")] pub struct AsgrepSettings {
    pub no_embed: Option<bool>, pub cloud_embed: Option<bool>, pub ollama_embed: Option<bool>, pub semantic_only: Option<bool>, pub ann_threshold: Option<usize>, pub embed_backend: Option<String>, pub index_path: Option<String>,
}

impl AsgrepSettings {
    pub fn from_initialization_options(value: &Value) -> Self {
        value
            .get("asgrep") .and_then(|n| serde_json::from_value(n.clone()).ok()) .or_else(|| serde_json::from_value(value.clone()).ok()) .unwrap_or_default()
    }

    pub fn apply_to_index_options(&self, opts: &mut IndexOptions) {
        if let Some(no) = self.no_embed { opts.embed_semantic = !no; } if let Some(ref b) = self.embed_backend { opts.embed_backend = EmbedBackend::parse(b); } if self.cloud_embed == Some(true) { opts.embed_backend = EmbedBackend::Cloud; }
        if self.ollama_embed == Some(true) { opts.embed_backend = EmbedBackend::Ollama; } if self.semantic_only == Some(true) { opts.embed_backend = EmbedBackend::Semantic; }
        if let Some(t) = self.ann_threshold { opts.ann_threshold = Some(t); } if let Some(ref p) = self.index_path { opts.index_path = Some(std::path::PathBuf::from(p)); }
    }

    pub fn apply_to_search_options(&self, opts: &mut SearchOptions) {
        if let Some(no) = self.no_embed { opts.use_embed = !no; } if let Some(c) = self.cloud_embed { opts.use_cloud_embed = c; } if let Some(o) = self.ollama_embed { opts.use_ollama_embed = o; }
        if let Some(s) = self.semantic_only { opts.use_semantic_only = s; } if let Some(t) = self.ann_threshold { opts.ann_threshold = Some(t); } if let Some(ref p) = self.index_path { opts.index_path = Some(std::path::PathBuf::from(p)); }
    }
}

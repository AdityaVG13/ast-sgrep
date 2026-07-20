use crate::embedder::embedder_for; use crate::semantic::SEMANTIC_DIM;

#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum EmbedBackendKind { Cloud, Ollama, Neural, Semantic } impl EmbedBackendKind {
    pub fn as_meta_str(self) -> &'static str {
        match self { Self::Cloud => "cloud", Self::Ollama => "ollama", Self::Neural => "neural", Self::Semantic => "semantic" }
    } pub fn parse(s: &str) -> Option<Self> {
        match s {
            "cloud" => Some(Self::Cloud), "ollama" => Some(Self::Ollama), "neural" | "fastembed" => Some(Self::Neural), "semantic" | "local" => Some(Self::Semantic), _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)] pub enum EmbedPreference { #[default] Auto, Cloud, Ollama, Neural, Semantic }

#[derive(Debug, Clone)] pub struct EmbedResult { pub vector: Vec<f32>, pub backend: EmbedBackendKind }

pub fn embed_with_chain(text: &str, preference: EmbedPreference) -> EmbedResult {
    for kind in chain_kinds(preference) {
        if let Some(vector) = try_backend(kind, text) { return EmbedResult { vector, backend: kind }; }
    } EmbedResult {
        vector: try_backend(EmbedBackendKind::Semantic, text).expect("local semantic embedder is always available and infallible"), backend: EmbedBackendKind::Semantic,
    }
}

pub fn embed_batch_with_chain(texts: &[&str], preference: EmbedPreference) -> Vec<EmbedResult> {
    if texts.is_empty() { return vec![]; } for kind in chain_kinds(preference) {
        if let Some(vectors) = try_backend_batch(kind, texts) {
            return vectors.into_iter().map(|v| EmbedResult { vector: v, backend: kind }).collect();
        }
    } try_backend_batch(EmbedBackendKind::Semantic, texts)
        .expect("local semantic embedder is always available and infallible") .into_iter().map(|v| EmbedResult { vector: v, backend: EmbedBackendKind::Semantic }).collect()
}

fn chain_kinds(preference: EmbedPreference) -> Vec<EmbedBackendKind> {
    let mut kinds = Vec::new(); if matches!(preference, EmbedPreference::Cloud | EmbedPreference::Auto) { kinds.push(EmbedBackendKind::Cloud); }
    if matches!(preference, EmbedPreference::Cloud | EmbedPreference::Ollama | EmbedPreference::Auto) {
        kinds.push(EmbedBackendKind::Ollama);
    } if matches!(preference, EmbedPreference::Neural)
        || (matches!(preference, EmbedPreference::Auto) && crate::neural::NeuralEmbeddingConfig::from_env().is_some())
    {
        kinds.push(EmbedBackendKind::Neural);
    } kinds
}

pub fn embed_query(
    text: &str, stored_backend: Option<&str>, stored_dim: usize, preference: EmbedPreference,
) -> Result<EmbedResult, String> {
    if let Some(backend) = stored_backend.and_then(EmbedBackendKind::parse) {
        return match try_backend(backend, text).map(|vector| EmbedResult { vector, backend }) {
            Some(r) if stored_dim == 0 || r.vector.len() == stored_dim => Ok(r), Some(r) => Err(format!(
                "stored embedding backend '{}' (dim {}) does not match active backend '{}' (dim {}); reindex the store with 'asgrep index --force-reindex'", backend.as_meta_str(), stored_dim, preference_str(preference), r.vector.len()
            )), None => Err(format!(
                "stored embedding backend '{}' is not available; switch backends or reindex with 'asgrep index --force-reindex' using '{}'", backend.as_meta_str(), preference_str(preference)
            )),
        };
    } Ok(embed_with_chain(text, preference))
}

fn preference_str(p: EmbedPreference) -> &'static str {
    match p {
        EmbedPreference::Auto => "auto", EmbedPreference::Cloud => "cloud", EmbedPreference::Ollama => "ollama", EmbedPreference::Neural => "neural", EmbedPreference::Semantic => "semantic",
    }
} fn try_backend(kind: EmbedBackendKind, text: &str) -> Option<Vec<f32>> { embedder_for(kind)?.embed(text).ok() }
fn try_backend_batch(kind: EmbedBackendKind, texts: &[&str]) -> Option<Vec<Vec<f32>>> { embedder_for(kind)?.embed_batch(texts).ok() } pub fn default_semantic_dim() -> usize { SEMANTIC_DIM }

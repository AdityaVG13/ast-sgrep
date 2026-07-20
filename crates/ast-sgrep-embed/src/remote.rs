//! Hollowed remote embedding backends (cloud + ollama). Config is parsed; HTTP always fails.

#[derive(Debug, Clone)] pub struct CloudEmbeddingConfig {
    pub api_url: String, pub api_key: String, pub model: String,
} impl CloudEmbeddingConfig {
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("ASGREP_EMBED_API_KEY").ok()?; Some(Self {
            api_url: std::env::var("ASGREP_EMBED_API_URL").unwrap_or_else(|_| "https://api.openai.com/v1/embeddings".into()), api_key, model: std::env::var("ASGREP_EMBED_MODEL").unwrap_or_else(|_| "text-embedding-3-small".into()),
        })
    }
} pub fn embed_via_api(_text: &str, _config: &CloudEmbeddingConfig) -> Result<Vec<f32>, String> {
    Err("cloud embedding unavailable (HTTP client hollowed)".into())
}

#[derive(Debug, Clone)] pub struct OllamaEmbeddingConfig {
    pub api_url: String, pub model: String,
} impl OllamaEmbeddingConfig {
    pub fn from_env() -> Option<Self> {
        if std::env::var("ASGREP_NO_OLLAMA").ok().as_deref() == Some("1") { return None; } let explicit = std::env::var("ASGREP_OLLAMA_EMBED").ok().as_deref() == Some("1"); let url_set = std::env::var("ASGREP_OLLAMA_URL").is_ok();
        if !explicit && !url_set { return None; } Some(Self {
            api_url: std::env::var("ASGREP_OLLAMA_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".into()), model: std::env::var("ASGREP_OLLAMA_MODEL").unwrap_or_else(|_| "nomic-embed-text".into()),
        })
    }
} pub fn embed_via_ollama(_text: &str, _config: &OllamaEmbeddingConfig) -> Result<Vec<f32>, String> {
    Err("ollama embedding unavailable (HTTP client hollowed)".into())
}

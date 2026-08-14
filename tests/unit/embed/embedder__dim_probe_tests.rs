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

#[cfg(feature = "cloud")]
#[test]
fn embed_http_agent_disables_redirects() {
    // Policy pin: allowlist is hop-final. ureq default is redirects=5.
    let agent = embed_http_agent();
    let rendered = format!("{agent:?}");
    assert!(
        rendered.contains("redirects: 0") || rendered.contains("redirects:0"),
        "embed agent must disable redirects so allowlist is final hop: {rendered}"
    );
    assert!(
        rendered.contains("timeout_read: Some("),
        "embed agent must set timeout_read (ureq default is None): {rendered}"
    );
    assert!(
        !rendered.contains("timeout_read: None"),
        "embed agent must not leave timeout_read unset: {rendered}"
    );
    assert!(
        rendered.contains("timeout: Some("),
        "embed agent must set overall timeout (ureq default is None): {rendered}"
    );
}

#[test]
fn cloud_config_debug_redacts_api_key() {
    let cfg = CloudEmbeddingConfig {
        api_url: "https://api.openai.com/v1/embeddings".into(),
        api_key: "sk-live-super-secret-value".into(),
        model: "text-embedding-3-small".into(),
    };
    let rendered = format!("{cfg:?}");
    assert!(
        rendered.contains("<redacted>"),
        "expected redaction marker: {rendered}"
    );
    assert!(
        !rendered.contains("sk-live-super-secret-value"),
        "api_key must not appear in Debug: {rendered}"
    );
    assert!(rendered.contains("text-embedding-3-small"));
}

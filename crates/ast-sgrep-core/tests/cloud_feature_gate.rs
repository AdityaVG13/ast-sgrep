//! ls6.3: cloud embed feature is wired through core (`cloud-embed` → embed/cloud).
//! Building CLI with `--no-default-features` disables cloud HTTP unless re-enabled.

#[test]
fn cloud_embed_feature_is_gated() {
    #[cfg(feature = "cloud-embed")]
    {
        // Default workspace build exposes cloud helpers.
        let _ = ast_sgrep_embed::CloudEmbeddingConfig::from_env();
    }
    #[cfg(not(feature = "cloud-embed"))]
    {
        // Without the feature, API embed returns the feature-disabled error path.
        let cfg = ast_sgrep_embed::CloudEmbeddingConfig {
            api_url: "https://api.openai.com/v1/embeddings".into(),
            api_key: "x".into(),
            model: "text-embedding-3-small".into(),
        };
        let err = ast_sgrep_embed::embed_via_api("hi", &cfg).unwrap_err();
        assert!(
            err.contains("cloud") || err.contains("feature"),
            "{err}"
        );
    }
}

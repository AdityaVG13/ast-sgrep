use super::*;

#[test]
fn hashed_embedder_dim_is_known_at_construction() {
    let embedder = HashedEmbedder::default();
    assert_eq!(embedder.dim(), SEMANTIC_DIM);
    let vector = Embedder::embed(&embedder, "hello").unwrap();
    assert_eq!(embedder.dim(), vector.len());
    assert_eq!(vector.len(), SEMANTIC_DIM);
}

#[test]
fn stored_http_backends_hard_error_on_query() {
    for stored in ["cloud", "ollama"] {
        let err = embed_query("q", Some(stored), 384, EmbedPreference::Auto).unwrap_err();
        assert!(
            err.contains("HTTP provider") && err.contains("reindex"),
            "{err}"
        );
    }
}

//! T1 topology pass 1: default-feature-surface inventory for ast-sgrep-core.
//!
//! Core's `[features] default = []`; `neural-embed` and `rerank` only forward
//! to ast-sgrep-embed. These tests pin DEFAULT-build behavior of that optional
//! surface: the local hashed embed path (deterministic, offline), rerank-off
//! search, and fail-closed neural/rerank gates.
//!
//! Every test is meaningful under multiple feature sets: `cfg!`-branched
//! expectations where the behavior is pure, `#[cfg]`-gated variants where the
//! heavy-feature side would touch the network (model downloads — never run the
//! heavy-feature test binaries; check-only is the gate there).
//! Offline, deterministic, tempfile fixtures, no new deps. Discriminant/value
//! assertions only — never message text.

use ast_sgrep_core::search::validate_search_feature_flags;
use ast_sgrep_core::{EmbedBackend, IndexOptions, Indexer, SearchOptions, Searcher, StoreError};
use ast_sgrep_embed::{
    default_semantic_dim, embed_query, embed_with_chain, embedder_for, neural_configured_model_id,
    neural_default_cache_dir, CostHint, EmbedBackendKind, EmbedPreference, Embedder, HashedEmbedder,
    NeuralModel, SemanticLocalEmbedding, SEMANTIC_DIM,
};
#[cfg(feature = "neural-embed")]
use ast_sgrep_embed::NeuralEmbeddingConfig;
use std::fs;

/// Plain options: no optional backends, regardless of ambient ASGREP_* env.
fn plain_options() -> SearchOptions {
    SearchOptions {
        use_embed: true,
        use_neural_embed: false,
        use_semantic_only: false,
        use_rerank: false,
        ..SearchOptions::default()
    }
}

/// Tiny indexed corpus (no semantic rows): keeps search tests offline.
fn indexed_corpus() -> (tempfile::TempDir, tempfile::TempDir, std::path::PathBuf) {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    fs::write(
        corpus.path().join("auth.rs"),
        "fn refresh_token() {}\nfn caller() { refresh_token(); }\n",
    )
    .unwrap();
    let index_path = index_dir.path().join("index.db");
    let mut indexer = Indexer::new(IndexOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();
    (corpus, index_dir, index_path)
}

#[test]
fn default_options_request_no_optional_backends() {
    let opts = plain_options();
    assert!(!opts.use_neural_embed);
    assert!(!opts.use_rerank);
    assert_eq!(opts.embed_backend(), EmbedBackend::Auto);
    assert_eq!(opts.embed_preference(), EmbedPreference::Auto);
    assert_eq!(EmbedPreference::default(), EmbedPreference::Auto);
    assert_eq!(EmbedBackend::default(), EmbedBackend::Auto);
    // Hashed/Auto paths are always available: no missing-backend report.
    assert!(opts.unavailable_non_hashed_embed().is_none());
    let mut sem = plain_options();
    sem.set_embed_backend(EmbedBackend::Semantic);
    assert!(sem.use_semantic_only && !sem.use_neural_embed);
    assert_eq!(sem.embed_preference(), EmbedPreference::Semantic);
    assert!(sem.unavailable_non_hashed_embed().is_none());
}

#[test]
fn validate_flags_accept_plain_search_under_every_feature_set() {
    assert!(validate_search_feature_flags(&plain_options()).is_ok());
    // Neural flag without embed enabled is inert at validation (no backend
    // is constructed), under every feature set.
    let mut opts = plain_options();
    opts.use_embed = false;
    opts.use_neural_embed = true;
    assert!(validate_search_feature_flags(&opts).is_ok());
}

#[test]
fn neural_request_fails_closed_without_feature() {
    let mut opts = plain_options();
    opts.use_embed = true;
    opts.use_neural_embed = true;
    let result = validate_search_feature_flags(&opts);
    assert_eq!(result.is_err(), !cfg!(feature = "neural-embed"));
    if let Err(e) = result {
        assert!(matches!(e, StoreError::Other(_)));
    }
}

#[test]
fn rerank_request_fails_closed_without_feature() {
    let mut opts = plain_options();
    opts.use_rerank = true;
    let result = validate_search_feature_flags(&opts);
    assert_eq!(result.is_err(), !cfg!(feature = "rerank"));
    if let Err(e) = result {
        assert!(matches!(e, StoreError::Other(_)));
    }
}

#[test]
fn searcher_new_enforces_feature_gates() {
    let (corpus, _index_dir, index_path) = indexed_corpus();
    let base = || SearchOptions {
        root: corpus.path().to_path_buf(),
        index_path: Some(index_path.clone()),
        limit: 16,
        ..plain_options()
    };
    assert!(Searcher::new(base()).is_ok());
    let mut neural = base();
    neural.use_embed = true;
    neural.use_neural_embed = true;
    assert_eq!(
        Searcher::new(neural).is_err(),
        !cfg!(feature = "neural-embed")
    );
    let mut rerank = base();
    rerank.use_rerank = true;
    assert_eq!(Searcher::new(rerank).is_err(), !cfg!(feature = "rerank"));
}

#[test]
fn hashed_embed_path_is_deterministic_and_offline() {
    let embedder = HashedEmbedder::default();
    assert_eq!(embedder.dim(), SEMANTIC_DIM);
    assert_eq!(embedder.dim(), default_semantic_dim());
    assert_eq!(embedder.cost_hint(), CostHint::LocalCheap);
    assert!(embedder.model_id().starts_with("hashed-256-"));
    let a = embedder.embed("refresh token authentication").unwrap();
    let b = embedder.embed("refresh token authentication").unwrap();
    assert_eq!(a.len(), SEMANTIC_DIM);
    assert_eq!(a, b);
    // Same input through the underlying local embedding agrees exactly.
    assert_eq!(SemanticLocalEmbedding.embed_text("refresh token authentication"), a);
    // Distinct texts map to distinct vectors (the hashed path discriminates).
    let c = embedder.embed("database migration rollback").unwrap();
    assert_ne!(a, c);
    // Empty input still yields a full-dim vector, never an error.
    assert_eq!(embedder.embed("").unwrap().len(), SEMANTIC_DIM);
    // Semantic backend is constructible under EVERY feature set.
    let boxed = embedder_for(EmbedBackendKind::Semantic).unwrap();
    assert_eq!(boxed.dim(), SEMANTIC_DIM);
    assert_eq!(boxed.cost_hint(), CostHint::LocalCheap);
}

#[test]
fn neural_embedder_absent_without_feature() {
    #[cfg(not(feature = "neural-embed"))]
    {
        assert!(embedder_for(EmbedBackendKind::Neural).is_none());
    }
    #[cfg(feature = "neural-embed")]
    {
        // Heavy-feature side must not construct the neural embedder here:
        // that would attempt a model load (network). Pin only that the
        // always-local semantic path stays available.
        assert!(embedder_for(EmbedBackendKind::Semantic).is_some());
    }
}

#[test]
fn embed_chain_backend_resolution() {
    // Semantic preference is local under EVERY feature set (never neural).
    let r = embed_with_chain("refresh token", EmbedPreference::Semantic);
    assert_eq!(r.backend, EmbedBackendKind::Semantic);
    assert_eq!(r.vector.len(), SEMANTIC_DIM);
    #[cfg(not(feature = "neural-embed"))]
    {
        // Default build: Neural/Auto requests resolve to the local backend.
        for pref in [EmbedPreference::Neural, EmbedPreference::Auto] {
            let r = embed_with_chain("refresh token", pref);
            assert_eq!(r.backend, EmbedBackendKind::Semantic);
            assert_eq!(r.vector.len(), SEMANTIC_DIM);
        }
    }
}

#[test]
fn neural_config_and_backend_parsing_surface() {
    // Neural model table is compiled in under every feature set (pure data).
    for model in [
        NeuralModel::AllMiniLmL6V2,
        NeuralModel::AllMiniLmL6V2Q,
        NeuralModel::BgeSmallEnV15,
    ] {
        assert_eq!(model.dim(), 384);
        assert!(!model.as_str().is_empty());
    }
    // Configured-model id always names a known model, whatever the env.
    let known = ["all-minilm-l6-v2", "all-minilm-l6-v2-q", "bge-small-en-v1.5"];
    assert!(known.contains(&neural_configured_model_id()));
    // Cache dir always ends at ast-sgrep/models (XDG or $HOME/.cache).
    assert!(neural_default_cache_dir().ends_with("ast-sgrep/models"));
    // Backend parsing round-trips (pure, env-independent).
    assert_eq!(
        EmbedBackendKind::parse("neural"),
        Some(EmbedBackendKind::Neural)
    );
    assert_eq!(
        EmbedBackendKind::parse("fastembed"),
        Some(EmbedBackendKind::Neural)
    );
    assert_eq!(
        EmbedBackendKind::parse("semantic-v2"),
        Some(EmbedBackendKind::Semantic)
    );
    assert_eq!(
        EmbedBackendKind::parse("local"),
        Some(EmbedBackendKind::Semantic)
    );
    assert_eq!(EmbedBackendKind::parse("bogus"), None);
    assert_eq!(EmbedBackend::parse("neural"), EmbedBackend::Neural);
    assert_eq!(EmbedBackend::parse("semantic"), EmbedBackend::Semantic);
    assert_eq!(EmbedBackend::parse("bogus"), EmbedBackend::Auto);
    assert_eq!(EmbedBackendKind::Neural.as_meta_str(), "neural");
    assert_eq!(EmbedBackendKind::Semantic.as_meta_str(), "semantic-v2");
}

#[test]
fn semantic_dim_and_embed_query_contract() {
    assert_eq!(SEMANTIC_DIM, 256);
    assert_eq!(default_semantic_dim(), SEMANTIC_DIM);
    // No stored backend: resolves through the chain (semantic here).
    let r = embed_query("refresh token", None, 0, EmbedPreference::Semantic).unwrap();
    assert_eq!(r.backend, EmbedBackendKind::Semantic);
    assert_eq!(r.vector.len(), SEMANTIC_DIM);
    // Stored semantic-v2 rows with matching dim embed fine.
    let r = embed_query(
        "refresh token",
        Some("semantic-v2"),
        SEMANTIC_DIM,
        EmbedPreference::Auto,
    )
    .unwrap();
    assert_eq!(r.vector.len(), SEMANTIC_DIM);
    // Dim mismatch, removed HTTP backends, and unknown backends are hard
    // errors (discriminant only).
    assert!(embed_query("q", Some("semantic-v2"), SEMANTIC_DIM + 1, EmbedPreference::Auto).is_err());
    assert!(embed_query("q", Some("cloud"), 0, EmbedPreference::Auto).is_err());
    assert!(embed_query("q", Some("ollama"), 0, EmbedPreference::Auto).is_err());
    assert!(embed_query("q", Some("bogus"), 0, EmbedPreference::Auto).is_err());
}

#[test]
fn unavailable_non_hashed_embed_pins_neural_gap() {
    assert!(plain_options().unavailable_non_hashed_embed().is_none());
    let mut neural = plain_options();
    neural.set_embed_backend(EmbedBackend::Neural);
    assert_eq!(neural.embed_preference(), EmbedPreference::Neural);
    #[cfg(not(feature = "neural-embed"))]
    assert!(neural.unavailable_non_hashed_embed().is_some());
    // Heavy-feature side: missing report iff neural is configured via env
    // (pure env read — no model load).
    #[cfg(feature = "neural-embed")]
    assert_eq!(
        neural.unavailable_non_hashed_embed().is_some(),
        NeuralEmbeddingConfig::from_env().is_none()
    );
}

#[test]
fn rerank_off_search_ignores_rerank_knobs() {
    let (corpus, _index_dir, index_path) = indexed_corpus();
    let search_with = |rerank_top_k: usize| {
        let searcher = Searcher::new(SearchOptions {
            root: corpus.path().to_path_buf(),
            index_path: Some(index_path.clone()),
            limit: 16,
            use_embed: false,
            use_rerank: false,
            rerank_top_k,
            ..SearchOptions::default()
        })
        .unwrap();
        let resp = searcher.search("callers:refresh_token").unwrap();
        assert!(!resp.hits.is_empty());
        serde_json::to_string(&resp).unwrap()
    };
    // Rerank off: the rerank knob must not perturb a single byte.
    assert_eq!(search_with(1), search_with(50));
}

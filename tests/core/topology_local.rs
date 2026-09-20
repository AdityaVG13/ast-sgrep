//! Topology CORE local path: the always-local embed surface as intent suites.
//!
//! Consolidates the local-path atoms of `topology_pass1.rs` (T1 default
//! surface) and the local-equivalence deltas of `topology_pass3.rs` (T3)
//! into 5 tests, one intent each. Features must not perturb the local path:
//! every universal test below asserts the identical property under all five
//! feature sets, and the one cfg-gated test spans BOTH non-neural cells
//! (default + rerank-only) instead of T3's single not(any) cell.
//!
//! OFFLINE POLICY: every test RUNS under every feature set. Only always-local
//! entry points are called universally (hashed direct, `embedder_for`
//! Semantic, `Semantic`-preference chain/batch, semantic-alias `embed_query`,
//! `use_embed = false` searches); `Neural`-preference chain calls and stored
//! neural probes happen only in the non-neural cfg-gated test, where the
//! no-feature stub answers without a load. `embed_with_chain(Auto)` runs
//! universally only when `from_env()` is None (chain provably empty).
//! Discriminant/value assertions only, never message text.

use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Active matrix cell, for failure messages on cross-set assertions.
fn active_cell() -> &'static str {
    match (cfg!(feature = "neural-embed"), cfg!(feature = "rerank")) {
        (false, false) => "default",
        (true, false) => "neural-only",
        (false, true) => "rerank-only",
        (true, true) => "all",
    }
}

/// Fully hermetic local options: every field explicit, no `..Default`, so no
/// ambient `ASGREP_*` env var can perturb the test. `use_embed = false` keeps
/// every search on the local path (no embedding under any feature set).
fn hermetic_local_options() -> SearchOptions {
    SearchOptions {
        root: std::path::PathBuf::from("."),
        index_path: None,
        limit: 16,
        lang_filter: None,
        use_embed: false,
        use_tantivy: false,
        use_neural_embed: false,
        use_semantic_only: false,
        use_repository_vocabulary: true,
        ann_threshold: None,
        ann_probes: None,
        use_rerank: false,
        rerank_top_k: 20,
        case_insensitive: false,
        context_before: 0,
        context_after: 0,
        count_only: false,
        file_filter: None,
    }
}

/// Tiny indexed corpus with NO semantic rows: keeps every search offline.
/// `embed_semantic: false` means no embedding at index time either. Corpus
/// dir via `testkit::file_tree` (shared seam); the index dir stays a bare
/// TempDir (no testkit helper pairs a private corpus with a private index).
fn indexed_corpus() -> (tempfile::TempDir, tempfile::TempDir, std::path::PathBuf) {
    let corpus = ast_sgrep_testkit::file_tree(&[(
        "auth.rs",
        "fn refresh_token() {}\nfn caller() { refresh_token(); }\n",
    )]);
    let index_dir = tempfile::tempdir().unwrap();
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

fn searcher_for(
    corpus: &std::path::Path,
    index_path: &std::path::Path,
    opts: SearchOptions,
) -> Searcher {
    let mut opts = opts;
    opts.root = corpus.to_path_buf();
    opts.index_path = Some(index_path.to_path_buf());
    Searcher::new(opts).unwrap()
}

/// Whole-response bytes for inertness facets: the knob must not perturb a
/// single byte. File-local because testkit's `response_hit_keys` drops
/// scores/counts/wrappers by design, which is exactly what inertness pins.
fn search_bytes(searcher: &Searcher, query: &str) -> String {
    let resp = searcher.search(query).unwrap();
    assert!(!resp.hits.is_empty(), "inertness probe must hit");
    serde_json::to_string(&resp).unwrap()
}

// ---------------------------------------------------------------------------
// Local-path equivalence (universal unless noted)
// ---------------------------------------------------------------------------

/// INTENT: every always-local embed entry point produces the BIT-IDENTICAL
/// vector for each input, under every feature set.
/// Facets: hashed dim/cost/model-id/determinism/empty-input; exact-vector
/// agreement across direct + `embedder_for` + chain + batch APIs; all three
/// stored semantic aliases x dims {0, 256}; distinct inputs discriminate.
/// Absorbs T1#6 (hashed determinism/offline) and T3#5 (vector agreement).
/// KILLS: vector-divergence.
#[test]
fn local_embed_vectors_bit_identical_across_entry_points() {
    use ast_sgrep_embed::{
        embed_batch_with_chain, embed_query, embed_with_chain, embedder_for, EmbedBackendKind,
        EmbedPreference, Embedder, HashedEmbedder, SemanticLocalEmbedding, SEMANTIC_DIM,
    };
    let embedder = HashedEmbedder::default();
    assert_eq!(embedder.dim(), SEMANTIC_DIM);
    assert_eq!(embedder.dim(), ast_sgrep_embed::default_semantic_dim());
    assert_eq!(embedder.cost_hint(), ast_sgrep_embed::CostHint::LocalCheap);
    assert!(embedder.model_id().starts_with("hashed-256-"));
    // Empty input still yields a full-dim vector, never an error.
    assert_eq!(embedder.embed("").unwrap().len(), SEMANTIC_DIM);
    // Same input through the underlying local embedding agrees exactly.
    assert_eq!(
        SemanticLocalEmbedding.embed_text("refresh token authentication"),
        embedder.embed("refresh token authentication").unwrap()
    );

    let inputs = [
        "refresh token authentication",
        "database migration rollback",
        "",
        "héllo wörld 🦀",
    ];
    let refs: Vec<&str> = inputs.to_vec();
    let batched = embed_batch_with_chain(&refs, EmbedPreference::Semantic);
    assert_eq!(batched.len(), inputs.len());
    let mut seen = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        let direct = embedder.embed(input).unwrap();
        assert_eq!(direct.len(), SEMANTIC_DIM);
        // Determinism: same input, same vector, every set.
        assert_eq!(direct, embedder.embed(input).unwrap());
        let via_for = embedder_for(EmbedBackendKind::Semantic).unwrap();
        assert_eq!(via_for.dim(), SEMANTIC_DIM);
        assert_eq!(via_for.cost_hint(), ast_sgrep_embed::CostHint::LocalCheap);
        assert_eq!(via_for.embed(input).unwrap(), direct);
        let chained = embed_with_chain(input, EmbedPreference::Semantic);
        assert_eq!(chained.backend, EmbedBackendKind::Semantic);
        assert_eq!(chained.vector, direct);
        assert_eq!(batched[i].backend, EmbedBackendKind::Semantic);
        assert_eq!(batched[i].vector, direct);
        for alias in ["semantic-v2", "semantic", "local"] {
            for dim in [0, SEMANTIC_DIM] {
                let r = embed_query(input, Some(alias), dim, EmbedPreference::Auto).unwrap();
                assert_eq!(r.backend, EmbedBackendKind::Semantic);
                assert_eq!(r.vector, direct, "alias {alias} dim {dim}");
            }
        }
        // The local path discriminates distinct inputs.
        for prev in &seen {
            assert_ne!(&direct, prev, "inputs must map distinctly");
        }
        seen.push(direct);
    }
}

/// INTENT: without env configuration the `Auto` preference stays on the
/// local backend with the exact local vector, and the pure model-id surface
/// is exact — under EVERY set including neural-ON.
/// Facets: Semantic preference exact local vector; Auto-unconfigured local
/// (chain provably empty, no load even ON); `from_env` deterministic pure
/// read; `configured_backend_model_id` exact strings for both backends.
/// Absorbs T1#8 (every-set Semantic facet) and T3#8 (offline resolution),
/// plus the T2#6 `from_env` well-formedness atom (the `size_of` atom is
/// dropped: presence is proven by compilation).
/// KILLS: resolution-inversion.
#[test]
fn offline_resolution_surface_stable() {
    use ast_sgrep_embed::{
        configured_backend_model_id, embed_with_chain, neural_configured_model_id,
        EmbedBackendKind, EmbedPreference, Embedder, HashedEmbedder, NeuralEmbeddingConfig,
        SEMANTIC_DIM,
    };
    // Env config is a pure deterministic read under every set.
    assert_eq!(
        NeuralEmbeddingConfig::from_env(),
        NeuralEmbeddingConfig::from_env()
    );
    if let Some(config) = NeuralEmbeddingConfig::from_env() {
        assert_eq!(config.model.dim(), 384);
        assert!(!config.cache_dir.as_os_str().is_empty());
    }
    // Pure model-id surface: exact strings, every set, never a load.
    assert_eq!(
        configured_backend_model_id(EmbedBackendKind::Semantic, SEMANTIC_DIM),
        Some("semantic:hashed-v2:256".to_string())
    );
    assert_eq!(
        configured_backend_model_id(EmbedBackendKind::Semantic, 999),
        Some("semantic:hashed-v2:999".to_string())
    );
    let neural_id = configured_backend_model_id(EmbedBackendKind::Neural, 384).unwrap();
    assert!(neural_id.starts_with("neural:"));
    assert!([
        "all-minilm-l6-v2",
        "all-minilm-l6-v2-q",
        "bge-small-en-v1.5"
    ]
    .contains(&neural_configured_model_id()));
    assert_eq!(
        neural_id,
        format!("neural:{}", neural_configured_model_id())
    );
    // Semantic preference is local with the exact local vector (every set).
    let hashed = HashedEmbedder::default();
    let direct = hashed.embed("refresh token").unwrap();
    let r = embed_with_chain("refresh token", EmbedPreference::Semantic);
    assert_eq!(r.backend, EmbedBackendKind::Semantic);
    assert_eq!(r.vector, direct);
    // Auto without env config: local too (chain empty => no load even ON).
    // With env configured, the ON side would resolve neural (model load), so
    // this test scopes itself to the unconfigured case and returns.
    if NeuralEmbeddingConfig::from_env().is_some() {
        return;
    }
    let r = embed_with_chain("refresh token", EmbedPreference::Auto);
    assert_eq!(r.backend, EmbedBackendKind::Semantic);
    assert_eq!(r.vector, direct);
}

/// INTENT: where neural is OFF, Neural/Auto requests resolve to the EXACT
/// local vector and stored neural spellings are unresolvable — across BOTH
/// non-neural cells (default + rerank-only), not just T3's not(any) cell.
/// Facets: Neural/Auto single+batch exact fallback; `neural`/`fastembed`
/// stored aliases rejected at dims {0, 384}; stored semantic alias still
/// resolves exact under a Neural preference.
/// Absorbs T1#8 (OFF-side fallback facet), T2#3 (stored-neural atom), T3#9
/// (exact fallback + alias rejection), and the T4#4 stored-backend matrix.
/// KILLS: fallback-inversion, gate-inversion.
#[cfg(not(feature = "neural-embed"))]
#[test]
fn non_neural_cells_fallback_exact_and_stored_neural_rejected() {
    // The #[cfg(not(feature = "neural-embed"))] gate above is the guard: this
    // cell cannot compile, let alone run, with neural compiled in.
    use ast_sgrep_embed::{
        embed_batch_with_chain, embed_query, embed_with_chain, EmbedBackendKind, EmbedPreference,
        Embedder, HashedEmbedder, SEMANTIC_DIM,
    };
    let hashed = HashedEmbedder::default();
    // Neural/Auto requests resolve to the local backend with the EXACT local
    // vector (single + batch APIs).
    for pref in [EmbedPreference::Neural, EmbedPreference::Auto] {
        for input in ["refresh token", ""] {
            let r = embed_with_chain(input, pref);
            assert_eq!(r.backend, EmbedBackendKind::Semantic);
            assert_eq!(r.vector, hashed.embed(input).unwrap());
        }
    }
    let batched = embed_batch_with_chain(&["alpha", "beta"], EmbedPreference::Neural);
    assert_eq!(batched.len(), 2);
    for (r, input) in batched.iter().zip(["alpha", "beta"]) {
        assert_eq!(r.backend, EmbedBackendKind::Semantic);
        assert_eq!(r.vector, hashed.embed(input).unwrap());
    }
    // Stored neural spellings cannot resolve without the feature, at any dim
    // (no load: the no-feature stub returns None before any model touch).
    for backend in ["neural", "fastembed"] {
        for dim in [0, SEMANTIC_DIM, 384] {
            assert!(
                embed_query("refresh token", Some(backend), dim, EmbedPreference::Auto).is_err(),
                "backend {backend} dim {dim} must be rejected in cell {}",
                active_cell()
            );
        }
    }
    // The always-local semantic path agrees exactly too, even under a Neural
    // preference (stored rows win over the request preference).
    let r = embed_query("q", Some("semantic"), SEMANTIC_DIM, EmbedPreference::Neural).unwrap();
    assert_eq!(r.backend, EmbedBackendKind::Semantic);
    assert_eq!(r.vector, hashed.embed("q").unwrap());
}

/// INTENT: the pure-data embed contracts — dims, model table, cache dir,
/// backend parsing, and the `embed_query` ok/error surface — hold under
/// every feature set with no model load.
/// Facets: SEMANTIC_DIM=256; model table dims/names; configured id known;
/// cache dir suffix; `EmbedBackendKind`/`EmbedBackend` parse round-trips +
/// meta strings; `embed_query` ok paths + dim-mismatch/cloud/ollama/bogus
/// rejections.
/// Absorbs T1#9 (parsing surface) and T1#10 (`embed_query` contract).
/// KILLS: error-omission (rejection paths); BEHAVIOR-ONLY on pure data.
#[test]
fn embed_query_and_backend_parsing_contracts() {
    use ast_sgrep_core::EmbedBackend;
    use ast_sgrep_embed::{
        default_semantic_dim, embed_query, neural_configured_model_id, neural_default_cache_dir,
        EmbedBackendKind, EmbedPreference, NeuralModel, SEMANTIC_DIM,
    };
    assert_eq!(SEMANTIC_DIM, 256);
    assert_eq!(default_semantic_dim(), SEMANTIC_DIM);
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
    let known = [
        "all-minilm-l6-v2",
        "all-minilm-l6-v2-q",
        "bge-small-en-v1.5",
    ];
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
    assert!(embed_query(
        "q",
        Some("semantic-v2"),
        SEMANTIC_DIM + 1,
        EmbedPreference::Auto
    )
    .is_err());
    assert!(embed_query("q", Some("cloud"), 0, EmbedPreference::Auto).is_err());
    assert!(embed_query("q", Some("ollama"), 0, EmbedPreference::Auto).is_err());
    assert!(embed_query("q", Some("bogus"), 0, EmbedPreference::Auto).is_err());
}

/// INTENT: option normalization clamps identically every set, and optional
/// knobs move zero search bytes when their feature path is off.
/// Facets: limit/top-k/context clamps (zero/oversize/in-range);
/// `rerank_top_k` byte-inert with rerank off; neural flag byte-inert at
/// search level with embed off, every set including neural-ON.
/// Absorbs T3#6 (normalization), T1#12 (rerank knob inertness), T3#4
/// (neural search-bytes inertness; validation-level inertness lives in
/// `topology_gates`).
/// KILLS: clamp-divergence, knob-leak, gate-ordering.
#[test]
fn normalization_clamps_and_knob_inertness() {
    use ast_sgrep_core::limits::{MAX_EXCERPT_LINES, MAX_OUTPUT_RESULTS};
    let (corpus, _index_dir, index_path) = indexed_corpus();
    // Clamps identical under every feature set.
    let build = |mut opts: SearchOptions| {
        opts.root = corpus.path().to_path_buf();
        opts.index_path = Some(index_path.clone());
        Searcher::new(opts).unwrap()
    };
    let mut zero = hermetic_local_options();
    zero.limit = 0;
    zero.rerank_top_k = 0;
    let s = build(zero);
    assert_eq!(s.options().limit, 16);
    assert_eq!(s.options().rerank_top_k, 1);
    let mut huge = hermetic_local_options();
    huge.limit = usize::MAX;
    huge.rerank_top_k = usize::MAX;
    huge.context_before = usize::MAX;
    huge.context_after = usize::MAX;
    let s = build(huge);
    assert_eq!(s.options().limit, MAX_OUTPUT_RESULTS);
    assert_eq!(s.options().rerank_top_k, MAX_OUTPUT_RESULTS);
    assert_eq!(s.options().context_before, MAX_EXCERPT_LINES);
    assert_eq!(s.options().context_after, MAX_EXCERPT_LINES);
    let s = build(hermetic_local_options());
    assert_eq!(s.options().limit, 16);
    assert_eq!(s.options().rerank_top_k, 20);
    // Rerank off: the rerank knob must not perturb a single byte.
    let search_with = |rerank_top_k: usize| {
        let mut opts = hermetic_local_options();
        opts.rerank_top_k = rerank_top_k;
        search_bytes(
            &searcher_for(corpus.path(), &index_path, opts),
            "callers:refresh_token",
        )
    };
    assert_eq!(search_with(1), search_with(50));
    // Embed off: the neural flag must not perturb a single byte — under
    // EVERY set including neural-ON (the embed pass is skipped when
    // `use_embed` is false, so no load). Whole-response bytes subsume T3's
    // hits-only comparison.
    let plain = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    let mut flagged_opts = hermetic_local_options();
    flagged_opts.use_neural_embed = true;
    let flagged = searcher_for(corpus.path(), &index_path, flagged_opts);
    for query in [
        "defs:refresh_token",
        "callers:refresh_token",
        "refresh_token",
    ] {
        assert_eq!(
            search_bytes(&plain, query),
            search_bytes(&flagged, query),
            "neural flag must not move search bytes for {query:?} in cell {}",
            active_cell()
        );
    }
}

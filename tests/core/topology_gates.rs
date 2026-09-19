//! Topology CORE gates: the 2x2 feature-gate matrix as intent-grouped suites.
//!
//! Consolidates `topology_pass1.rs` (T1 default surface, gate atoms),
//! `topology_pass2.rs` (T2 gate matrix), and the gate-fidelity halves of T3/T4
//! into 2 tests, one intent each. Passes T1-T4 are sections, not files; the
//! per-cell gate-pair boilerplate (T2#3-9, T1#3-5) folds into per-surface
//! matrix intents asserted with `cfg!` branches so ONE test pins all four
//! cells when the target runs under the five feature sets.
//!
//! | cell        | neural-embed | rerank | gate pair (neural, rerank) |
//! |-------------|--------------|--------|----------------------------|
//! | default     | OFF          | OFF    | (Err, Err)                 |
//! | neural-only | ON           | OFF    | (Ok, Err)                  |
//! | rerank-only | OFF          | ON     | (Err, Ok)                  |
//! | all         | ON           | ON     | (Ok, Ok)                   |
//!
//! OFFLINE POLICY: every test RUNS under every feature set (default /
//! `rerank` / `neural-embed` / `--all-features` / `--no-default-features`).
//! Only validation (pure) and `Searcher::new` construction (no embedding at
//! construction) are exercised — never `.search()` with open optional flags,
//! never `embedder_for(Neural)` on the ON side. Discriminant/value assertions
//! only, never message text. All options are built field by field (no
//! `..Default`) so ambient `ASGREP_*` env cannot perturb them.

use ast_sgrep_core::search::validate_search_feature_flags;
use ast_sgrep_core::{EmbedBackend, IndexOptions, Indexer, SearchOptions, Searcher, StoreError};
use ast_sgrep_embed::{embedder_for, EmbedBackendKind, EmbedPreference};

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

/// Fully hermetic plain options: every field explicit, no `..Default`, so no
/// ambient `ASGREP_*` env var can perturb the test. No optional backends.
fn hermetic_plain() -> SearchOptions {
    SearchOptions {
        root: std::path::PathBuf::from("."),
        index_path: None,
        limit: 16,
        lang_filter: None,
        use_embed: true,
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

fn neural_req() -> SearchOptions {
    let mut opts = hermetic_plain();
    opts.use_embed = true;
    opts.use_neural_embed = true;
    opts
}

fn rerank_req() -> SearchOptions {
    let mut opts = hermetic_plain();
    opts.use_rerank = true;
    opts
}

fn both_req() -> SearchOptions {
    let mut opts = neural_req();
    opts.use_rerank = true;
    opts
}

/// Tiny indexed corpus (no semantic rows): keeps construction tests offline.
/// Corpus dir via `testkit::file_tree` (shared seam); the index dir stays a
/// bare TempDir because no testkit helper pairs a private corpus with a
/// private on-disk index for these bespoke 1-file contents.
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

// ---------------------------------------------------------------------------
// Gate matrix (universal: one test pins all four cells across the five sets)
// ---------------------------------------------------------------------------

/// INTENT: the joint 2x2 gate pair (neural, rerank) plus the both-flags
/// conjunction plus `Searcher::new`-vs-`validate` equivalence all equal the
/// ambient feature set, in every build.
/// Facets: plain row Ok everywhere; embed-off neural inert at validation;
/// neural/rerank/both verdicts equal cfg presence; every closed gate reports
/// `StoreError::Other`; construction matches validation for all 4 option sets.
/// Absorbs T1#2 (both halves), T1#3, T1#4, T1#5, T2#1 (joint pair),
/// T2#3 (gate halves; its stored-neural atom lives in `topology_local`),
/// T2#4 (no-default equivalence), T3#7 (universal construction equivalence),
/// and every per-cell validate+construct half (T2#5/7/9, T4#4 construction
/// atoms, CLI T2 lib-validation rows).
/// KILLS: gate-inversion, conjunction-error, construction-gate-divergence.
#[test]
fn gate_pair_conjunction_and_construction_match_features() {
    // Plain row validates Ok under every feature set.
    assert!(
        validate_search_feature_flags(&hermetic_plain()).is_ok(),
        "plain search must validate in cell {}",
        active_cell()
    );
    // Neural flag without embed enabled is inert at validation (no backend
    // is constructed), under every feature set.
    let mut inert = hermetic_plain();
    inert.use_embed = false;
    inert.use_neural_embed = true;
    assert!(
        validate_search_feature_flags(&inert).is_ok(),
        "embed-off neural must be inert at validation in cell {}",
        active_cell()
    );
    // Joint pair: each gate verdict equals its feature presence.
    let neural_ok = validate_search_feature_flags(&neural_req()).is_ok();
    let rerank_ok = validate_search_feature_flags(&rerank_req()).is_ok();
    assert_eq!(
        (neural_ok, rerank_ok),
        (cfg!(feature = "neural-embed"), cfg!(feature = "rerank")),
        "joint gate pair in cell {}",
        active_cell()
    );
    // Conjunction opens only when BOTH features are present.
    assert_eq!(
        validate_search_feature_flags(&both_req()).is_ok(),
        cfg!(all(feature = "neural-embed", feature = "rerank")),
        "conjunction in cell {}",
        active_cell()
    );
    // Every closed gate reports the fail-closed discriminant (never success,
    // never a wrong variant).
    for (result, open, name) in [
        (
            validate_search_feature_flags(&neural_req()),
            cfg!(feature = "neural-embed"),
            "neural",
        ),
        (
            validate_search_feature_flags(&rerank_req()),
            cfg!(feature = "rerank"),
            "rerank",
        ),
        (
            validate_search_feature_flags(&both_req()),
            cfg!(all(feature = "neural-embed", feature = "rerank")),
            "both",
        ),
    ] {
        assert_eq!(
            result.is_ok(),
            open,
            "{name} verdict in cell {}",
            active_cell()
        );
        if let Err(e) = result {
            assert!(
                matches!(e, StoreError::Other(_)),
                "{name} must fail closed with Other in cell {}",
                active_cell()
            );
        }
    }
    // Construction enforces exactly the validate gates in EVERY cell,
    // including the conjunction (construction only — no search with open
    // optional flags, so no load under any set).
    let (corpus, _index_dir, index_path) = indexed_corpus();
    for (opts, name) in [
        (hermetic_plain(), "plain"),
        (neural_req(), "neural"),
        (rerank_req(), "rerank"),
        (both_req(), "both"),
    ] {
        let mut opts = opts;
        opts.root = corpus.path().to_path_buf();
        opts.index_path = Some(index_path.clone());
        assert_eq!(
            Searcher::new(opts.clone()).is_ok(),
            validate_search_feature_flags(&opts).is_ok(),
            "{name} construction must match validation in cell {}",
            active_cell()
        );
    }
}

/// INTENT: option constructors map to the documented backends and the
/// missing-backend report fires iff neural is unavailable.
/// Facets: plain options map to Auto/Semantic, never neural; Semantic setter
/// round-trip; `unavailable_non_hashed_embed` None for plain / Some for
/// Neural OFF-side / env-iff ON-side; `embedder_for(Neural)` None OFF-side
/// while Semantic stays constructible under every set.
/// Absorbs T1#1 (option semantics), T1#11 (unavailable report), T1#7
/// (embedder_for OFF-side None).
/// KILLS: default-flip, report-inversion, fallback-inversion, gate-inversion.
#[test]
fn option_semantics_and_missing_backend_report() {
    // Plain options request no optional backends, whatever the ambient env.
    let opts = hermetic_plain();
    assert!(!opts.use_neural_embed);
    assert!(!opts.use_rerank);
    assert_eq!(opts.embed_backend(), EmbedBackend::Auto);
    assert_eq!(opts.embed_preference(), EmbedPreference::Auto);
    assert_eq!(EmbedPreference::default(), EmbedPreference::Auto);
    assert_eq!(EmbedBackend::default(), EmbedBackend::Auto);
    // Hashed/Auto paths are always available: no missing-backend report.
    assert!(opts.unavailable_non_hashed_embed().is_none());
    let mut sem = hermetic_plain();
    sem.set_embed_backend(EmbedBackend::Semantic);
    assert!(sem.use_semantic_only && !sem.use_neural_embed);
    assert_eq!(sem.embed_preference(), EmbedPreference::Semantic);
    assert!(sem.unavailable_non_hashed_embed().is_none());
    // Missing-backend report fires iff neural is unavailable.
    let mut neural = hermetic_plain();
    neural.set_embed_backend(EmbedBackend::Neural);
    assert_eq!(neural.embed_preference(), EmbedPreference::Neural);
    #[cfg(not(feature = "neural-embed"))]
    assert!(
        neural.unavailable_non_hashed_embed().is_some(),
        "Neural backend must report missing in cell {}",
        active_cell()
    );
    // Heavy-feature side: missing report iff neural is configured via env
    // (pure env read — no model load).
    #[cfg(feature = "neural-embed")]
    assert_eq!(
        neural.unavailable_non_hashed_embed().is_some(),
        ast_sgrep_embed::NeuralEmbeddingConfig::from_env().is_none(),
        "missing report must track env config in cell {}",
        active_cell()
    );
    // Direct OFF-side None pin; the ON side must not construct the neural
    // embedder here (model load) — pin only the always-local path.
    #[cfg(not(feature = "neural-embed"))]
    assert!(embedder_for(EmbedBackendKind::Neural).is_none());
    let semantic = embedder_for(EmbedBackendKind::Semantic).unwrap();
    assert_eq!(semantic.dim(), ast_sgrep_embed::SEMANTIC_DIM);
    assert_eq!(semantic.cost_hint(), ast_sgrep_embed::CostHint::LocalCheap);
}

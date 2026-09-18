//! T2 topology pass 2: feature-COMBINATION matrix for ast-sgrep-core.
//!
//! T1 pinned the default-build surface (local hashed path, single-gate
//! fail-closed behavior). T2 pins the 2x2 COMBINATION matrix over
//! `neural-embed` x `rerank` in ONE file. Core's `[features] default = []`,
//! so `--no-default-features` compiles the identical cell as the default
//! build; one cell below asserts that equivalence explicitly.
//!
//! Cells (exactly one is active per build; see
//! `matrix_cells_mutually_exclusive`):
//!
//! | cell        | neural-embed | rerank | gate pair (neural, rerank) |
//! |-------------|--------------|--------|----------------------------|
//! | default     | OFF          | OFF    | (Err, Err)                 |
//! | neural-only | ON           | OFF    | (Ok, Err)                  |
//! | rerank-only | OFF          | ON     | (Err, Ok)                  |
//! | all         | ON           | ON     | (Ok, Ok)                   |
//!
//! Each cfg-gated cell test opens with NEGATIVE cfg guards (`assert!` on the
//! exact expected `cfg!(feature = ...)` pair) so the test FAILS loudly if it
//! ever runs under the wrong feature set (e.g. gating broken or binary
//! mislabeled), instead of silently passing.
//!
//! OFFLINE POLICY: heavy cells (`neural-embed` and/or `rerank`) pull
//! fastembed+ort. Their tests are COMPILE gates (check-only): every runtime
//! assertion in them is provably load-free — validation (pure), `Searcher::new`
//! construction (no embedding at construction), `size_of::<NeuralEmbedder>()`
//! (const, no construction), `NeuralEmbeddingConfig::from_env()` (pure env
//! read), `embedder_for(Semantic)` (always-local hashed path), and
//! `rerank(_, &[])` (early-returns before any model load). Model-load paths
//! (`embedder_for(Neural)`, `embed_with_chain` with a neural-resolving
//! preference, search with neural flags, `rerank` with non-empty docs) are
//! NEVER called here. Never run the heavy test binaries to "see"; check them.
//!
//! Deterministic, offline, tempfile fixtures, no new deps. Discriminant/value
//! assertions only — never message text.

use ast_sgrep_core::search::validate_search_feature_flags;
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher, StoreError};
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

fn neural_req() -> SearchOptions {
    let mut opts = plain_options();
    opts.use_embed = true;
    opts.use_neural_embed = true;
    opts
}

fn rerank_req() -> SearchOptions {
    let mut opts = plain_options();
    opts.use_rerank = true;
    opts
}

fn both_req() -> SearchOptions {
    let mut opts = neural_req();
    opts.use_rerank = true;
    opts
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

fn assert_other_discriminant<T, E: std::fmt::Debug>(result: Result<T, E>, is_other: impl Fn(&E) -> bool) {
    if let Err(e) = result {
        assert!(is_other(&e), "expected StoreError::Other, got {e:?}");
    }
}

/// The joint 2x2 gate pair, under EVERY feature set: T1 pinned each gate
/// alone; this pins the PAIR (and the conjunction) as one matrix assertion.
#[test]
fn matrix_joint_gate_pair_matches_feature_set() {
    let neural_ok = validate_search_feature_flags(&neural_req()).is_ok();
    let rerank_ok = validate_search_feature_flags(&rerank_req()).is_ok();
    assert_eq!(
        (neural_ok, rerank_ok),
        (cfg!(feature = "neural-embed"), cfg!(feature = "rerank"))
    );
    // Conjunction opens only when BOTH features are present.
    assert_eq!(
        validate_search_feature_flags(&both_req()).is_ok(),
        cfg!(all(feature = "neural-embed", feature = "rerank"))
    );
    // Every closed gate reports the fail-closed discriminant (never success,
    // never a wrong variant).
    for (result, open) in [
        (
            validate_search_feature_flags(&neural_req()),
            cfg!(feature = "neural-embed"),
        ),
        (
            validate_search_feature_flags(&rerank_req()),
            cfg!(feature = "rerank"),
        ),
        (
            validate_search_feature_flags(&both_req()),
            cfg!(all(feature = "neural-embed", feature = "rerank")),
        ),
    ] {
        assert_eq!(result.is_ok(), open);
        assert_other_discriminant(result, |e| matches!(e, StoreError::Other(_)));
    }
}

/// Structural invariant: the four cells partition the feature space — exactly
/// one is active in every build. A new feature combination that lands outside
/// the matrix fails HERE, loudly, instead of silently matching no cell.
#[test]
fn matrix_cells_mutually_exclusive() {
    let cells = [
        !cfg!(any(feature = "neural-embed", feature = "rerank")),
        cfg!(all(feature = "neural-embed", not(feature = "rerank"))),
        cfg!(all(feature = "rerank", not(feature = "neural-embed"))),
        cfg!(all(feature = "neural-embed", feature = "rerank")),
    ];
    assert_eq!(
        cells.iter().filter(|c| **c).count(),
        1,
        "exactly one matrix cell must be active"
    );
}

/// DEFAULT cell (no features): both gates fail closed at validation level,
/// and a stored neural backend is unresolvable without a model load.
#[cfg(not(any(feature = "neural-embed", feature = "rerank")))]
#[test]
fn cell_default_both_gates_fail_closed() {
    // Negative guards: loud failure if this cell runs under heavy features.
    assert!(!cfg!(feature = "neural-embed"), "default cell requires neural-embed OFF");
    assert!(!cfg!(feature = "rerank"), "default cell requires rerank OFF");
    for opts in [neural_req(), rerank_req(), both_req()] {
        let result = validate_search_feature_flags(&opts);
        assert!(result.is_err());
        assert_other_discriminant(result, |e| matches!(e, StoreError::Other(_)));
    }
    // Stored neural rows cannot resolve without the feature — and resolving
    // them must NOT attempt a load (the no-feature stub returns None).
    assert!(
        ast_sgrep_embed::embed_query(
            "refresh token",
            Some("neural"),
            384,
            ast_sgrep_embed::EmbedPreference::Auto,
        )
        .is_err()
    );
}

/// `--no-default-features` EQUIVALENCE cell: core declares `default = []`
/// with no `default`-gated code, so `cargo test` and
/// `cargo test --no-default-features` compile this IDENTICAL cell. Pin the
/// observable consequence: the full `Searcher::new` joint matrix matches the
/// `validate_search_feature_flags` verdict exactly (construction adds no
/// hidden gates and drops none), including the both-flags conjunction.
#[cfg(not(any(feature = "neural-embed", feature = "rerank")))]
#[test]
fn cell_no_default_features_equivalent_to_default() {
    // Negative guards: this equivalence holds ONLY in the no-heavy cell.
    assert!(!cfg!(feature = "neural-embed"), "no-default cell requires neural-embed OFF");
    assert!(!cfg!(feature = "rerank"), "no-default cell requires rerank OFF");
    let (corpus, _index_dir, index_path) = indexed_corpus();
    let with_paths = |mut opts: SearchOptions| {
        opts.root = corpus.path().to_path_buf();
        opts.index_path = Some(index_path.clone());
        opts.limit = 16;
        opts
    };
    for opts in [plain_options(), neural_req(), rerank_req(), both_req()] {
        let opts = with_paths(opts);
        assert_eq!(
            Searcher::new(opts.clone()).is_ok(),
            validate_search_feature_flags(&opts).is_ok(),
            "Searcher::new must enforce exactly the validate gates"
        );
    }
    assert!(Searcher::new(with_paths(plain_options())).is_ok());
    assert!(Searcher::new(with_paths(neural_req())).is_err());
    assert!(Searcher::new(with_paths(rerank_req())).is_err());
    assert!(Searcher::new(with_paths(both_req())).is_err());
}

/// NEURAL-ONLY cell, gates: neural opens, rerank stays closed (including the
/// conjunction). `Searcher::new` construction is load-free; NO search is run
/// with neural flags here (search would load the model — network).
#[cfg(all(feature = "neural-embed", not(feature = "rerank")))]
#[test]
fn cell_neural_only_gate_pair() {
    // Negative guards: exact cell — neural ON, rerank OFF.
    assert!(cfg!(feature = "neural-embed"), "neural-only cell requires neural-embed ON");
    assert!(!cfg!(feature = "rerank"), "neural-only cell requires rerank OFF");
    assert!(validate_search_feature_flags(&neural_req()).is_ok());
    for opts in [rerank_req(), both_req()] {
        let result = validate_search_feature_flags(&opts);
        assert!(result.is_err());
        assert_other_discriminant(result, |e| matches!(e, StoreError::Other(_)));
    }
    let (corpus, _index_dir, index_path) = indexed_corpus();
    let with_paths = |mut opts: SearchOptions| {
        opts.root = corpus.path().to_path_buf();
        opts.index_path = Some(index_path.clone());
        opts.limit = 16;
        opts
    };
    // Construction only — never `.search()` with neural flags.
    assert!(Searcher::new(with_paths(neural_req())).is_ok());
    assert!(Searcher::new(with_paths(rerank_req())).is_err());
    assert!(Searcher::new(with_paths(both_req())).is_err());
}

/// NEURAL-ONLY cell, API presence without load (CHECK-ONLY test): the neural
/// API exists, env config stays a pure read, and the always-local semantic
/// path is intact. `embedder_for(Neural)` / `embed_with_chain` with a
/// neural-resolving preference are DELIBERATELY never called: they load the
/// model (network). Never run this binary to "see" — check it.
#[cfg(all(feature = "neural-embed", not(feature = "rerank")))]
#[test]
fn cell_neural_only_api_presence_without_load() {
    assert!(cfg!(feature = "neural-embed"), "neural-only cell requires neural-embed ON");
    assert!(!cfg!(feature = "rerank"), "neural-only cell requires rerank OFF");
    // Type presence at compile time; `size_of` is const — zero runtime load.
    assert!(std::mem::size_of::<ast_sgrep_embed::NeuralEmbedder>() > 0);
    // Env config is a pure read: when set, values are well-formed (dim +
    // non-empty cache dir). No model is touched.
    if let Some(config) = ast_sgrep_embed::NeuralEmbeddingConfig::from_env() {
        assert_eq!(config.model.dim(), 384);
        assert!(!config.cache_dir.as_os_str().is_empty());
    }
    // Always-local semantic path intact under the neural cell.
    let semantic =
        ast_sgrep_embed::embedder_for(ast_sgrep_embed::EmbedBackendKind::Semantic).unwrap();
    assert_eq!(semantic.dim(), ast_sgrep_embed::SEMANTIC_DIM);
    assert_eq!(semantic.cost_hint(), ast_sgrep_embed::CostHint::LocalCheap);
}

/// RERANK-ONLY cell, gates: rerank opens, neural stays closed (including the
/// conjunction). Construction only; no neural search is run.
#[cfg(all(feature = "rerank", not(feature = "neural-embed")))]
#[test]
fn cell_rerank_only_gate_pair() {
    // Negative guards: exact cell — rerank ON, neural OFF.
    assert!(cfg!(feature = "rerank"), "rerank-only cell requires rerank ON");
    assert!(!cfg!(feature = "neural-embed"), "rerank-only cell requires neural-embed OFF");
    assert!(validate_search_feature_flags(&rerank_req()).is_ok());
    for opts in [neural_req(), both_req()] {
        let result = validate_search_feature_flags(&opts);
        assert!(result.is_err());
        assert_other_discriminant(result, |e| matches!(e, StoreError::Other(_)));
    }
    let (corpus, _index_dir, index_path) = indexed_corpus();
    let with_paths = |mut opts: SearchOptions| {
        opts.root = corpus.path().to_path_buf();
        opts.index_path = Some(index_path.clone());
        opts.limit = 16;
        opts
    };
    assert!(Searcher::new(with_paths(rerank_req())).is_ok());
    assert!(Searcher::new(with_paths(neural_req())).is_err());
    assert!(Searcher::new(with_paths(both_req())).is_err());
}

/// RERANK-ONLY cell, API presence offline (CHECK-ONLY test): the rerank API
/// exists and the empty-docs call resolves WITHOUT a model load
/// (`rerank` early-returns on empty input before any `load()`). Non-empty
/// docs WOULD load the model (network) and are never passed here.
#[cfg(all(feature = "rerank", not(feature = "neural-embed")))]
#[test]
fn cell_rerank_only_api_offline() {
    assert!(cfg!(feature = "rerank"), "rerank-only cell requires rerank ON");
    assert!(!cfg!(feature = "neural-embed"), "rerank-only cell requires neural-embed OFF");
    let scores = ast_sgrep_embed::rerank("refresh token", &[]).unwrap();
    assert!(scores.is_empty());
    let score = ast_sgrep_embed::RerankScore { index: 3, score: 0.5 };
    assert_eq!(score.index, 3);
    assert_eq!(score.score, 0.5);
}

/// ALL-FEATURES cell, gates: both open, including the conjunction.
/// Construction only — never `.search()` with neural flags (model load).
#[cfg(all(feature = "neural-embed", feature = "rerank"))]
#[test]
fn cell_all_features_both_gates_open() {
    // Negative guards: exact cell — BOTH features ON.
    assert!(cfg!(feature = "neural-embed"), "all-features cell requires neural-embed ON");
    assert!(cfg!(feature = "rerank"), "all-features cell requires rerank ON");
    for opts in [plain_options(), neural_req(), rerank_req(), both_req()] {
        assert!(validate_search_feature_flags(&opts).is_ok());
    }
    let (corpus, _index_dir, index_path) = indexed_corpus();
    let with_paths = |mut opts: SearchOptions| {
        opts.root = corpus.path().to_path_buf();
        opts.index_path = Some(index_path.clone());
        opts.limit = 16;
        opts
    };
    for opts in [plain_options(), neural_req(), rerank_req(), both_req()] {
        assert!(Searcher::new(with_paths(opts)).is_ok());
    }
}

/// ALL-FEATURES cell, API presence without load (CHECK-ONLY test): both heavy
/// APIs exist and every runtime assertion is load-free (const `size_of`,
/// pure env read, empty-docs early return, always-local semantic path).
/// Model-load paths are never called. Never run — check.
#[cfg(all(feature = "neural-embed", feature = "rerank"))]
#[test]
fn cell_all_features_apis_present_without_load() {
    assert!(cfg!(feature = "neural-embed"), "all-features cell requires neural-embed ON");
    assert!(cfg!(feature = "rerank"), "all-features cell requires rerank ON");
    assert!(std::mem::size_of::<ast_sgrep_embed::NeuralEmbedder>() > 0);
    if let Some(config) = ast_sgrep_embed::NeuralEmbeddingConfig::from_env() {
        assert_eq!(config.model.dim(), 384);
        assert!(!config.cache_dir.as_os_str().is_empty());
    }
    let scores = ast_sgrep_embed::rerank("refresh token", &[]).unwrap();
    assert!(scores.is_empty());
    let semantic =
        ast_sgrep_embed::embedder_for(ast_sgrep_embed::EmbedBackendKind::Semantic).unwrap();
    assert_eq!(semantic.dim(), ast_sgrep_embed::SEMANTIC_DIM);
    assert_eq!(semantic.cost_hint(), ast_sgrep_embed::CostHint::LocalCheap);
}

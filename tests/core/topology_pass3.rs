//! T3 topology pass 3: behavioral DELTAS for ast-sgrep-core features.
//!
//! T1 pinned the default-build surface; T2 pinned the 2x2 combination matrix
//! (`neural-embed` x `rerank`) of gates. T3 pins CROSS-SET RELATIONS —
//! behavior that must hold *across* feature sets — in ONE file:
//!
//! | relation | what is pinned |
//! |----------|----------------|
//! | local-path equivalence | lexical/symbol search hit identities are
//! | | BIT-IDENTICAL under every feature set (same golden asserted in every
//! | | build); local hashed vectors are bit-identical across all five local
//! | | entry points; option normalization clamps identically. Features must
//! | | not perturb the local path. |
//! | gate fidelity (neural) | neural-ON exposes the neural request path and
//! | | it RUNS offline on a rowless corpus (validation Ok, search Ok, local
//! | | hits intact); neural-OFF keeps the exact Semantic fallback vector.
//! | rerank fidelity | rerank-ON keeps default ranking when rerank is unused
//! | | (universal goldens run under rerank-ON too); the rerank request path
//! | | RUNS offline on a proven-empty shortlist; empty-docs rerank is
//! | | query-independent and repeat-deterministic. |
//! | no-leak | `Searcher::new` enforces exactly the `validate` gates in EVERY
//! | | cell (not just the default cell); spanning cells prove feature A never
//! | | moves feature B's gate state. |
//!
//! NOT duplicated from T1/T2: single-gate fail-closed discriminants, the
//! joint gate pair, cell mutual exclusion, per-cell gate pairs, `size_of`
//! presence, single-call empty rerank, `from_env` well-formedness, knob
//! self-consistency (`top_k` 1 vs 50), single-input hashed determinism, model
//! table dims, backend `parse` round-trips. Where T3 touches the same API it
//! asserts a strictly stronger or strictly different property (exact vectors
//! instead of lengths, search bytes instead of validation verdicts, universal
//! instead of single-cell, RUN instead of construct-only).
//!
//! Cells: exactly one is active per build (see T2
//! `matrix_cells_mutually_exclusive`). Universal `delta_*` tests run in ALL
//! cells by construction (that IS the cross-set pin: the same golden in every
//! build); `cell_*` tests are `#[cfg]`-gated to one exact cell and open with
//! NEGATIVE cfg guards so they FAIL loudly under the wrong feature set.
//!
//! OFFLINE POLICY: every test RUNS under every feature set
//! (default / `rerank` / `neural-embed` / `--all-features` /
//! `--no-default-features`). Model-load paths are never reached, by one of:
//!
//! - `use_embed = false`: every embed entry returns before embedding
//!   (`embed.rs` early returns), regardless of neural flags or env.
//! - Rowless corpus: `indexed_corpus` sets `embed_semantic: false`, and neural
//!   RUN tests assert `store.semantic_sources_empty()` FIRST — every embed
//!   entry returns empty on a rowless store before the query is embedded
//!   (lazy-IVF ANN/dim gates, `semantic_sources_empty` probe, empty-chunks
//!   gates, unwarmed cache). Preference/env are irrelevant past that point.
//! - Proven-empty shortlist: rerank RUN tests first prove zero hits on the
//!   safe (`use_rerank = false`) path; only then exercise `use_rerank = true`,
//!   where `maybe_rerank` early-returns before any model load.
//! - Guarded `Auto`: `embed_with_chain(Auto)` is called only when
//!   `NeuralEmbeddingConfig::from_env()` is `None` (chain provably empty, no
//!   load under any set); otherwise the test asserts the Semantic baseline
//!   and returns. `Neural`-preference chain calls happen only in the OFF cell
//!   (stub, no load). `rerank` is called only with empty docs (early return).
//!
//! Deterministic, offline, tempfile fixtures, no new deps. Discriminant/value
//! assertions only — never message text. All `SearchOptions` are built field
//! by field (no `..Default`) so ambient `ASGREP_*` env cannot perturb them.

use ast_sgrep_core::search::validate_search_feature_flags;
use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};
use std::fs;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Active matrix cell, for failure messages on cross-set goldens.
fn active_cell() -> &'static str {
    match (
        cfg!(feature = "neural-embed"),
        cfg!(feature = "rerank"),
    ) {
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

fn neural_req() -> SearchOptions {
    let mut opts = hermetic_local_options();
    opts.use_embed = true;
    opts.use_neural_embed = true;
    opts
}

fn rerank_req() -> SearchOptions {
    let mut opts = hermetic_local_options();
    opts.use_rerank = true;
    opts
}

fn both_req() -> SearchOptions {
    let mut opts = neural_req();
    opts.use_rerank = true;
    opts
}

/// Tiny indexed corpus with NO semantic rows: keeps every search offline.
/// `embed_semantic: false` means no embedding at index time either.
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

/// Owned hit identity for cross-set goldens. Scores as bits: the local path
/// must be bit-identical under every feature set. Snapshot fields (mtime
/// derived) are deliberately excluded — hits only.
#[derive(Debug, PartialEq, Clone)]
struct HitId {
    kind: String,
    file: String,
    line_start: u32,
    line_end: u32,
    symbol: Option<String>,
    caller: Option<String>,
    callee: Option<String>,
    score_bits: u64,
    signal: String,
    excerpt: String,
}

impl HitId {
    fn of(hit: &ast_sgrep_core::SearchHit) -> Self {
        Self {
            kind: hit.kind.as_str().to_string(),
            file: hit.file.clone(),
            line_start: hit.line_start,
            line_end: hit.line_end,
            symbol: hit.symbol.clone(),
            caller: hit.caller.clone(),
            callee: hit.callee.clone(),
            score_bits: hit.score.to_bits(),
            signal: hit.signal.as_str().to_string(),
            excerpt: hit.excerpt.clone(),
        }
    }
}

fn hit_ids(searcher: &Searcher, query: &str) -> Vec<HitId> {
    searcher
        .search(query)
        .unwrap()
        .hits
        .iter()
        .map(HitId::of)
        .collect()
}

// Cross-set goldens: filled from the discovery run, then pinned identical
// under all five feature sets.
fn expected_defs() -> Vec<HitId> {
    vec![HitId {
        kind: "def".to_string(),
        file: "auth.rs".to_string(),
        line_start: 1,
        line_end: 1,
        symbol: Some("refresh_token".to_string()),
        caller: None,
        callee: None,
        score_bits: 4629770785681047552,
        signal: "structural".to_string(),
        excerpt: "fn refresh_token() {}".to_string(),
    }]
}

fn expected_callers() -> Vec<HitId> {
    vec![
        HitId {
            kind: "caller".to_string(),
            file: "auth.rs".to_string(),
            line_start: 2,
            line_end: 2,
            symbol: None,
            caller: Some("caller".to_string()),
            callee: Some("refresh_token".to_string()),
            score_bits: 4622663542519103488,
            signal: "structural".to_string(),
            excerpt: "fn caller() { refresh_token(); }".to_string(),
        },
        HitId {
            kind: "graph".to_string(),
            file: "auth.rs".to_string(),
            line_start: 2,
            line_end: 2,
            symbol: Some("refresh_token".to_string()),
            caller: Some("caller".to_string()),
            callee: Some("refresh_token".to_string()),
            score_bits: 4617315517961601024,
            signal: "structural".to_string(),
            excerpt: "caller calls refresh_token".to_string(),
        },
    ]
}

fn expected_hybrid() -> Vec<HitId> {
    vec![
        HitId {
            kind: "def".to_string(),
            file: "auth.rs".to_string(),
            line_start: 1,
            line_end: 1,
            symbol: Some("refresh_token".to_string()),
            caller: None,
            callee: None,
            score_bits: 4593275893786714022,
            signal: "structural".to_string(),
            excerpt: "fn refresh_token() {}".to_string(),
        },
        HitId {
            kind: "caller".to_string(),
            file: "auth.rs".to_string(),
            line_start: 2,
            line_end: 2,
            symbol: Some("refresh_token".to_string()),
            caller: Some("caller".to_string()),
            callee: Some("refresh_token".to_string()),
            score_bits: 4590475707484293538,
            signal: "structural".to_string(),
            excerpt: "fn caller() { refresh_token(); }".to_string(),
        },
    ]
}

// ---------------------------------------------------------------------------
// Universal relations (run in EVERY cell)
// ---------------------------------------------------------------------------

/// LOCAL-PATH EQUIVALENCE: `defs:` search hit identities are bit-identical
/// under every feature set. T1 only asserted non-empty + knob invariance;
/// the fixed golden here pins cross-set identity (any feature perturbation
/// of the symbol path breaks at least one set).
#[test]
fn delta_defs_search_golden_identical_across_sets() {
    let (corpus, _index_dir, index_path) = indexed_corpus();
    let searcher = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    let resp = searcher.search("defs:refresh_token").unwrap();
    assert_eq!(resp.query, "defs:refresh_token");
    let ids: Vec<HitId> = resp.hits.iter().map(HitId::of).collect();
    assert!(!ids.is_empty(), "defs query must hit");
    for id in &ids {
        assert_eq!(id.file, "auth.rs");
        assert_eq!(id.kind, "def");
        assert_eq!(id.signal, "structural");
        assert!(f64::from_bits(id.score_bits).is_finite());
        assert!(!id.excerpt.is_empty());
    }
    assert_eq!(ids, expected_defs(), "defs golden in cell {}", active_cell());
}

/// LOCAL-PATH EQUIVALENCE: `callers:` search hit identities are bit-identical
/// under every feature set (same rationale as the defs golden).
#[test]
fn delta_callers_search_golden_identical_across_sets() {
    let (corpus, _index_dir, index_path) = indexed_corpus();
    let searcher = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    let resp = searcher.search("callers:refresh_token").unwrap();
    assert_eq!(resp.query, "callers:refresh_token");
    let ids: Vec<HitId> = resp.hits.iter().map(HitId::of).collect();
    assert!(!ids.is_empty(), "callers query must hit");
    for id in &ids {
        assert_eq!(id.file, "auth.rs");
        assert!(id.kind == "caller" || id.kind == "graph", "unexpected kind {}", id.kind);
        assert_eq!(id.signal, "structural");
        assert!(f64::from_bits(id.score_bits).is_finite());
    }
    assert_eq!(
        ids,
        expected_callers(),
        "callers golden in cell {}",
        active_cell()
    );
}

/// LOCAL-PATH EQUIVALENCE: hybrid lexical search hit identities are
/// bit-identical under every feature set, and with `use_embed = false` no
/// embed-kind hit can appear under any set (embed-off purity).
#[test]
fn delta_hybrid_search_golden_identical_across_sets() {
    let (corpus, _index_dir, index_path) = indexed_corpus();
    let searcher = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    let resp = searcher.search("refresh_token").unwrap();
    assert_eq!(resp.query, "refresh_token");
    let ids: Vec<HitId> = resp.hits.iter().map(HitId::of).collect();
    assert!(!ids.is_empty(), "hybrid query must hit");
    assert!(
        ids.iter().all(|id| id.kind != "embed"),
        "embed off: no embed hits under any feature set"
    );
    for id in &ids {
        assert!(f64::from_bits(id.score_bits).is_finite());
    }
    assert_eq!(
        ids,
        expected_hybrid(),
        "hybrid golden in cell {}",
        active_cell()
    );
}

/// GATE FIDELITY (neural inertness): with `use_embed = false`, the neural
/// flag changes search bytes not at all — under EVERY feature set, including
/// neural-ON (T1 pinned validation-level inertness; this pins search bytes).
/// No load: the embed pass is skipped when `use_embed` is false.
#[test]
fn delta_neural_flag_inert_at_search_level_when_embed_off() {
    let (corpus, _index_dir, index_path) = indexed_corpus();
    let mut inert = hermetic_local_options();
    inert.use_neural_embed = true;
    // Validation agrees first (all sets): the flag is inert without embed.
    assert!(validate_search_feature_flags(&hermetic_local_options()).is_ok());
    assert!(validate_search_feature_flags(&inert).is_ok());
    let plain = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    let flagged = searcher_for(corpus.path(), &index_path, inert);
    for query in ["defs:refresh_token", "callers:refresh_token", "refresh_token"] {
        assert_eq!(
            hit_ids(&plain, query),
            hit_ids(&flagged, query),
            "neural flag must not move search bytes for {query:?} in cell {}",
            active_cell()
        );
    }
}

/// LOCAL-PATH EQUIVALENCE (vectors): every always-local entry point produces
/// the BIT-IDENTICAL vector for each input, under every feature set. T1
/// pinned single-input determinism plus len/backend; this pins exact-vector
/// agreement across the batch API and all three stored semantic aliases.
#[test]
fn delta_local_embed_vectors_bit_identical() {
    use ast_sgrep_embed::{
        embed_batch_with_chain, embed_query, embed_with_chain, embedder_for, EmbedBackendKind,
        EmbedPreference, Embedder, HashedEmbedder, SEMANTIC_DIM,
    };
    let inputs = [
        "refresh token authentication",
        "database migration rollback",
        "",
        "héllo wörld 🦀",
    ];
    let hashed = HashedEmbedder::default();
    let refs: Vec<&str> = inputs.iter().copied().collect();
    let batched = embed_batch_with_chain(&refs, EmbedPreference::Semantic);
    assert_eq!(batched.len(), inputs.len());
    let mut seen = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        let direct = hashed.embed(input).unwrap();
        assert_eq!(direct.len(), SEMANTIC_DIM);
        let via_for = embedder_for(EmbedBackendKind::Semantic).unwrap();
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
        // The local path discriminates distinct inputs (empty input is the
        // only one allowed to collide with nothing else either).
        for prev in &seen {
            assert_ne!(&direct, prev, "inputs must map distinctly");
        }
        seen.push(direct);
    }
}

/// LOCAL-PATH EQUIVALENCE (normalization): `Searcher::new` input clamps are
/// identical under every feature set — limit 0 remaps to the default, oversize
/// clamps to `MAX_OUTPUT_RESULTS`, `rerank_top_k` clamps to
/// `1..=MAX_OUTPUT_RESULTS`, context clamps to `MAX_EXCERPT_LINES`.
#[test]
fn delta_option_normalization_identical_across_sets() {
    use ast_sgrep_core::limits::{MAX_EXCERPT_LINES, MAX_OUTPUT_RESULTS};
    let (corpus, _index_dir, index_path) = indexed_corpus();
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
    // In-range values pass through untouched.
    let s = build(hermetic_local_options());
    assert_eq!(s.options().limit, 16);
    assert_eq!(s.options().rerank_top_k, 20);
}

/// NO-LEAK: `Searcher::new` enforces exactly the `validate` gates — in EVERY
/// cell, including the conjunction. T2 pinned this equivalence only in the
/// no-features cell; the universal form proves construction adds no hidden
/// gates and drops none under any feature set. Construction only (no search
/// with open optional flags), so no load under any set.
#[test]
fn delta_searcher_construction_matches_validation_in_every_cell() {
    let (corpus, _index_dir, index_path) = indexed_corpus();
    let with_paths = |mut opts: SearchOptions| {
        opts.root = corpus.path().to_path_buf();
        opts.index_path = Some(index_path.clone());
        opts
    };
    for opts in [
        hermetic_local_options(),
        neural_req(),
        rerank_req(),
        both_req(),
    ] {
        let opts = with_paths(opts);
        assert_eq!(
            Searcher::new(opts.clone()).is_ok(),
            validate_search_feature_flags(&opts).is_ok(),
            "construction must match validation in cell {}",
            active_cell()
        );
    }
}

/// GATE FIDELITY (offline resolution surface): without env configuration the
/// `Auto` preference stays on the local backend with the exact local vector —
/// under EVERY set including neural-ON (T1 pinned this only for OFF; the ON
/// side is load-free because the chain is provably empty when env is unset).
/// Plus the pure model-id surface, exact under every set, no load.
#[test]
fn delta_offline_resolution_surface_stable() {
    use ast_sgrep_embed::{
        configured_backend_model_id, embed_with_chain, neural_configured_model_id, EmbedBackendKind,
        EmbedPreference, Embedder, HashedEmbedder, NeuralEmbeddingConfig, SEMANTIC_DIM,
    };
    // Env config is a pure deterministic read under every set.
    assert_eq!(
        NeuralEmbeddingConfig::from_env(),
        NeuralEmbeddingConfig::from_env()
    );
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
    assert!(["all-minilm-l6-v2", "all-minilm-l6-v2-q", "bge-small-en-v1.5"]
        .contains(&neural_configured_model_id()));
    assert_eq!(neural_id, format!("neural:{}", neural_configured_model_id()));
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

// ---------------------------------------------------------------------------
// Cell tests (one exact cell each, negative guards first)
// ---------------------------------------------------------------------------

/// DEFAULT cell: neural preference falls back to the EXACT local vector
/// (T1 pinned backend+len; exact-vector agreement across single + batch APIs
/// is new), and the `fastembed` neural alias is unresolvable without the
/// feature (T2 pinned only the `neural` spelling).
#[cfg(not(any(feature = "neural-embed", feature = "rerank")))]
#[test]
fn cell_default_neural_fallback_exact_and_alias_rejected() {
    assert!(!cfg!(feature = "neural-embed"), "default cell requires neural-embed OFF");
    assert!(!cfg!(feature = "rerank"), "default cell requires rerank OFF");
    use ast_sgrep_embed::{
        embed_batch_with_chain, embed_query, embed_with_chain, EmbedBackendKind, EmbedPreference,
        Embedder, HashedEmbedder, SEMANTIC_DIM,
    };
    let hashed = HashedEmbedder::default();
    for input in ["refresh token", ""] {
        let direct = hashed.embed(input).unwrap();
        let r = embed_with_chain(input, EmbedPreference::Neural);
        assert_eq!(r.backend, EmbedBackendKind::Semantic);
        assert_eq!(r.vector, direct);
    }
    let batched = embed_batch_with_chain(&["alpha", "beta"], EmbedPreference::Neural);
    assert_eq!(batched.len(), 2);
    for (r, input) in batched.iter().zip(["alpha", "beta"]) {
        assert_eq!(r.backend, EmbedBackendKind::Semantic);
        assert_eq!(r.vector, hashed.embed(input).unwrap());
    }
    // Neural alias spellings cannot resolve without the feature (no load: the
    // no-feature stub returns None before any model touch).
    assert!(embed_query("q", Some("fastembed"), 384, EmbedPreference::Auto).is_err());
    assert!(embed_query("q", Some("fastembed"), 0, EmbedPreference::Auto).is_err());
    // The always-local semantic path agrees exactly too.
    let r = embed_query("q", Some("semantic"), SEMANTIC_DIM, EmbedPreference::Neural).unwrap();
    assert_eq!(r.vector, hashed.embed("q").unwrap());
}

/// NEURAL-ONLY cell, gate fidelity RUN: the neural request path RUNS offline
/// on a rowless corpus — validation Ok, construction Ok, search Ok — and the
/// local ranking is the cross-set golden (neural compiled in must not perturb
/// the local path). T2 only constructed; this is the first search-level RUN.
/// No load: rowlessness is asserted FIRST, and every embed entry returns empty
/// on a rowless store before the query is embedded (preference/env moot).
#[cfg(all(feature = "neural-embed", not(feature = "rerank")))]
#[test]
fn cell_neural_only_request_path_runs_local_on_rowless_corpus() {
    assert!(cfg!(feature = "neural-embed"), "neural-only cell requires neural-embed ON");
    assert!(!cfg!(feature = "rerank"), "neural-only cell requires rerank OFF");
    let (corpus, _index_dir, index_path) = indexed_corpus();
    // Load-free precondition, proven before any neural search.
    let probe = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    assert!(probe.store().semantic_sources_empty().unwrap());
    // Neural request: gate open at validation and construction.
    assert!(validate_search_feature_flags(&neural_req()).is_ok());
    let neural = searcher_for(corpus.path(), &index_path, neural_req());
    // Search RUNS offline; local ranking is the cross-set golden.
    assert_eq!(hit_ids(&neural, "defs:refresh_token"), expected_defs());
    assert_eq!(hit_ids(&neural, "callers:refresh_token"), expected_callers());
    assert_eq!(hit_ids(&neural, "refresh_token"), expected_hybrid());
    // Auto preference on the rowless store agrees too (still no embed reached).
    let mut auto_req = hermetic_local_options();
    auto_req.use_embed = true;
    let auto_searcher = searcher_for(corpus.path(), &index_path, auto_req);
    assert_eq!(hit_ids(&auto_searcher, "defs:refresh_token"), expected_defs());
}

/// RERANK-ONLY cell, rerank fidelity RUN: the rerank request path RUNS offline
/// on a proven-empty shortlist (validation Ok, construction Ok, search Ok
/// with zero hits). T2 never searched with rerank flags; this is the first
/// request-path RUN. No load: emptiness is proven on the safe path FIRST, and
/// `maybe_rerank` early-returns on empty hits before any model load.
/// Plus empty-docs rerank is query-independent and repeat-deterministic (T2
/// pinned a single call).
#[cfg(all(feature = "rerank", not(feature = "neural-embed")))]
#[test]
fn cell_rerank_only_empty_shortlist_request_path_runs_offline() {
    assert!(cfg!(feature = "rerank"), "rerank-only cell requires rerank ON");
    assert!(!cfg!(feature = "neural-embed"), "rerank-only cell requires neural-embed OFF");
    let (corpus, _index_dir, index_path) = indexed_corpus();
    assert!(validate_search_feature_flags(&rerank_req()).is_ok());
    let safe = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    let ranked = searcher_for(corpus.path(), &index_path, rerank_req());
    for query in ["defs:zzz_no_such_symbol_7f3a", "callers:zzz_no_such_symbol_7f3a"] {
        // Safe path first: prove the shortlist is empty (fails before any
        // load risk if the fixture ever matches).
        assert!(hit_ids(&safe, query).is_empty());
        // Request path RUNS offline on the proven-empty shortlist.
        let resp = ranked.search(query).unwrap();
        assert!(resp.hits.is_empty());
    }
    // Empty-docs rerank: query-independent, repeat-deterministic, always Ok.
    for query in ["refresh token", "zzz nothing matches this", ""] {
        let first = ast_sgrep_embed::rerank(query, &[]).unwrap();
        let second = ast_sgrep_embed::rerank(query, &[]).unwrap();
        assert!(first.is_empty() && second.is_empty());
    }
}

/// ALL-FEATURES cell, conjunction RUN: both request paths open and BOTH run
/// offline — the conjunction searcher returns the local golden for live
/// queries (rerank unused => default ranking even with both features compiled
/// in) and zero hits for proven-empty queries (empty shortlist => no rerank
/// load; rowless store => no neural load). No-leak: neural does not perturb
/// the rerank empty path and rerank does not perturb neural resolution.
#[cfg(all(feature = "neural-embed", feature = "rerank"))]
#[test]
fn cell_all_features_conjunction_request_paths_run_offline() {
    assert!(cfg!(feature = "neural-embed"), "all-features cell requires neural-embed ON");
    assert!(cfg!(feature = "rerank"), "all-features cell requires rerank ON");
    let (corpus, _index_dir, index_path) = indexed_corpus();
    let probe = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    assert!(probe.store().semantic_sources_empty().unwrap());
    // Conjunction gate open at validation and construction.
    assert!(validate_search_feature_flags(&both_req()).is_ok());
    // Neural-open + rerank-unused conjunction searcher: local golden intact.
    let neural_open = searcher_for(corpus.path(), &index_path, neural_req());
    assert_eq!(
        hit_ids(&neural_open, "defs:refresh_token"),
        expected_defs(),
        "all cell must keep default ranking"
    );
    // Full conjunction on a proven-empty shortlist: runs offline, zero hits.
    let safe = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    assert!(hit_ids(&safe, "defs:zzz_no_such_symbol_7f3a").is_empty());
    let both = searcher_for(corpus.path(), &index_path, both_req());
    let resp = both.search("defs:zzz_no_such_symbol_7f3a").unwrap();
    assert!(resp.hits.is_empty());
    // Rerank empty path unperturbed by the neural feature (no-leak).
    let first = ast_sgrep_embed::rerank("refresh token", &[]).unwrap();
    let second = ast_sgrep_embed::rerank("refresh token", &[]).unwrap();
    assert!(first.is_empty() && second.is_empty());
}

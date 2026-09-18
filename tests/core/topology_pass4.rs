//! T4 topology pass 4: end-to-end drills for ast-sgrep-core features.
//!
//! T1 pinned the default-build surface, T2 the 2x2 gate matrix
//! (`neural-embed` x `rerank`), T3 the cross-set behavioral deltas. T4 pins
//! FULL index→search→fuse FLOWS per feature set in ONE file, on a multi-file
//! corpus T1-T3 never used:
//!
//! | drill | what is pinned |
//! |-------|----------------|
//! | full-flow equivalence | identical corpus + query battery (every local
//! | | search entry point) yields BIT-IDENTICAL local-path hit identities
//! | | under default / rerank / neural / all sets (one golden asserted in
//! | | every build); the explicit fuse stage (`apply_weighted_rrf` over
//! | | symbol+lexical hits) is golden-pinned too. |
//! | misuse per set | requesting neural/rerank behavior where the feature is
//! | | OFF fails closed at flow level (`Searcher::new` →
//! | | `StoreError::Other`); where ON, the request path RUNS offline per
//! | | contract (rowless corpus / proven-empty shortlist preconditions). |
//! | mixed-flow | local + gated requests interleaved on ONE shared index:
//! | | local battery bytes before == after, and equal the golden — gated
//! | | failures never poison the index or perturb local results. |
//!
//! Documented fail-closed discriminants (asserted, never message text):
//!
//! | request | OFF cell verdict | ON cell verdict |
//! |----------------------------|------------------|-------------------------------|
//! | neural (`use_embed`+`use_neural_embed`) | `Searcher::new` → `Err(Other)` | `Ok`; search runs offline on rowless store |
//! | rerank (`use_rerank`) | `Searcher::new` → `Err(Other)` | `Ok`; search runs offline on proven-empty shortlist |
//! | both flags | `Searcher::new` → `Err(Other)` | `Ok`; conjunction runs offline under both preconditions |
//! | stored `neural`/`fastembed` backend | `embed_query` → `Err` (any dim) | never probed (would load the model) |
//!
//! NOT duplicated from T1/T2/T3: single-gate validation discriminants, the
//! joint gate pair, cell mutual exclusion, per-cell gate pairs, `size_of`
//! presence, single-call empty rerank, `from_env` well-formedness, knob
//! self-consistency, single-input hashed determinism, model-table dims,
//! backend `parse` round-trips, single-file defs/callers/hybrid goldens,
//! neural-flag inertness, local vector agreement, normalization clamps,
//! validate-vs-construction equivalence, resolution surface, exact neural
//! fallback. Where T4 touches the same API it asserts a strictly stronger or
//! strictly different property: multi-file corpus, multi-entry battery,
//! explicit fuse-stage golden, flow-level (not validation-level) misuse,
//! same-index post-failure integrity, full-response (not hits-only) equality
//! on empty rerank shortlists, and the first `search_semantic` runs.
//!
//! Cells: exactly one is active per build (see T2
//! `matrix_cells_mutually_exclusive`). Universal `drill_*` tests run in ALL
//! cells by construction (same golden in every build); `cell_*` tests are
//! `#[cfg]`-gated to one exact cell and open with NEGATIVE cfg guards so
//! they FAIL loudly under the wrong feature set.
//!
//! OFFLINE POLICY: every test RUNS under every feature set
//! (default / `rerank` / `neural-embed` / `--all-features` /
//! `--no-default-features`). Model-load paths are never reached, by one of:
//!
//! - `use_embed = false`: every embed entry returns before embedding,
//!   regardless of neural flags or env.
//! - Rowless corpus: `indexed_corpus` sets `embed_semantic: false`, and neural
//!   RUN tests assert `store.semantic_sources_empty()` FIRST — every embed
//!   entry returns empty on a rowless store before the query is embedded.
//! - Proven-empty shortlist: rerank RUN tests prove zero hits on the safe
//!   (`use_rerank = false`) path FIRST; only then exercise `use_rerank = true`,
//!   where `maybe_rerank` early-returns before any model load.
//! - Stored neural backends are probed only in OFF cells (stub error, no
//!   load). `Neural`-preference chain calls never happen here.
//!
//! Deterministic, offline, tempfile fixtures, no new deps. Discriminant/value
//! assertions only — never message text. All `SearchOptions` are built field
//! by field (no `..Default`) so ambient `ASGREP_*` env cannot perturb them.

use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher, StoreError};
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

/// Multi-file indexed corpus with NO semantic rows (T1-T3 used a single
/// `auth.rs`; the second file exercises cross-file fusion ordering).
/// `embed_semantic: false` means no embedding at index time either.
fn indexed_corpus() -> (tempfile::TempDir, tempfile::TempDir, std::path::PathBuf) {
    let corpus = tempfile::tempdir().unwrap();
    let index_dir = tempfile::tempdir().unwrap();
    fs::write(
        corpus.path().join("auth.rs"),
        "fn refresh_token() {}\nfn caller() { refresh_token(); }\n",
    )
    .unwrap();
    fs::write(
        corpus.path().join("store.rs"),
        "fn persist_session() {}\nfn writer() { persist_session(); }\n",
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

/// Full-flow query battery: every local search entry point over the shared
/// corpus. One (label, hits) row per entry; the whole battery is golden-pinned
/// in every cell.
fn run_battery(searcher: &Searcher) -> Vec<(String, Vec<HitId>)> {
    let mut battery = Vec::new();
    for query in [
        "defs:refresh_token",
        "callers:refresh_token",
        "defs:persist_session",
        "callers:persist_session",
        "refresh_token",
        "persist_session",
    ] {
        let resp = searcher.search(query).unwrap();
        assert_eq!(resp.query, query);
        battery.push((
            format!("search:{query}"),
            resp.hits.iter().map(HitId::of).collect(),
        ));
    }
    for (label, resp) in [
        ("symbol", searcher.search_symbol_pass("refresh_token").unwrap()),
        ("lexical", searcher.search_lexical("refresh_token").unwrap()),
        ("literal", searcher.search_literal("refresh_token").unwrap()),
        ("word", searcher.search_word("persist_session").unwrap()),
        ("regex", searcher.search_regex("persist_.*").unwrap()),
        ("semantic", searcher.search_semantic("refresh token").unwrap()),
    ] {
        battery.push((
            label.to_string(),
            resp.hits.iter().map(HitId::of).collect(),
        ));
    }
    battery
}

// Cross-set goldens: filled from the discovery run, then pinned identical
// under all five feature sets.
#[allow(clippy::too_many_arguments)]
fn hit(
    kind: &str,
    file: &str,
    line_start: u32,
    line_end: u32,
    symbol: Option<&str>,
    caller: Option<&str>,
    callee: Option<&str>,
    score_bits: u64,
    signal: &str,
    excerpt: &str,
) -> HitId {
    HitId {
        kind: kind.to_string(),
        file: file.to_string(),
        line_start,
        line_end,
        symbol: symbol.map(str::to_string),
        caller: caller.map(str::to_string),
        callee: callee.map(str::to_string),
        score_bits,
        signal: signal.to_string(),
        excerpt: excerpt.to_string(),
    }
}

fn expected_battery() -> Vec<(String, Vec<HitId>)> {
    vec![
        (
            "search:defs:refresh_token".to_string(),
            vec![hit(
                "def",
                "auth.rs",
                1,
                1,
                Some("refresh_token"),
                None,
                None,
                4629770785681047552,
                "structural",
                "fn refresh_token() {}",
            )],
        ),
        (
            "search:callers:refresh_token".to_string(),
            vec![
                hit(
                    "caller",
                    "auth.rs",
                    2,
                    2,
                    None,
                    Some("caller"),
                    Some("refresh_token"),
                    4622663542519103488,
                    "structural",
                    "fn caller() { refresh_token(); }",
                ),
                hit(
                    "graph",
                    "auth.rs",
                    2,
                    2,
                    Some("refresh_token"),
                    Some("caller"),
                    Some("refresh_token"),
                    4617315517961601024,
                    "structural",
                    "caller calls refresh_token",
                ),
            ],
        ),
        (
            "search:defs:persist_session".to_string(),
            vec![hit(
                "def",
                "store.rs",
                1,
                1,
                Some("persist_session"),
                None,
                None,
                4629770785681047552,
                "structural",
                "fn persist_session() {}",
            )],
        ),
        (
            "search:callers:persist_session".to_string(),
            vec![
                hit(
                    "caller",
                    "store.rs",
                    2,
                    2,
                    None,
                    Some("writer"),
                    Some("persist_session"),
                    4622663542519103488,
                    "structural",
                    "fn writer() { persist_session(); }",
                ),
                hit(
                    "graph",
                    "store.rs",
                    2,
                    2,
                    Some("persist_session"),
                    Some("writer"),
                    Some("persist_session"),
                    4617315517961601024,
                    "structural",
                    "writer calls persist_session",
                ),
            ],
        ),
        (
            "search:refresh_token".to_string(),
            vec![
                hit(
                    "def",
                    "auth.rs",
                    1,
                    1,
                    Some("refresh_token"),
                    None,
                    None,
                    4593275893786714022,
                    "structural",
                    "fn refresh_token() {}",
                ),
                hit(
                    "caller",
                    "auth.rs",
                    2,
                    2,
                    Some("refresh_token"),
                    Some("caller"),
                    Some("refresh_token"),
                    4590475707484293538,
                    "structural",
                    "fn caller() { refresh_token(); }",
                ),
            ],
        ),
        (
            "search:persist_session".to_string(),
            vec![
                hit(
                    "def",
                    "store.rs",
                    1,
                    1,
                    Some("persist_session"),
                    None,
                    None,
                    4593275893786714022,
                    "structural",
                    "fn persist_session() {}",
                ),
                hit(
                    "caller",
                    "store.rs",
                    2,
                    2,
                    Some("persist_session"),
                    Some("writer"),
                    Some("persist_session"),
                    4590475707484293538,
                    "structural",
                    "fn writer() { persist_session(); }",
                ),
            ],
        ),
        (
            "symbol".to_string(),
            vec![
                hit(
                    "def",
                    "auth.rs",
                    1,
                    1,
                    Some("refresh_token"),
                    None,
                    None,
                    4632585535448154112,
                    "structural",
                    "fn refresh_token() {}",
                ),
                hit(
                    "caller",
                    "auth.rs",
                    2,
                    2,
                    None,
                    Some("caller"),
                    Some("refresh_token"),
                    4626181979727986688,
                    "structural",
                    "fn caller() { refresh_token(); }",
                ),
                hit(
                    "graph",
                    "auth.rs",
                    2,
                    2,
                    Some("refresh_token"),
                    Some("caller"),
                    Some("refresh_token"),
                    4617315517961601024,
                    "structural",
                    "caller calls refresh_token",
                ),
            ],
        ),
        (
            "lexical".to_string(),
            vec![
                hit(
                    "asgrep",
                    "auth.rs",
                    1,
                    1,
                    None,
                    None,
                    None,
                    4619068968636191996,
                    "exact",
                    "fn refresh_token() {}",
                ),
                hit(
                    "asgrep",
                    "auth.rs",
                    2,
                    2,
                    None,
                    None,
                    None,
                    4618949888794114510,
                    "exact",
                    "fn caller() { refresh_token(); }",
                ),
            ],
        ),
        (
            "literal".to_string(),
            vec![
                hit(
                    "asgrep",
                    "auth.rs",
                    1,
                    1,
                    None,
                    None,
                    None,
                    4607182418800017408,
                    "exact",
                    "fn refresh_token() {}",
                ),
                hit(
                    "asgrep",
                    "auth.rs",
                    2,
                    2,
                    None,
                    None,
                    None,
                    4607093238609376408,
                    "exact",
                    "fn caller() { refresh_token(); }",
                ),
            ],
        ),
        (
            "word".to_string(),
            vec![
                hit(
                    "asgrep",
                    "store.rs",
                    1,
                    1,
                    None,
                    None,
                    None,
                    4607182418800017408,
                    "exact",
                    "fn persist_session() {}",
                ),
                hit(
                    "asgrep",
                    "store.rs",
                    2,
                    2,
                    None,
                    None,
                    None,
                    4607093238609376408,
                    "exact",
                    "fn writer() { persist_session(); }",
                ),
            ],
        ),
        (
            "regex".to_string(),
            vec![
                hit(
                    "asgrep",
                    "store.rs",
                    1,
                    1,
                    None,
                    None,
                    None,
                    4606920073190656020,
                    "exact",
                    "fn persist_session() {}",
                ),
                hit(
                    "asgrep",
                    "store.rs",
                    2,
                    2,
                    None,
                    None,
                    None,
                    4606835988059450446,
                    "exact",
                    "fn writer() { persist_session(); }",
                ),
            ],
        ),
        ("semantic".to_string(), vec![]),
    ]
}

fn expected_fused() -> Vec<HitId> {
    vec![
        hit(
            "def",
            "auth.rs",
            1,
            1,
            Some("refresh_token"),
            None,
            None,
            4586775944422882899,
            "structural",
            "fn refresh_token() {}",
        ),
        hit(
            "caller",
            "auth.rs",
            2,
            2,
            Some("refresh_token"),
            Some("caller"),
            Some("refresh_token"),
            4586036696763265870,
            "structural",
            "fn caller() { refresh_token(); }",
        ),
    ]
}

// ---------------------------------------------------------------------------
// Universal drills (run in EVERY cell)
// ---------------------------------------------------------------------------

/// FULL-FLOW EQUIVALENCE: index→search over the multi-file corpus through
/// every local entry point yields the identical battery in every feature
/// set. T3 pinned single-file defs/callers/hybrid goldens; the multi-file
/// corpus, the extra modes (second symbol pair, symbol/lexical/literal/word/
/// regex/semantic entries), and the single-battery shape are new.
#[test]
fn drill_full_flow_battery_golden_identical_across_sets() {
    let (corpus, _index_dir, index_path) = indexed_corpus();
    let searcher = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    assert!(searcher.store().semantic_sources_empty().unwrap());
    let battery = run_battery(&searcher);
    assert_eq!(battery.len(), 12);
    // Every battery row is well-formed: finite scores, non-empty excerpts on
    // hits, files confined to the fixture pair.
    for (label, ids) in &battery {
        for id in ids {
            assert!(
                id.file == "auth.rs" || id.file == "store.rs",
                "row {label}: unexpected file {}",
                id.file
            );
            assert!(f64::from_bits(id.score_bits).is_finite(), "row {label}");
            assert!(!id.excerpt.is_empty(), "row {label}");
        }
    }
    // The defs rows must hit (the corpus defines both symbols); the semantic
    // row must be empty (embed off: no embed-kind hit under any set).
    for (label, ids) in &battery {
        if label.starts_with("search:defs:") {
            assert!(!ids.is_empty(), "row {label} must hit");
        }
        if label == "semantic" {
            assert!(ids.is_empty(), "embed off: semantic row must be empty");
        }
    }
    assert_eq!(battery, expected_battery(), "battery in cell {}", active_cell());
}

/// FUSE-STAGE EQUIVALENCE: the explicit fuse stage — symbol+lexical hits
/// fused via `apply_weighted_rrf` with intent weights — yields the identical
/// fused order in every feature set, is repeat-deterministic, and the pure
/// RRF math takes exact values. T1-T3 never touched the fuse API.
#[test]
fn drill_fuse_stage_explicit_rrf_deterministic_across_sets() {
    use ast_sgrep_core::fusion::{apply_weighted_rrf, weighted_rrf_score, ChannelRanks};
    use ast_sgrep_core::intent::{classify, weights_for};
    use ast_sgrep_core::rank::{fuse_rrf, rrf_score, RRF_K};
    use ast_sgrep_core::ParsedQuery;

    // Pure RRF math: exact values (1/(k+rank+1)), every set.
    assert_eq!(RRF_K, 60.0);
    assert_eq!(rrf_score(0, RRF_K), 1.0 / 61.0);
    assert_eq!(rrf_score(1, RRF_K), 1.0 / 62.0);
    assert_eq!(fuse_rrf(&[0, 1], RRF_K), 1.0 / 61.0 + 1.0 / 62.0);
    assert_eq!(fuse_rrf(&[], RRF_K), 0.0);

    let (corpus, _index_dir, index_path) = indexed_corpus();
    let searcher = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    let parsed = ParsedQuery::parse("refresh_token");
    let weights = weights_for(classify(&parsed));

    // Weighted RRF score laws (property assertions on the fuse math): empty
    // ranks score exactly zero, better ranks score strictly higher, and an
    // extra channel never lowers the score.
    assert_eq!(weighted_rrf_score(&ChannelRanks::default(), &weights), 0.0);
    let rank0 = ChannelRanks {
        lexical: Some(0),
        ..ChannelRanks::default()
    };
    let rank5 = ChannelRanks {
        lexical: Some(5),
        ..ChannelRanks::default()
    };
    assert!(weighted_rrf_score(&rank0, &weights) > weighted_rrf_score(&rank5, &weights));
    let rank0_plus = ChannelRanks {
        lexical: Some(0),
        definition: Some(0),
        ..ChannelRanks::default()
    };
    assert!(weighted_rrf_score(&rank0_plus, &weights) >= weighted_rrf_score(&rank0, &weights));

    // Full fuse flow: gather two channels, fuse, pin the fused order.
    let mut hits = searcher.search_symbol_pass("refresh_token").unwrap().hits;
    hits.extend(searcher.search_lexical("refresh_token").unwrap().hits);
    assert!(!hits.is_empty(), "fuse drill needs channel hits");
    let mut first = hits.clone();
    apply_weighted_rrf(&mut first, &weights);
    let mut second = hits.clone();
    apply_weighted_rrf(&mut second, &weights);
    let encode = |hs: &[ast_sgrep_core::SearchHit]| hs.iter().map(HitId::of).collect::<Vec<_>>();
    assert_eq!(encode(&first), encode(&second), "fuse must be repeat-deterministic");
    assert_eq!(encode(&first), expected_fused(), "fused order in cell {}", active_cell());
}

/// MIXED-FLOW: local + gated requests interleaved on ONE shared index. Gated
/// constructions resolve per the ambient feature set (closed → `Other`
/// discriminant); the local battery before == after, and equals the golden —
/// gated failures never poison the index or perturb local results. The
/// sequencing property (before/after equality across interleaved failures)
/// is new in T4.
#[test]
fn drill_mixed_flow_local_unaffected_by_gated_requests() {
    let (corpus, _index_dir, index_path) = indexed_corpus();
    let local = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    let before = run_battery(&local);
    assert_eq!(before, expected_battery());

    // Interleaved gated constructions: verdict matches the ambient set.
    for (opts, open, name) in [
        (neural_req(), cfg!(feature = "neural-embed"), "neural"),
        (rerank_req(), cfg!(feature = "rerank"), "rerank"),
        (
            both_req(),
            cfg!(all(feature = "neural-embed", feature = "rerank")),
            "both",
        ),
    ] {
        let mut opts = opts;
        opts.root = corpus.path().to_path_buf();
        opts.index_path = Some(index_path.clone());
        let result = Searcher::new(opts);
        assert_eq!(result.is_ok(), open, "{name} gate in cell {}", active_cell());
        if let Err(e) = result {
            assert!(
                matches!(e, StoreError::Other(_)),
                "{name} must fail closed with Other"
            );
        }
    }
    // Embed-level gated probe, offline-safe in every cell: the stored-neural
    // probe runs only where the stub answers without a load.
    #[cfg(not(feature = "neural-embed"))]
    {
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

    // Local flow after the interleaved gated requests: byte-identical.
    let after = run_battery(&local);
    assert_eq!(before, after, "gated requests must not perturb local results");
    assert_eq!(after, expected_battery(), "post-failure golden in cell {}", active_cell());
}

// ---------------------------------------------------------------------------
// Cell drills (one exact cell each, negative guards first)
// ---------------------------------------------------------------------------

/// DEFAULT cell, full-flow misuse: every gated flow fails closed at
/// construction with the documented discriminant, and the full local battery
/// still serves the golden on the SAME index after the failures (mixed-flow
/// integrity). T2 pinned construction verdicts; the full-flow framing plus
/// post-failure battery integrity is new.
#[cfg(not(any(feature = "neural-embed", feature = "rerank")))]
#[test]
fn cell_default_full_flow_misuse_fails_closed() {
    assert!(!cfg!(feature = "neural-embed"), "default cell requires neural-embed OFF");
    assert!(!cfg!(feature = "rerank"), "default cell requires rerank OFF");
    let (corpus, _index_dir, index_path) = indexed_corpus();
    for (mut opts, name) in [
        (neural_req(), "neural"),
        (rerank_req(), "rerank"),
        (both_req(), "both"),
    ] {
        opts.root = corpus.path().to_path_buf();
        opts.index_path = Some(index_path.clone());
        let result = Searcher::new(opts);
        assert!(result.is_err(), "{name} flow must fail closed");
        if let Err(e) = result {
            assert!(matches!(e, StoreError::Other(_)), "{name} discriminant");
        }
    }
    // Stored neural spellings unresolvable at any dim (no load: stub error).
    for backend in ["neural", "fastembed"] {
        for dim in [0, 384] {
            assert!(
                ast_sgrep_embed::embed_query(
                    "refresh token",
                    Some(backend),
                    dim,
                    ast_sgrep_embed::EmbedPreference::Auto,
                )
                .is_err(),
                "backend {backend} dim {dim} must be rejected"
            );
        }
    }
    // Same index still serves the full local golden after the failures.
    let local = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    assert_eq!(run_battery(&local), expected_battery());
}

/// NEURAL-ONLY cell: rerank misuse fails closed; the neural request path RUNS
/// the full battery offline with the local golden intact, and the semantic
/// pass runs empty on the rowless store. T3 ran 3 queries on the single-file
/// corpus; the 12-row multi-file battery plus the first `search_semantic`
/// neural-open run are new.
#[cfg(all(feature = "neural-embed", not(feature = "rerank")))]
#[test]
fn cell_neural_only_full_flow_neural_runs_rerank_closed() {
    assert!(cfg!(feature = "neural-embed"), "neural-only cell requires neural-embed ON");
    assert!(!cfg!(feature = "rerank"), "neural-only cell requires rerank OFF");
    let (corpus, _index_dir, index_path) = indexed_corpus();
    // Load-free precondition, proven before any neural search.
    let probe = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    assert!(probe.store().semantic_sources_empty().unwrap());
    // Rerank misuse fails closed (both the single and conjunction flows).
    for (mut opts, name) in [(rerank_req(), "rerank"), (both_req(), "both")] {
        opts.root = corpus.path().to_path_buf();
        opts.index_path = Some(index_path.clone());
        let result = Searcher::new(opts);
        assert!(result.is_err(), "{name} flow must fail closed");
        if let Err(e) = result {
            assert!(matches!(e, StoreError::Other(_)), "{name} discriminant");
        }
    }
    // Neural-open flow RUNS the full battery offline: local golden intact.
    let neural = searcher_for(corpus.path(), &index_path, neural_req());
    assert_eq!(run_battery(&neural), expected_battery());
    // Semantic pass on the rowless store: Ok + empty, no neural load.
    let resp = neural.search_semantic("refresh token").unwrap();
    assert!(resp.hits.is_empty());
    // Mixed interleave: local golden intact after gated runs and failures.
    assert_eq!(run_battery(&probe), expected_battery());
}

/// RERANK-ONLY cell: neural misuse fails closed; the rerank request path RUNS
/// offline on proven-empty shortlists with FULL response equality to the safe
/// path (query/limit/hits/counts — strictly stronger than T3's hits-empty).
/// New entry point in the empty drill: the literal pass.
#[cfg(all(feature = "rerank", not(feature = "neural-embed")))]
#[test]
fn cell_rerank_only_full_flow_rerank_runs_neural_closed() {
    assert!(cfg!(feature = "rerank"), "rerank-only cell requires rerank ON");
    assert!(!cfg!(feature = "neural-embed"), "rerank-only cell requires neural-embed OFF");
    let (corpus, _index_dir, index_path) = indexed_corpus();
    // Neural misuse fails closed (both the single and conjunction flows).
    for (mut opts, name) in [(neural_req(), "neural"), (both_req(), "both")] {
        opts.root = corpus.path().to_path_buf();
        opts.index_path = Some(index_path.clone());
        let result = Searcher::new(opts);
        assert!(result.is_err(), "{name} flow must fail closed");
        if let Err(e) = result {
            assert!(matches!(e, StoreError::Other(_)), "{name} discriminant");
        }
    }
    let safe = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    let ranked = searcher_for(corpus.path(), &index_path, rerank_req());
    // Proven-empty shortlists across dispatch + literal entries: safe path
    // proves empty FIRST, then the ranked flow runs offline with full
    // response equality (rerank must not perturb an empty shortlist).
    for query in [
        "defs:zzz_no_such_symbol_7f3a",
        "callers:zzz_no_such_symbol_7f3a",
        // Bare-hybrid empty: no term may substring-match any fixture symbol
        // ("token" would match "refresh_token" via substring scoring).
        "zzzqqq_wwvxxx",
    ] {
        let safe_resp = safe.search(query).unwrap();
        assert!(safe_resp.hits.is_empty(), "fixture must not match {query:?}");
        let ranked_resp = ranked.search(query).unwrap();
        assert_eq!(ranked_resp.query, safe_resp.query);
        assert_eq!(ranked_resp.limit, safe_resp.limit);
        assert_eq!(ranked_resp.hits.len(), safe_resp.hits.len());
        assert_eq!(ranked_resp.counts, safe_resp.counts);
    }
    let safe_lit = safe.search_literal("zzz_no_such_literal_7f3a").unwrap();
    assert!(safe_lit.hits.is_empty());
    let ranked_lit = ranked.search_literal("zzz_no_such_literal_7f3a").unwrap();
    assert_eq!(ranked_lit.query, safe_lit.query);
    assert_eq!(ranked_lit.limit, safe_lit.limit);
    assert!(ranked_lit.hits.is_empty());
    // Mixed interleave: local golden intact after gated runs and failures.
    assert_eq!(run_battery(&safe), expected_battery());
}

/// ALL-FEATURES cell: every gated flow opens and RUNS offline — the neural
/// searcher serves the full local battery golden (rowless precondition), the
/// conjunction searcher matches the safe path on proven-empty shortlists, and
/// the semantic pass runs empty. Mixed interleave closes with the local
/// golden intact. The 12-row battery + full-response conjunction equality are
/// new over T3's 3-query run.
#[cfg(all(feature = "neural-embed", feature = "rerank"))]
#[test]
fn cell_all_features_full_flow_conjunction_runs_offline() {
    assert!(cfg!(feature = "neural-embed"), "all-features cell requires neural-embed ON");
    assert!(cfg!(feature = "rerank"), "all-features cell requires rerank ON");
    let (corpus, _index_dir, index_path) = indexed_corpus();
    let probe = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    assert!(probe.store().semantic_sources_empty().unwrap());
    // Every gated flow opens at construction.
    for (mut opts, name) in [
        (neural_req(), "neural"),
        (rerank_req(), "rerank"),
        (both_req(), "both"),
    ] {
        opts.root = corpus.path().to_path_buf();
        opts.index_path = Some(index_path.clone());
        assert!(Searcher::new(opts).is_ok(), "{name} flow must open");
    }
    // Neural-open flow: full battery golden intact (no neural load: rowless).
    let neural = searcher_for(corpus.path(), &index_path, neural_req());
    assert_eq!(run_battery(&neural), expected_battery());
    // Semantic pass on the rowless store: Ok + empty, no neural load.
    assert!(neural.search_semantic("refresh token").unwrap().hits.is_empty());
    // Conjunction flow on proven-empty shortlists: full response equality.
    let both = searcher_for(corpus.path(), &index_path, both_req());
    for query in ["defs:zzz_no_such_symbol_7f3a", "zzzqqq_wwvxxx"] {
        let safe_resp = probe.search(query).unwrap();
        assert!(safe_resp.hits.is_empty(), "fixture must not match {query:?}");
        let both_resp = both.search(query).unwrap();
        assert_eq!(both_resp.query, safe_resp.query);
        assert_eq!(both_resp.limit, safe_resp.limit);
        assert_eq!(both_resp.hits.len(), safe_resp.hits.len());
        assert_eq!(both_resp.counts, safe_resp.counts);
    }
    // Mixed interleave: local golden intact at the end.
    assert_eq!(run_battery(&probe), expected_battery());
}

//! Topology CORE runs: search goldens and end-to-end flow drills as intents.
//!
//! Consolidates the behavioral deltas of `topology_pass3.rs` (T3) and the
//! flow drills of `topology_pass4.rs` (T4) into 7 tests, one intent each:
//! single-file goldens, multi-file battery, fuse stage, mixed-flow
//! sequencing (all universal), plus one RUN cell per heavy feature
//! combination. T3's single-file RUN coverage is absorbed INTO the T4 cell
//! drills as extra facets on the single-file corpus (the corpus contract
//! differs, so the facet stays — same test, same cell).
//!
//! OFFLINE POLICY: every test RUNS under every feature set. Model-load paths
//! are never reached, by one of: `use_embed = false` (every embed entry
//! returns before embedding); rowless corpus (`semantic_sources_empty`
//! asserted FIRST, embed entries return empty before the query is embedded);
//! proven-empty shortlist (emptiness proven on the safe path FIRST, then the
//! gated path runs where `maybe_rerank` early-returns). Stored neural
//! backends are never probed here (that matrix lives in `topology_local`).
//! Discriminant/value assertions only, never message text. All options are
//! built field by field (no `..Default`) so ambient `ASGREP_*` env cannot
//! perturb them.

use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher, StoreError};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Active matrix cell, for failure messages on cross-set goldens.
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

/// Single-file indexed corpus with NO semantic rows (T3's `auth.rs`).
/// `embed_semantic: false` means no embedding at index time either. Corpus
/// dir via `testkit::file_tree` (shared seam); the index dir stays a bare
/// TempDir (no testkit helper pairs a private corpus with a private index).
fn indexed_corpus_1file() -> (tempfile::TempDir, tempfile::TempDir, std::path::PathBuf) {
    let corpus = ast_sgrep_testkit::file_tree(&[(
        "auth.rs",
        "fn refresh_token() {}\nfn caller() { refresh_token(); }\n",
    )]);
    indexed_over(corpus)
}

/// Multi-file indexed corpus with NO semantic rows (T4's auth.rs + store.rs;
/// the second file exercises cross-file fusion ordering).
fn indexed_corpus_2file() -> (tempfile::TempDir, tempfile::TempDir, std::path::PathBuf) {
    let corpus = ast_sgrep_testkit::file_tree(&[
        (
            "auth.rs",
            "fn refresh_token() {}\nfn caller() { refresh_token(); }\n",
        ),
        (
            "store.rs",
            "fn persist_session() {}\nfn writer() { persist_session(); }\n",
        ),
    ]);
    indexed_over(corpus)
}

fn indexed_over(
    corpus: tempfile::TempDir,
) -> (tempfile::TempDir, tempfile::TempDir, std::path::PathBuf) {
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

/// Owned hit identity for cross-set goldens. Scores as bits: the local path
/// must be bit-identical under every feature set. Snapshot fields (mtime
/// derived) are deliberately excluded — hits only. File-local because
/// testkit's `response_hit_keys` drops scores/signal/excerpts by design,
/// which is exactly what these goldens pin.
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
// Universal goldens (run in EVERY cell; same golden in every build)
// ---------------------------------------------------------------------------

/// INTENT: single-file local search hit identities are bit-identical under
/// every feature set, and with embed off no embed-kind hit can appear.
/// Facets: defs golden, callers golden, hybrid golden, embed-off purity.
/// Absorbs T3#1, T3#2, T3#3 (one intent — local-path equivalence — with
/// three query-shape facets; the values coincide with the battery rows,
/// which is the IDF tripwire, not duplication).
/// KILLS: ranking-perturbation, embed-leak.
#[test]
fn single_file_search_goldens_identical_across_sets() {
    let (corpus, _index_dir, index_path) = indexed_corpus_1file();
    let searcher = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    // Defs facet.
    assert_eq!(
        searcher.search("defs:refresh_token").unwrap().query,
        "defs:refresh_token"
    );
    let ids = hit_ids(&searcher, "defs:refresh_token");
    assert!(!ids.is_empty(), "defs query must hit");
    for id in &ids {
        assert_eq!(id.file, "auth.rs");
        assert_eq!(id.kind, "def");
        assert_eq!(id.signal, "structural");
        assert!(f64::from_bits(id.score_bits).is_finite());
        assert!(!id.excerpt.is_empty());
    }
    assert_eq!(
        ids,
        expected_defs(),
        "defs golden in cell {}",
        active_cell()
    );
    // Callers facet.
    assert_eq!(
        searcher.search("callers:refresh_token").unwrap().query,
        "callers:refresh_token"
    );
    let ids = hit_ids(&searcher, "callers:refresh_token");
    assert!(!ids.is_empty(), "callers query must hit");
    for id in &ids {
        assert_eq!(id.file, "auth.rs");
        assert!(
            id.kind == "caller" || id.kind == "graph",
            "unexpected kind {}",
            id.kind
        );
        assert_eq!(id.signal, "structural");
        assert!(f64::from_bits(id.score_bits).is_finite());
    }
    assert_eq!(
        ids,
        expected_callers(),
        "callers golden in cell {}",
        active_cell()
    );
    // Hybrid facet + embed-off purity.
    assert_eq!(
        searcher.search("refresh_token").unwrap().query,
        "refresh_token"
    );
    let ids = hit_ids(&searcher, "refresh_token");
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
        (
            "symbol",
            searcher.search_symbol_pass("refresh_token").unwrap(),
        ),
        ("lexical", searcher.search_lexical("refresh_token").unwrap()),
        ("literal", searcher.search_literal("refresh_token").unwrap()),
        ("word", searcher.search_word("persist_session").unwrap()),
        ("regex", searcher.search_regex("persist_.*").unwrap()),
        (
            "semantic",
            searcher.search_semantic("refresh token").unwrap(),
        ),
    ] {
        battery.push((label.to_string(), resp.hits.iter().map(HitId::of).collect()));
    }
    battery
}

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

/// INTENT: index→search over the multi-file corpus through every local entry
/// point yields the identical 12-row battery in every feature set.
/// Facets: row well-formedness (files/scores/excerpts); defs rows hit;
/// semantic row empty (embed-off purity); whole-battery golden equality.
/// Absorbs T4#1 (battery golden; top consolidation target).
/// KILLS: ranking-perturbation, entry-divergence.
#[test]
fn multi_file_battery_golden_identical_across_sets() {
    let (corpus, _index_dir, index_path) = indexed_corpus_2file();
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
    assert_eq!(
        battery,
        expected_battery(),
        "battery in cell {}",
        active_cell()
    );
}

/// INTENT: the explicit fuse stage — symbol+lexical hits fused via
/// `apply_weighted_rrf` with intent weights — is exact, lawful, and
/// repeat-deterministic in every feature set.
/// Facets: pure RRF math exact values; weighted score laws (empty=0,
/// better-rank-wins, extra-channel-monotone); fused-order golden; repeat
/// determinism.
/// Absorbs T4#2 (only fuse API pin).
/// KILLS: fuse-regression, nondeterminism.
#[test]
fn fuse_stage_explicit_rrf_deterministic_across_sets() {
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

    let (corpus, _index_dir, index_path) = indexed_corpus_2file();
    let searcher = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    let parsed = ParsedQuery::parse("refresh_token");
    let weights = weights_for(classify(&parsed));

    // Weighted RRF score laws: empty ranks score exactly zero, better ranks
    // score strictly higher, and an extra channel never lowers the score.
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
    assert_eq!(
        encode(&first),
        encode(&second),
        "fuse must be repeat-deterministic"
    );
    assert_eq!(
        encode(&first),
        expected_fused(),
        "fused order in cell {}",
        active_cell()
    );
}

/// INTENT: local + gated requests interleaved on ONE shared index never
/// poison the index or perturb local results, in any cell.
/// Facets: local battery before == golden; interleaved gated constructions
/// resolve per the ambient set with the `Other` discriminant on closed;
/// local battery after == before == golden (post-failure integrity).
/// Absorbs T4#3 (only sequencing pin) and the T4#4 default-cell construction
/// + post-failure atoms (its stored-backend matrix lives in
/// `topology_local`, with a wider two-cell span).
/// KILLS: index-poisoning, gate-inversion.
#[test]
fn mixed_flow_local_unaffected_by_gated_requests() {
    let (corpus, _index_dir, index_path) = indexed_corpus_2file();
    let local = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    let before = run_battery(&local);
    assert_eq!(before, expected_battery());

    // Interleaved gated constructions: verdict matches the ambient set
    // (construction only — never `.search()` with open optional flags).
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
        assert_eq!(
            result.is_ok(),
            open,
            "{name} gate in cell {}",
            active_cell()
        );
        if let Err(e) = result {
            assert!(
                matches!(e, StoreError::Other(_)),
                "{name} must fail closed with Other"
            );
        }
    }

    // Local flow after the interleaved gated requests: byte-identical.
    let after = run_battery(&local);
    assert_eq!(
        before, after,
        "gated requests must not perturb local results"
    );
    assert_eq!(
        after,
        expected_battery(),
        "post-failure golden in cell {}",
        active_cell()
    );
}

// ---------------------------------------------------------------------------
// Cell drills (one exact cell each, negative guards first)
// ---------------------------------------------------------------------------

/// INTENT: in the neural-only cell the neural request path RUNS offline with
/// the local golden intact while the rerank half stays fail-closed.
/// Facets: rowless precondition proven first; single-file 3-query goldens
/// (T3 corpus contract) + multi-file 12-row battery (T4 corpus contract) on
/// neural-open flows; semantic pass Ok + empty; rerank/both constructions
/// fail closed with `Other`; local golden intact after the interleave.
/// Absorbs T3#10 (single-file neural RUN) and T4#5 (neural-open battery RUN).
/// No load: every embed entry returns empty on the rowless store before the
/// query is embedded (preference/env moot past that point).
/// KILLS: load-on-rowless, ranking-perturbation, gate-inversion.
#[cfg(all(feature = "neural-embed", not(feature = "rerank")))]
#[test]
fn neural_cell_runs_local_rerank_closed() {
    // Negative guards: exact cell — neural ON, rerank OFF.
    assert!(
        cfg!(feature = "neural-embed"),
        "neural-only cell requires neural-embed ON"
    );
    assert!(
        !cfg!(feature = "rerank"),
        "neural-only cell requires rerank OFF"
    );
    // Single-file corpus facet (T3 contract): neural RUN offline, goldens intact.
    let (corpus1, _i1, index1) = indexed_corpus_1file();
    let probe1 = searcher_for(corpus1.path(), &index1, hermetic_local_options());
    assert!(probe1.store().semantic_sources_empty().unwrap());
    let neural1 = searcher_for(corpus1.path(), &index1, neural_req());
    assert_eq!(hit_ids(&neural1, "defs:refresh_token"), expected_defs());
    assert_eq!(
        hit_ids(&neural1, "callers:refresh_token"),
        expected_callers()
    );
    assert_eq!(hit_ids(&neural1, "refresh_token"), expected_hybrid());
    // Auto preference on the rowless store agrees too (still no embed reached).
    let mut auto_req = hermetic_local_options();
    auto_req.use_embed = true;
    let auto_searcher = searcher_for(corpus1.path(), &index1, auto_req);
    assert_eq!(
        hit_ids(&auto_searcher, "defs:refresh_token"),
        expected_defs()
    );
    // Multi-file corpus facet (T4 contract): full battery on neural-open flow.
    let (corpus, _index_dir, index_path) = indexed_corpus_2file();
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

/// INTENT: in the rerank-only cell the rerank request path RUNS offline on
/// proven-empty shortlists with full-response equality while neural stays
/// fail-closed.
/// Facets: safe path proves empty FIRST, ranked flow runs with query/limit/
/// counts equality across dispatch + literal entries; empty-docs rerank
/// query-independent + repeat-deterministic; neural/both constructions fail
/// closed with `Other`; local battery golden intact after.
/// Absorbs T3#11 (empty-shortlist RUN + direct empty-docs atoms) and T4#6
/// (full-response equality drill). No load: `maybe_rerank` early-returns on
/// empty hits before any model load; `rerank` only ever sees empty docs.
/// KILLS: load-on-empty, nondeterminism, gate-inversion.
#[cfg(all(feature = "rerank", not(feature = "neural-embed")))]
#[test]
fn rerank_cell_runs_empty_neural_closed() {
    // Negative guards: exact cell — rerank ON, neural OFF.
    assert!(
        cfg!(feature = "rerank"),
        "rerank-only cell requires rerank ON"
    );
    assert!(
        !cfg!(feature = "neural-embed"),
        "rerank-only cell requires neural-embed OFF"
    );
    let (corpus, _index_dir, index_path) = indexed_corpus_2file();
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
        assert!(
            safe_resp.hits.is_empty(),
            "fixture must not match {query:?}"
        );
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
    // Empty-docs rerank: query-independent, repeat-deterministic, always Ok
    // (early return before any model load).
    for query in ["refresh token", "zzz nothing matches this", ""] {
        let first = ast_sgrep_embed::rerank(query, &[]).unwrap();
        let second = ast_sgrep_embed::rerank(query, &[]).unwrap();
        assert!(first.is_empty() && second.is_empty());
    }
    // Mixed interleave: local golden intact after gated runs and failures.
    assert_eq!(run_battery(&safe), expected_battery());
}

/// INTENT: in the all-features cell every gated flow opens and RUNS offline,
/// with no leak in either direction.
/// Facets: all three gated flows open at construction; neural-open flow
/// serves the full battery golden (rowless); semantic pass Ok + empty;
/// conjunction matches the safe path on proven-empty shortlists (full
/// response equality); empty-docs rerank unperturbed by neural; local
/// golden intact at the end.
/// Absorbs T3#12 (conjunction RUN + no-leak atoms) and T4#7 (conjunction
/// flow drill). No load: rowless store + proven-empty shortlists.
/// KILLS: load-on-empty, leak-across-sets, gate-inversion.
#[cfg(all(feature = "neural-embed", feature = "rerank"))]
#[test]
fn all_features_cell_conjunction_runs_offline() {
    // Negative guards: exact cell — BOTH features ON.
    assert!(
        cfg!(feature = "neural-embed"),
        "all-features cell requires neural-embed ON"
    );
    assert!(
        cfg!(feature = "rerank"),
        "all-features cell requires rerank ON"
    );
    let (corpus, _index_dir, index_path) = indexed_corpus_2file();
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
    assert!(neural
        .search_semantic("refresh token")
        .unwrap()
        .hits
        .is_empty());
    // Conjunction flow on proven-empty shortlists: full response equality.
    let both = searcher_for(corpus.path(), &index_path, both_req());
    for query in ["defs:zzz_no_such_symbol_7f3a", "zzzqqq_wwvxxx"] {
        let safe_resp = probe.search(query).unwrap();
        assert!(
            safe_resp.hits.is_empty(),
            "fixture must not match {query:?}"
        );
        let both_resp = both.search(query).unwrap();
        assert_eq!(both_resp.query, safe_resp.query);
        assert_eq!(both_resp.limit, safe_resp.limit);
        assert_eq!(both_resp.hits.len(), safe_resp.hits.len());
        assert_eq!(both_resp.counts, safe_resp.counts);
    }
    // Rerank empty path unperturbed by the neural feature (no-leak), and
    // repeat-deterministic.
    let first = ast_sgrep_embed::rerank("refresh token", &[]).unwrap();
    let second = ast_sgrep_embed::rerank("refresh token", &[]).unwrap();
    assert!(first.is_empty() && second.is_empty());
    // Mixed interleave: local golden intact at the end.
    assert_eq!(run_battery(&probe), expected_battery());
}

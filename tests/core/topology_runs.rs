//! Topology CORE runs: search goldens and end-to-end flow drills as intents.
//!
//! Single-file goldens, multi-file battery, fuse stage, and mixed-flow
//! sequencing. Phase 2 (br-6oh) deleted the feature matrix and its per-cell
//! drills; every test here is universal. All corpora are rowless
//! (`embed_semantic: false`) and all searches run with `use_embed = false`
//! unless noted, so nothing embeds. Discriminant/value assertions only,
//! never message text. All options are built field by field (no
//! `..Default`) so ambient `ASGREP_*` env cannot perturb them.

use ast_sgrep_core::{IndexOptions, Indexer, SearchOptions, Searcher};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build label for failure messages on cross-run goldens. Phase 2 (br-6oh)
/// deleted the feature matrix; there is one build shape now.
fn active_cell() -> &'static str {
    "single-engine"
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
        use_semantic_only: false,
        use_repository_vocabulary: true,
        ann_threshold: None,
        ann_probes: None,
        case_insensitive: false,
        context_before: 0,
        context_after: 0,
        count_only: false,
        file_filter: None,
    }
}

fn embed_req() -> SearchOptions {
    let mut opts = hermetic_local_options();
    opts.use_embed = true;
    opts
}

fn semantic_only_req() -> SearchOptions {
    let mut opts = embed_req();
    opts.use_semantic_only = true;
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

/// INTENT: embed-enabled requests interleaved on ONE shared index never
/// poison the index or perturb local results.
/// Facets: local battery before == golden; interleaved embed/semantic-only
/// constructions succeed; local battery after == before == golden.
/// KILLS: index-poisoning.
#[test]
fn mixed_flow_local_unaffected_by_embed_requests() {
    let (corpus, _index_dir, index_path) = indexed_corpus_2file();
    let local = searcher_for(corpus.path(), &index_path, hermetic_local_options());
    let before = run_battery(&local);
    assert_eq!(before, expected_battery());

    // Interleaved embed-enabled constructions on the same index.
    for (opts, name) in [
        (embed_req(), "embed"),
        (semantic_only_req(), "semantic-only"),
    ] {
        let mut opts = opts;
        opts.root = corpus.path().to_path_buf();
        opts.index_path = Some(index_path.clone());
        assert!(
            Searcher::new(opts).is_ok(),
            "{name} construction must succeed"
        );
    }

    // Local flow after the interleaved requests: byte-identical.
    let after = run_battery(&local);
    assert_eq!(
        before, after,
        "embed requests must not perturb local results"
    );
    assert_eq!(
        after,
        expected_battery(),
        "post-interleave golden in cell {}",
        active_cell()
    );
}

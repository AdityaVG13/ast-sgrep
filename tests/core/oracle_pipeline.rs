//! Core end-to-end pipeline oracles (consolidated).
//!
//! Consolidates the CAT=e2e facets of `oracle_foundry_pass4` (real tempdir
//! corpus → `Indexer::index_all` → `Searcher::search` → merged hits with
//! signal/confidence provenance and snapshot stamps) into 3 intent-grouped
//! tests. Every expectation is hand-computed from fixture contents; failures
//! assert discriminants, never message text.

use ast_sgrep_core::{
    HitKind, Indexer, Searcher, StoreError, INDEX_CANCELLED, INDEX_SCHEMA_VERSION,
};
use ast_sgrep_testkit::{
    build_core_index, core_index_options, core_search_options, core_searcher,
    response_hit_keys_with_scores, sorted_hit_files, write_core_fixture,
};
use std::fs;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// INTENT: full pipelines cite indexed evidence end to end — defs/callers
/// round-trip, hybrid merges with per-hit signal provenance,
/// schema-stamped snapshots, limit shaping with echo, subtree filters, and
/// hand-computed count-only tables.
/// KILLS: BEHAVIOR-ONLY plus limit-ignored, filter-ignored, error-on-empty,
/// and count-aggregation mutants.
/// ABSORBS: pipeline_defs_and_callers_cite_indexed_symbols,
/// pipeline_hybrid_merges_evidence_with_signal_provenance,
/// pipeline_snapshot_stamp_provenance, pipeline_limit_shapes_results_end_to_end,
/// pipeline_file_filter_keeps_matching_subtree,
/// pipeline_count_only_reports_hand_computed_counts.
#[test]
fn pipeline_evidence_roundtrip_shaping_and_counts() {
    // Facet 1: build → prefixed search round-trips the indexed symbols.
    let fixture = write_core_fixture(&[(
        "lib.rs",
        "fn refresh_token() {}\nfn caller_one() { refresh_token(); }\n",
    )]);
    drop(build_core_index(&fixture));
    let prefixed = core_searcher(&fixture, 16);

    let defs = prefixed.search("defs:refresh_token").expect("defs search");
    let def = defs
        .hits
        .iter()
        .find(|hit| hit.kind == HitKind::Def && hit.symbol.as_deref() == Some("refresh_token"))
        .expect("Def hit must cite the indexed symbol");
    assert!(def.score > 0.0);
    assert_eq!(def.file, "lib.rs");

    let callers = prefixed.search("callers:refresh_token").expect("callers search");
    let caller = callers
        .hits
        .iter()
        .find(|hit| hit.kind == HitKind::Caller && hit.callee.as_deref() == Some("refresh_token"))
        .expect("Caller hit must cite the indexed callee");
    assert!(caller.score > 0.0);
    assert_eq!(caller.file, "lib.rs");

    // Facet 2: hybrid search merges evidence; every hit carries signal provenance.
    let fixture = write_core_fixture(&[(
        "auth.rs",
        "fn auth_refresh() {}\nfn login() { auth_refresh(); }\n",
    )]);
    drop(build_core_index(&fixture));
    let hybrid = core_searcher(&fixture, 16).search("auth_refresh").expect("hybrid search");

    assert!(!hybrid.hits.is_empty(), "hybrid must surface the def");
    assert!(
        hybrid.hits.iter().any(|hit| hit.kind == HitKind::Def
            && hit.symbol.as_deref() == Some("auth_refresh")),
        "hybrid must include the Def hit; got {:?}",
        hybrid.hits.iter().map(|hit| (hit.kind.as_str(), hit.symbol.clone())).collect::<Vec<_>>()
    );
    // Pipeline-wide provenance invariant: no anonymous evidence survives.
    for hit in &hybrid.hits {
        assert!(!hit.contributors.is_empty(), "contributors must be non-empty");
        assert_eq!(hit.signal, hit.kind.signal());
        assert!(hit.confidence > 0.0 && hit.confidence <= 1.0);
    }

    // Facet 3: responses are stamped with schema provenance, not git state.
    let fixture = write_core_fixture(&[("a.rs", "fn stamped_symbol() {}\n")]);
    drop(build_core_index(&fixture));
    let response = core_searcher(&fixture, 16)
        .search("defs:stamped_symbol")
        .expect("search");
    assert!(!response.hits.is_empty());
    assert_eq!(response.snapshot.schema_version, INDEX_SCHEMA_VERSION);
    // A tempdir corpus is not a git worktree: no HEAD is claimed.
    assert_eq!(response.snapshot.git_head, None);

    // Facet 4: limit shapes end-to-end results; the echo names the request.
    let bodies: Vec<(String, String)> = (0..5)
        .map(|n| (format!("probe{n}.rs"), format!("let probe_token = {n};\n")))
        .collect();
    let refs: Vec<(&str, &str)> =
        bodies.iter().map(|(name, body)| (name.as_str(), body.as_str())).collect();
    let fixture = write_core_fixture(&refs);
    drop(build_core_index(&fixture));

    let head = core_searcher(&fixture, 2).search("literal:probe_token").expect("search");
    assert_eq!(head.hits.len(), 2);
    assert_eq!(head.limit, 2);

    let full = core_searcher(&fixture, 10).search("literal:probe_token").expect("search");
    assert_eq!(full.hits.len(), 5);
    assert_eq!(full.limit, 10);
    assert_eq!(
        sorted_hit_files(&full),
        vec!["probe0.rs", "probe1.rs", "probe2.rs", "probe3.rs", "probe4.rs"]
    );
    for hit in &full.hits {
        assert!(hit.excerpt.contains("probe_token"));
    }

    // Facet 5: file filters keep exactly the matching subtree; empty match is empty.
    let fixture = write_core_fixture(&[
        ("src/a.rs", "let filter_probe = 1;\n"),
        ("src/b.rs", "let filter_probe = 2;\n"),
        ("src/c.rs", "let filter_probe = 3;\n"),
        ("other/d.rs", "let filter_probe = 4;\n"),
        ("other/e.rs", "let filter_probe = 5;\n"),
    ]);
    drop(build_core_index(&fixture));

    let unfiltered = core_searcher(&fixture, 10).search("literal:filter_probe").expect("search");
    assert_eq!(unfiltered.hits.len(), 5);

    let mut filtered = core_search_options(&fixture.root, &fixture.db, 10);
    filtered.file_filter = Some("src/**".to_string());
    let response = Searcher::new(filtered).expect("searcher").search("literal:filter_probe").expect("search");
    assert_eq!(response.hits.len(), 3);
    assert_eq!(sorted_hit_files(&response), vec!["src/a.rs", "src/b.rs", "src/c.rs"]);

    let mut nomatch = core_search_options(&fixture.root, &fixture.db, 10);
    nomatch.file_filter = Some("zzz/**".to_string());
    let empty = Searcher::new(nomatch).expect("searcher").search("literal:filter_probe").expect("search");
    assert!(empty.hits.is_empty());

    // Facet 6: count-only reports hand-computed per-file counts.
    let bodies: Vec<(String, String)> = (0..5)
        .map(|n| (format!("count{n}.rs"), format!("let count_probe = {n};\n")))
        .collect();
    let refs: Vec<(&str, &str)> =
        bodies.iter().map(|(name, body)| (name.as_str(), body.as_str())).collect();
    let fixture = write_core_fixture(&refs);
    drop(build_core_index(&fixture));

    let mut options = core_search_options(&fixture.root, &fixture.db, 10);
    options.count_only = true;
    let response = Searcher::new(options).expect("searcher").search("literal:count_probe").expect("search");
    assert!(response.hits.is_empty());
    let expected: Vec<(String, u32)> =
        (0..5).map(|n| (format!("count{n}.rs"), 1)).collect();
    assert_eq!(response.counts, expected);
    assert_eq!(response.counts.iter().map(|(_, n)| n).sum::<u32>(), 5);
}

/// INTENT: pipeline stability — rebuild+research is bit-identical (exact
/// score bits), unchanged reindex is a zero-file no-op with identical
/// results, and incremental update adds then removes exactly one file's
/// evidence with no collateral.
/// KILLS: BEHAVIOR-ONLY.
/// ABSORBS: pipeline_repeated_full_runs_are_identical,
/// pipeline_reindex_is_stable_noop, pipeline_update_paths_add_then_remove.
#[test]
fn pipeline_determinism_reindex_and_incremental() {
    // Facet 1: repeated full runs (rebuild + research) are bit-identical.
    let corpus = tempfile::tempdir().expect("corpus");
    fs::write(
        corpus.path().join("lib.rs"),
        "fn alpha_token() {}\nfn beta_caller() { alpha_token(); }\n",
    )
    .expect("write");
    fs::write(corpus.path().join("util.rs"), "fn beta_helper() {}\n").expect("write");

    let run = |query: &str| {
        let index = tempfile::tempdir().expect("index");
        let db = index.path().join("index.db");
        let mut indexer = Indexer::new(core_index_options(corpus.path(), &db)).expect("indexer");
        let stats = indexer.index_all().expect("index_all");
        assert_eq!(stats.files_indexed, 2);
        drop(indexer);
        let searcher =
            Searcher::new(core_search_options(corpus.path(), &db, 16)).expect("searcher");
        let keys = response_hit_keys_with_scores(&searcher.search(query).expect("search"));
        let repeat = response_hit_keys_with_scores(&searcher.search(query).expect("re-search"));
        assert_eq!(keys, repeat, "same searcher must repeat identically");
        assert!(!keys.is_empty());
        (index, keys)
    };

    let (_index_a, keys_a) = run("alpha_token");
    let (_index_b, keys_b) = run("alpha_token");
    assert_eq!(keys_a, keys_b, "independent rebuilds must agree exactly");

    // Facet 2: reindex is a stable no-op — zero files, identical results.
    let fixture = write_core_fixture(&[
        ("one.rs", "fn stable_one() {}\n"),
        ("two.rs", "fn stable_two() {}\n"),
    ]);
    let mut indexer =
        Indexer::new(core_index_options(&fixture.root, &fixture.db)).expect("indexer");
    assert_eq!(indexer.index_all().expect("first").files_indexed, 2);
    let before = response_hit_keys_with_scores(&core_searcher(&fixture, 16).search("stable_one").expect("search"));
    assert!(!before.is_empty());
    drop(indexer);

    // Without force_reindex the unchanged tree is a no-op...
    let mut plain = core_index_options(&fixture.root, &fixture.db);
    plain.force_reindex = false;
    let mut indexer = Indexer::new(plain).expect("reopen");
    let second = indexer.index_all().expect("second");
    assert_eq!(second.files_indexed, 0);
    drop(indexer);
    let after = response_hit_keys_with_scores(&core_searcher(&fixture, 16).search("stable_one").expect("search"));
    assert_eq!(before, after);

    // Facet 3: incremental update adds then removes exactly one file's evidence.
    let fixture = write_core_fixture(&[("base.rs", "fn base_symbol() {}\n")]);
    let mut indexer =
        Indexer::new(core_index_options(&fixture.root, &fixture.db)).expect("indexer");
    indexer.index_all().expect("index_all");

    let added = fixture.root.join("added.rs");
    fs::write(&added, "fn added_symbol() {}\n").expect("write");
    let up = indexer.update_paths(std::slice::from_ref(&added)).expect("add");
    assert_eq!(up.files_indexed, 1);
    assert_eq!(up.files_removed, 0);
    drop(indexer);
    let found = core_searcher(&fixture, 16).search("defs:added_symbol").expect("search");
    assert!(
        found.hits.iter().any(|hit| hit.kind == HitKind::Def
            && hit.symbol.as_deref() == Some("added_symbol")),
        "added file must be searchable"
    );

    fs::remove_file(&added).expect("remove");
    let mut indexer =
        Indexer::new(core_index_options(&fixture.root, &fixture.db)).expect("reopen");
    let down = indexer.update_paths(std::slice::from_ref(&added)).expect("remove");
    assert_eq!(down.files_removed, 1);
    drop(indexer);
    let gone = core_searcher(&fixture, 16).search("defs:added_symbol").expect("search");
    assert!(
        gone.hits.iter().all(|hit| hit.symbol.as_deref() != Some("added_symbol")),
        "removed file must leave no def evidence"
    );
    // No collateral removal: the untouched file still serves.
    let kept = core_searcher(&fixture, 16).search("defs:base_symbol").expect("search");
    assert!(
        kept.hits.iter().any(|hit| hit.kind == HitKind::Def
            && hit.symbol.as_deref() == Some("base_symbol")),
        "untouched file must keep serving"
    );
}

/// INTENT: pipeline fail-closed edges — empty corpus is Ok-with-empty across
/// query modes (incl count-only), missing/file-as-root fails at Indexer and
/// Searcher construction, and pre-cancelled indexing fails with the
/// INDEX_CANCELLED discriminant then recovers on clear.
/// KILLS: silent-default, error-on-empty, and cancel-ignored mutants.
/// ABSORBS: pipeline_empty_corpus_returns_absence_not_error,
/// pipeline_missing_root_fails_closed, pipeline_pre_cancelled_index_fails_closed.
#[test]
fn pipeline_empty_missing_and_cancelled_fail_closed() {
    // Facet 1: empty corpus — absence is Ok-with-empty, never an error.
    let fixture = write_core_fixture(&[]);
    let mut indexer =
        Indexer::new(core_index_options(&fixture.root, &fixture.db)).expect("indexer");
    let stats = indexer.index_all().expect("index empty");
    assert_eq!(stats.files_indexed, 0);
    assert_eq!(stats.files_failed, 0);
    drop(indexer);

    for query in ["anything_at_all", "defs:anything_at_all", "literal:anything_at_all"] {
        let response = core_searcher(&fixture, 16).search(query).expect("search empty");
        assert!(response.hits.is_empty(), "query {query} must find nothing");
    }
    let mut counted = core_search_options(&fixture.root, &fixture.db, 16);
    counted.count_only = true;
    let counts = Searcher::new(counted).expect("searcher").search("anything_at_all").expect("count");
    assert!(counts.hits.is_empty());
    assert!(counts.counts.is_empty());

    // Facet 2: missing roots fail closed at both construction sites.
    let scratch = tempfile::tempdir().expect("scratch");
    let missing = scratch.path().join("does-not-exist");
    let db = scratch.path().join("index.db");

    assert!(Indexer::new(core_index_options(&missing, &db)).is_err());
    match Searcher::new(core_search_options(&missing, &db, 16)) {
        Ok(_) => panic!("missing root must fail closed"),
        Err(err) => assert!(matches!(err, StoreError::Other(_))),
    }

    // A regular file is not a project root either.
    let file = scratch.path().join("file.rs");
    fs::write(&file, "fn x() {}\n").expect("write");
    assert!(Searcher::new(core_search_options(&file, &db, 16)).is_err());

    // Facet 3: pre-cancelled indexing fails closed, then recovers on clear.
    let fixture = write_core_fixture(&[
        ("a.rs", "fn cancel_one() {}\n"),
        ("b.rs", "fn cancel_two() {}\n"),
    ]);
    let mut indexer =
        Indexer::new(core_index_options(&fixture.root, &fixture.db)).expect("indexer");
    let flag = Arc::new(AtomicBool::new(true));
    indexer.set_cancel(Arc::clone(&flag));
    let err = indexer.index_all().expect_err("cancelled index must fail");
    // Discriminant: the Other variant carrying the documented stable text.
    assert!(matches!(err, StoreError::Other(ref text) if text == INDEX_CANCELLED));

    // The same indexer recovers once the flag clears: the failure was the
    // cancel, not the corpus.
    flag.store(false, Ordering::Release);
    assert_eq!(indexer.index_all().expect("retry").files_indexed, 2);
    drop(indexer);
    let found = core_searcher(&fixture, 16).search("defs:cancel_one").expect("search");
    assert!(
        found.hits.iter().any(|hit| hit.symbol.as_deref() == Some("cancel_one")),
        "post-cancel index must serve"
    );
}

//! Pass 4 (oracle-foundry): L4 end-to-end oracles for ast-sgrep-core.
//!
//! Pass 1 owns L1 unit contracts (clamps, query-len, FTS escape,
//! schema-mismatch, mmap); pass 2 owns L2 scoring/fusion/query/limit
//! discriminants; pass 3 owns L3 stage-level oracles (bounded IO, excerpts,
//! wire distrust, dedup merge, planner, finish gates, resolution, lexicon,
//! SCIP, filters, intent). This pass owns FULL pipelines only: real tempdir
//! corpus -> `Indexer::index_all` -> `Searcher::search` -> merged hits with
//! signal/confidence provenance and snapshot stamps. No stage-level table
//! from passes 1-3 is re-asserted here.
//!
//! Every expectation is hand-computed from fixture contents. Failures assert
//! discriminants (`is_err`, emptiness, `matches!`, exact keys/counts/sets),
//! never message text. Deterministic; no new deps.

use ast_sgrep_core::{
    HitKind, IndexOptions, Indexer, SearchOptions, Searcher, StoreError, INDEX_CANCELLED,
    INDEX_SCHEMA_VERSION,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

struct Fixture {
    _corpus: tempfile::TempDir,
    _index: tempfile::TempDir,
    root: PathBuf,
    db: PathBuf,
}

fn write_fixture(files: &[(&str, &str)]) -> Fixture {
    let corpus = tempfile::tempdir().expect("corpus tempdir");
    for (rel, body) in files {
        let path = corpus.path().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        fs::write(&path, body).expect("write fixture");
    }
    let index = tempfile::tempdir().expect("index tempdir");
    let db = index.path().join("index.db");
    Fixture {
        root: corpus.path().to_path_buf(),
        db,
        _corpus: corpus,
        _index: index,
    }
}

fn index_options(root: &Path, db: &Path) -> IndexOptions {
    IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        force_reindex: true,
        embed_semantic: false,
        ..IndexOptions::default()
    }
}

fn search_options(root: &Path, db: &Path, limit: usize) -> SearchOptions {
    SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        limit,
        use_embed: false,
        ..SearchOptions::default()
    }
}

/// Full build leg: fresh indexer over the fixture, whole tree indexed.
fn build(fixture: &Fixture) -> Indexer {
    let mut indexer =
        Indexer::new(index_options(&fixture.root, &fixture.db)).expect("indexer new");
    indexer.index_all().expect("index_all");
    indexer
}

fn searcher(fixture: &Fixture, limit: usize) -> Searcher {
    Searcher::new(search_options(&fixture.root, &fixture.db, limit)).expect("searcher new")
}

/// Order-sensitive identity of a ranked response: file, span, kind, symbol,
/// and exact score bits. Two full runs agree iff these sequences agree.
fn hit_keys(response: &ast_sgrep_core::SearchResponse) -> Vec<(String, u32, u32, String, Option<String>, u64)> {
    response
        .hits
        .iter()
        .map(|hit| {
            (
                hit.file.clone(),
                hit.line_start,
                hit.line_end,
                hit.kind.as_str().to_string(),
                hit.symbol.clone(),
                hit.score.to_bits(),
            )
        })
        .collect()
}

fn sorted_files(response: &ast_sgrep_core::SearchResponse) -> Vec<String> {
    let mut files: Vec<String> = response.hits.iter().map(|hit| hit.file.clone()).collect();
    files.sort();
    files
}

// ---------------------------------------------------------------------------
// 1. build -> prefixed search round-trips the indexed symbols
// ---------------------------------------------------------------------------

#[test]
fn pipeline_defs_and_callers_cite_indexed_symbols() {
    let fixture = write_fixture(&[(
        "lib.rs",
        "fn refresh_token() {}\nfn caller_one() { refresh_token(); }\n",
    )]);
    drop(build(&fixture));
    let searcher = searcher(&fixture, 16);

    let defs = searcher.search("defs:refresh_token").expect("defs search");
    let def = defs
        .hits
        .iter()
        .find(|hit| hit.kind == HitKind::Def && hit.symbol.as_deref() == Some("refresh_token"))
        .expect("Def hit must cite the indexed symbol");
    assert!(def.score > 0.0);
    assert_eq!(def.file, "lib.rs");

    let callers = searcher.search("callers:refresh_token").expect("callers search");
    let caller = callers
        .hits
        .iter()
        .find(|hit| hit.kind == HitKind::Caller && hit.callee.as_deref() == Some("refresh_token"))
        .expect("Caller hit must cite the indexed callee");
    assert!(caller.score > 0.0);
    assert_eq!(caller.file, "lib.rs");
}

// ---------------------------------------------------------------------------
// 2. hybrid search merges evidence; every hit carries signal provenance
// ---------------------------------------------------------------------------

#[test]
fn pipeline_hybrid_merges_evidence_with_signal_provenance() {
    let fixture = write_fixture(&[(
        "auth.rs",
        "fn auth_refresh() {}\nfn login() { auth_refresh(); }\n",
    )]);
    drop(build(&fixture));
    let hybrid = searcher(&fixture, 16).search("auth_refresh").expect("hybrid search");

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
}

// ---------------------------------------------------------------------------
// 3. responses are stamped with schema provenance, not git state
// ---------------------------------------------------------------------------

#[test]
fn pipeline_snapshot_stamp_provenance() {
    let fixture = write_fixture(&[("a.rs", "fn stamped_symbol() {}\n")]);
    drop(build(&fixture));
    let response = searcher(&fixture, 16)
        .search("defs:stamped_symbol")
        .expect("search");
    assert!(!response.hits.is_empty());
    assert_eq!(response.snapshot.schema_version, INDEX_SCHEMA_VERSION);
    // A tempdir corpus is not a git worktree: no HEAD is claimed.
    assert_eq!(response.snapshot.git_head, None);
}

// ---------------------------------------------------------------------------
// 4. repeated full runs (rebuild + research) are bit-identical
// ---------------------------------------------------------------------------

#[test]
fn pipeline_repeated_full_runs_are_identical() {
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
        let mut indexer = Indexer::new(index_options(corpus.path(), &db)).expect("indexer");
        let stats = indexer.index_all().expect("index_all");
        assert_eq!(stats.files_indexed, 2);
        drop(indexer);
        let searcher =
            Searcher::new(search_options(corpus.path(), &db, 16)).expect("searcher");
        let keys = hit_keys(&searcher.search(query).expect("search"));
        let repeat = hit_keys(&searcher.search(query).expect("re-search"));
        assert_eq!(keys, repeat, "same searcher must repeat identically");
        assert!(!keys.is_empty());
        (index, keys)
    };

    let (_index_a, keys_a) = run("alpha_token");
    let (_index_b, keys_b) = run("alpha_token");
    assert_eq!(keys_a, keys_b, "independent rebuilds must agree exactly");
}

// ---------------------------------------------------------------------------
// 5. reindex is a stable no-op: zero files, identical results
// ---------------------------------------------------------------------------

#[test]
fn pipeline_reindex_is_stable_noop() {
    let fixture = write_fixture(&[
        ("one.rs", "fn stable_one() {}\n"),
        ("two.rs", "fn stable_two() {}\n"),
    ]);
    let mut indexer =
        Indexer::new(index_options(&fixture.root, &fixture.db)).expect("indexer");
    assert_eq!(indexer.index_all().expect("first").files_indexed, 2);
    let before = hit_keys(&searcher(&fixture, 16).search("stable_one").expect("search"));
    assert!(!before.is_empty());
    drop(indexer);

    // Without force_reindex the unchanged tree is a no-op...
    let mut plain = index_options(&fixture.root, &fixture.db);
    plain.force_reindex = false;
    let mut indexer = Indexer::new(plain).expect("reopen");
    let second = indexer.index_all().expect("second");
    assert_eq!(second.files_indexed, 0);
    drop(indexer);
    let after = hit_keys(&searcher(&fixture, 16).search("stable_one").expect("search"));
    assert_eq!(before, after);
}

// ---------------------------------------------------------------------------
// 6. empty corpus: absence is Ok-with-empty, never an error
// ---------------------------------------------------------------------------

#[test]
fn pipeline_empty_corpus_returns_absence_not_error() {
    let fixture = write_fixture(&[]);
    let mut indexer =
        Indexer::new(index_options(&fixture.root, &fixture.db)).expect("indexer");
    let stats = indexer.index_all().expect("index empty");
    assert_eq!(stats.files_indexed, 0);
    assert_eq!(stats.files_failed, 0);
    drop(indexer);

    for query in ["anything_at_all", "defs:anything_at_all", "literal:anything_at_all"] {
        let response = searcher(&fixture, 16).search(query).expect("search empty");
        assert!(response.hits.is_empty(), "query {query} must find nothing");
    }
    let mut counted = search_options(&fixture.root, &fixture.db, 16);
    counted.count_only = true;
    let counts = Searcher::new(counted).expect("searcher").search("anything_at_all").expect("count");
    assert!(counts.hits.is_empty());
    assert!(counts.counts.is_empty());
}

// ---------------------------------------------------------------------------
// 7. missing roots fail closed at both construction sites
// ---------------------------------------------------------------------------

#[test]
fn pipeline_missing_root_fails_closed() {
    let scratch = tempfile::tempdir().expect("scratch");
    let missing = scratch.path().join("does-not-exist");
    let db = scratch.path().join("index.db");

    assert!(Indexer::new(index_options(&missing, &db)).is_err());
    match Searcher::new(search_options(&missing, &db, 16)) {
        Ok(_) => panic!("missing root must fail closed"),
        Err(err) => assert!(matches!(err, StoreError::Other(_))),
    }

    // A regular file is not a project root either.
    let file = scratch.path().join("file.rs");
    fs::write(&file, "fn x() {}\n").expect("write");
    assert!(Searcher::new(search_options(&file, &db, 16)).is_err());
}

// ---------------------------------------------------------------------------
// 8. limit shapes end-to-end results; the echo names the request
// ---------------------------------------------------------------------------

#[test]
fn pipeline_limit_shapes_results_end_to_end() {
    let bodies: Vec<(String, String)> = (0..5)
        .map(|n| (format!("probe{n}.rs"), format!("let probe_token = {n};\n")))
        .collect();
    let refs: Vec<(&str, &str)> =
        bodies.iter().map(|(name, body)| (name.as_str(), body.as_str())).collect();
    let fixture = write_fixture(&refs);
    drop(build(&fixture));

    let head = searcher(&fixture, 2).search("literal:probe_token").expect("search");
    assert_eq!(head.hits.len(), 2);
    assert_eq!(head.limit, 2);

    let full = searcher(&fixture, 10).search("literal:probe_token").expect("search");
    assert_eq!(full.hits.len(), 5);
    assert_eq!(full.limit, 10);
    assert_eq!(
        sorted_files(&full),
        vec!["probe0.rs", "probe1.rs", "probe2.rs", "probe3.rs", "probe4.rs"]
    );
    for hit in &full.hits {
        assert!(hit.excerpt.contains("probe_token"));
    }
}

// ---------------------------------------------------------------------------
// 9. file filters keep exactly the matching subtree; empty match is empty
// ---------------------------------------------------------------------------

#[test]
fn pipeline_file_filter_keeps_matching_subtree() {
    let fixture = write_fixture(&[
        ("src/a.rs", "let filter_probe = 1;\n"),
        ("src/b.rs", "let filter_probe = 2;\n"),
        ("src/c.rs", "let filter_probe = 3;\n"),
        ("other/d.rs", "let filter_probe = 4;\n"),
        ("other/e.rs", "let filter_probe = 5;\n"),
    ]);
    drop(build(&fixture));

    let unfiltered = searcher(&fixture, 10).search("literal:filter_probe").expect("search");
    assert_eq!(unfiltered.hits.len(), 5);

    let mut filtered = search_options(&fixture.root, &fixture.db, 10);
    filtered.file_filter = Some("src/**".to_string());
    let response = Searcher::new(filtered).expect("searcher").search("literal:filter_probe").expect("search");
    assert_eq!(response.hits.len(), 3);
    assert_eq!(sorted_files(&response), vec!["src/a.rs", "src/b.rs", "src/c.rs"]);

    let mut nomatch = search_options(&fixture.root, &fixture.db, 10);
    nomatch.file_filter = Some("zzz/**".to_string());
    let empty = Searcher::new(nomatch).expect("searcher").search("literal:filter_probe").expect("search");
    assert!(empty.hits.is_empty());
}

// ---------------------------------------------------------------------------
// 10. count-only reports hand-computed per-file counts
// ---------------------------------------------------------------------------

#[test]
fn pipeline_count_only_reports_hand_computed_counts() {
    let bodies: Vec<(String, String)> = (0..5)
        .map(|n| (format!("count{n}.rs"), format!("let count_probe = {n};\n")))
        .collect();
    let refs: Vec<(&str, &str)> =
        bodies.iter().map(|(name, body)| (name.as_str(), body.as_str())).collect();
    let fixture = write_fixture(&refs);
    drop(build(&fixture));

    let mut options = search_options(&fixture.root, &fixture.db, 10);
    options.count_only = true;
    let response = Searcher::new(options).expect("searcher").search("literal:count_probe").expect("search");
    assert!(response.hits.is_empty());
    let expected: Vec<(String, u32)> =
        (0..5).map(|n| (format!("count{n}.rs"), 1)).collect();
    assert_eq!(response.counts, expected);
    assert_eq!(response.counts.iter().map(|(_, n)| n).sum::<u32>(), 5);
}

// ---------------------------------------------------------------------------
// 11. incremental update adds then removes exactly one file's evidence
// ---------------------------------------------------------------------------

#[test]
fn pipeline_update_paths_add_then_remove() {
    let fixture = write_fixture(&[("base.rs", "fn base_symbol() {}\n")]);
    let mut indexer =
        Indexer::new(index_options(&fixture.root, &fixture.db)).expect("indexer");
    indexer.index_all().expect("index_all");

    let added = fixture.root.join("added.rs");
    fs::write(&added, "fn added_symbol() {}\n").expect("write");
    let up = indexer.update_paths(std::slice::from_ref(&added)).expect("add");
    assert_eq!(up.files_indexed, 1);
    assert_eq!(up.files_removed, 0);
    drop(indexer);
    let found = searcher(&fixture, 16).search("defs:added_symbol").expect("search");
    assert!(
        found.hits.iter().any(|hit| hit.kind == HitKind::Def
            && hit.symbol.as_deref() == Some("added_symbol")),
        "added file must be searchable"
    );

    fs::remove_file(&added).expect("remove");
    let mut indexer =
        Indexer::new(index_options(&fixture.root, &fixture.db)).expect("reopen");
    let down = indexer.update_paths(std::slice::from_ref(&added)).expect("remove");
    assert_eq!(down.files_removed, 1);
    drop(indexer);
    let gone = searcher(&fixture, 16).search("defs:added_symbol").expect("search");
    assert!(
        gone.hits.iter().all(|hit| hit.symbol.as_deref() != Some("added_symbol")),
        "removed file must leave no def evidence"
    );
    // No collateral removal: the untouched file still serves.
    let kept = searcher(&fixture, 16).search("defs:base_symbol").expect("search");
    assert!(
        kept.hits.iter().any(|hit| hit.kind == HitKind::Def
            && hit.symbol.as_deref() == Some("base_symbol")),
        "untouched file must keep serving"
    );
}

// ---------------------------------------------------------------------------
// 12. pre-cancelled indexing fails closed with the stable cancel discriminant
// ---------------------------------------------------------------------------

#[test]
fn pipeline_pre_cancelled_index_fails_closed() {
    let fixture = write_fixture(&[
        ("a.rs", "fn cancel_one() {}\n"),
        ("b.rs", "fn cancel_two() {}\n"),
    ]);
    let mut indexer =
        Indexer::new(index_options(&fixture.root, &fixture.db)).expect("indexer");
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
    let found = searcher(&fixture, 16).search("defs:cancel_one").expect("search");
    assert!(
        found.hits.iter().any(|hit| hit.symbol.as_deref() == Some("cancel_one")),
        "post-cancel index must serve"
    );
}

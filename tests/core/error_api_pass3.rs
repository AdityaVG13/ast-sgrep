//! E3 negative-path metamorphic tests for ast-sgrep-core errors.
//!
//! E1 pins taxonomy (each `StoreError` discriminant is reachable). E2 pins
//! propagation (a depth error surfaces with the promised discriminant at each
//! layer). E3 pins RELATIONS over failures — properties that hold across
//! entry points, repetitions, and surrounding state:
//!
//! - MR1 same-fault-same-discriminant: one underlying fault observed through
//!   different entry points yields the same discriminant (and sqlite kind).
//! - MR2 error determinism: the same trigger repeated yields the identical
//!   discriminant sequence; a failure never poisons a later success.
//! - MR3 fail-closed: a poisoned store returns `Err` through every lane —
//!   never `Ok` with partial or empty results misreported as success. (The
//!   cancelled-writes entry×assert relation folds into the E2 cancel anchor;
//!   this file keeps the poisoned-store legs.)
//! - MR4 error-before-side-effect: a failed open/write leaves the filesystem
//!   and all committed store rows exactly as they were.
//!
//! Discriminants (and sqlite `ErrorCode`) only; no message-text asserts.

#[path = "error_testkit.rs"]
mod error_testkit;

use ast_sgrep_core::{
    search_pattern, IndexOptions, IndexStore, Indexer, SearchOptions, Searcher,
    MAX_INCREMENTAL_PATHS, MAX_QUERY_CHARS,
};
use ast_sgrep_testkit::err_of;
use error_testkit::{
    discriminant, indexer_at, mem_searcher, searcher_at, sqlite_code, write_garbage_db,
};
use rusqlite::ErrorCode;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::AtomicBool,
    Arc,
};
use tempfile::TempDir;

/// INTENT: MR1 — the same corrupt-bytes fault through the store open, the
/// read-layer constructor, and the write-layer constructor yields the same
/// discriminant AND the same sqlite kind (MR1 anchor).
/// KILLS: layer-conversion, kind-divergence.
/// ABSORBS: none (kept solo).
/// OVERLAP: E2 garbage legs (this is the relation statement).
#[test]
fn garbage_db_same_discriminant_across_open_search_and_index() {
    let temp = TempDir::new().unwrap();
    let db = write_garbage_db(temp.path(), "index.db");
    let via_store = err_of(IndexStore::open(temp.path(), Some(&db)));
    let via_searcher = err_of(searcher_at(temp.path(), &db));
    let via_indexer = err_of(Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(db),
        ..IndexOptions::default()
    }));
    for (name, err) in [
        ("store open", &via_store),
        ("searcher new", &via_searcher),
        ("indexer new", &via_indexer),
    ] {
        assert_eq!(
            discriminant(err),
            0,
            "garbage db via {name} must stay Database, got {err:?}"
        );
    }
    assert_eq!(
        (sqlite_code(&via_store), sqlite_code(&via_searcher), sqlite_code(&via_indexer)),
        (
            Some(ErrorCode::NotADatabase),
            Some(ErrorCode::NotADatabase),
            Some(ErrorCode::NotADatabase),
        ),
        "sqlite kind must survive every layer identically"
    );
}

/// INTENT: MR1 — a file blocking the index directory fails identically through
/// the store open and the write-layer constructor (both `Other` by the
/// deliberate `create_dir_all` Io→Other map).
/// KILLS: layer-divergence.
/// ABSORBS: none (kept solo; distinct map site from garbage).
#[test]
fn blocked_index_dir_same_discriminant_across_store_and_indexer() {
    let temp = TempDir::new().unwrap();
    let blocker = temp.path().join("blocker");
    std::fs::write(&blocker, b"i am a file, not a directory").unwrap();
    let target = blocker.join("index.db");
    let via_store = err_of(IndexStore::open(temp.path(), Some(&target)));
    let via_indexer = err_of(Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(target),
        ..IndexOptions::default()
    }));
    assert_eq!(
        (discriminant(&via_store), discriminant(&via_indexer)),
        (2, 2),
        "blocked index dir must fail as Other through both layers, got {via_store:?} / {via_indexer:?}"
    );
}

/// INTENT: MR1 — one oversize query through every lexical search lane,
/// including the multi-pattern fan-in, yields `Other` everywhere. No lane
/// answers, truncates, or converts.
/// KILLS: lane-truncate-and-answer, lane-conversion.
/// ABSORBS: none (kept solo).
#[test]
fn oversize_query_same_discriminant_across_search_lanes() {
    let temp = TempDir::new().unwrap();
    std::fs::write(temp.path().join("a.rs"), b"fn alpha() {}\n").unwrap();
    let searcher = mem_searcher(temp.path());
    let query = "a".repeat(MAX_QUERY_CHARS + 1);
    let errs = [
        ("search", err_of(searcher.search(&query))),
        ("search_literal", err_of(searcher.search_literal(&query))),
        ("search_word", err_of(searcher.search_word(&query))),
        ("search_regex", err_of(searcher.search_regex(&query))),
        (
            "search_multi_pattern",
            err_of(searcher.search_multi_pattern(&[query.clone()])),
        ),
    ];
    for (lane, err) in &errs {
        assert_eq!(
            discriminant(err),
            2,
            "oversize query via {lane} must fail as Other, got {err:?}"
        );
    }
}

/// INTENT: MR1+MR3 — one dropped-table fault fails as Database with the same
/// sqlite kind through the depth query, the free function, the prefixed search
/// lane, the multi-pattern fan-in, AND the status probe. Controls answer Ok
/// first on the healthy stores, so the post-drop Errs are the poison.
/// KILLS: lane-degrade, kind-divergence, status-Ok-over-broken-schema,
/// Ok-empty-over-poison, cache-masks-poison.
/// ABSORBS: dropped_table_fails_status_with_database_discriminant (status leg),
/// poisoned_store_fails_closed_across_pattern_lanes (control-first + cache-off
/// rationale).
/// OVERLAP: E4 dropped-tables drill pins the status leg too (end-to-end + rebuild).
#[test]
fn dropped_table_same_discriminant_across_depth_and_search_lanes() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open_in_memory(temp.path()).expect("in-memory store");
    // Response cache OFF: otherwise the control queries below would populate the
    // cache and the post-drop lanes would answer cached `Ok` without touching
    // the poisoned table — a legitimate cache layer, not the fail-closed
    // relation under test.
    let searcher = Searcher::with_store(
        IndexStore::open_in_memory(temp.path()).expect("second store"),
        SearchOptions {
            root: temp.path().to_path_buf(),
            limit: 10,
            use_embed: false,
            ..SearchOptions::default()
        },
    )
    .with_response_cache(false);
    // Control first: every lane answers Ok on the healthy stores.
    assert_eq!(
        store.pattern_node_count().expect("table exists"),
        0,
        "control: fresh store must have an empty pattern_nodes table"
    );
    assert!(
        search_pattern("greet_user", &store, temp.path(), None, 10).is_ok(),
        "control: free function on healthy store must be Ok"
    );
    assert!(
        searcher.search("pattern:greet_user").is_ok(),
        "control: prefixed search on healthy store must be Ok"
    );
    assert!(
        searcher
            .search_multi_pattern(&["greet_user".to_string()])
            .is_ok(),
        "control: multi-pattern on healthy store must be Ok"
    );
    assert!(
        store.status().is_ok(),
        "control: status on a healthy store must be Ok"
    );
    store
        .connection()
        .execute_batch("DROP TABLE pattern_nodes; DROP TABLE symbols")
        .unwrap();
    searcher
        .store()
        .connection()
        .execute_batch("DROP TABLE pattern_nodes")
        .unwrap();
    let via_depth = err_of(store.pattern_node_count().map(|_| ()));
    let via_free_fn =
        err_of(search_pattern("greet_user", &store, temp.path(), None, 10).map(|_| ()));
    let via_search = err_of(searcher.search("pattern:greet_user").map(|_| ()));
    let via_multi =
        err_of(searcher.search_multi_pattern(&["greet_user".to_string()]).map(|_| ()));
    let via_status = err_of(store.status().map(|_| ()));
    for (lane, err) in [
        ("depth query", &via_depth),
        ("free function", &via_free_fn),
        ("prefixed search", &via_search),
        ("multi-pattern", &via_multi),
        ("status probe", &via_status),
    ] {
        assert_eq!(
            discriminant(err),
            0,
            "dropped table via {lane} must stay Database, got {err:?}"
        );
    }
    assert_eq!(
        sqlite_code(&via_depth),
        sqlite_code(&via_free_fn),
        "depth and free-function sqlite kinds must match"
    );
    assert_eq!(
        sqlite_code(&via_search),
        sqlite_code(&via_multi),
        "search and multi-pattern sqlite kinds must match"
    );
    assert!(
        sqlite_code(&via_status).is_some(),
        "status failure must carry a sqlite kind, got {via_status:?}"
    );
}

/// INTENT: MR2 — the same four triggers, run twice against fresh fixtures,
/// yield the identical discriminant sequence (and identical sqlite kinds).
/// Error identity is a pure function of the trigger.
/// KILLS: run-to-run-drift.
/// ABSORBS: none (kept solo).
#[test]
fn repeated_triggers_yield_identical_discriminant_sequence() {
    fn run_once() -> (Vec<u8>, Vec<Option<ErrorCode>>) {
        let temp = TempDir::new().unwrap();
        let garbage = write_garbage_db(temp.path(), "garbage.db");
        let blocker = temp.path().join("blocker");
        std::fs::write(&blocker, b"i am a file, not a directory").unwrap();
        let e1 = err_of(IndexStore::open(temp.path(), Some(&garbage)));
        let e2 = err_of(IndexStore::open(temp.path(), Some(&blocker.join("index.db"))));
        let e3 = err_of(Searcher::new(SearchOptions {
            root: temp.path().join("nosuch-root"),
            ..SearchOptions::default()
        }));
        let searcher = mem_searcher(temp.path());
        let e4 = err_of(searcher.search(&"q".repeat(MAX_QUERY_CHARS + 1)));
        let errs = [e1, e2, e3, e4];
        (
            errs.iter().map(discriminant).collect(),
            errs.iter().map(sqlite_code).collect(),
        )
    }
    let (discs_a, codes_a) = run_once();
    let (discs_b, codes_b) = run_once();
    assert_eq!(discs_a, vec![0, 2, 2, 2], "first run must pin the sequence");
    assert_eq!(
        discs_a, discs_b,
        "repeated triggers must yield identical discriminants"
    );
    assert_eq!(
        codes_a, codes_b,
        "repeated triggers must yield identical sqlite kinds"
    );
}

/// INTENT: MR2 — a failure never poisons later success through the same handle:
/// an `Err` followed by a valid call is `Ok`, and the `Err` repeated is the
/// same discriminant. Errors are stateless rejections, not handle poison.
/// KILLS: handle-poison-on-error.
/// ABSORBS: none (kept solo).
/// OVERLAP: E4 oversize drill (E3 is ingress-level).
#[test]
fn error_does_not_poison_later_success_on_same_handle() {
    let temp = TempDir::new().unwrap();
    std::fs::write(temp.path().join("a.rs"), b"fn alpha() {}\n").unwrap();
    let searcher = mem_searcher(temp.path());
    let oversize = "q".repeat(MAX_QUERY_CHARS + 1);
    assert_eq!(
        discriminant(&err_of(searcher.search(&oversize))),
        2,
        "oversize query must fail as Other"
    );
    assert!(
        searcher.search("alpha").is_ok(),
        "valid search after a failure must still succeed"
    );
    assert_eq!(
        discriminant(&err_of(searcher.search_regex("(["))),
        2,
        "invalid regex must fail as Other"
    );
    assert!(
        searcher.search_regex("alp.*").is_ok(),
        "valid regex after a failure must still succeed"
    );
    assert_eq!(
        discriminant(&err_of(searcher.search(&oversize))),
        2,
        "repeated oversize query must fail identically"
    );
}

/// INTENT: MR4 — failed opens leave the filesystem byte-identical: no
/// half-created index file, no repaired garbage, no directory entries added or
/// removed. Snapshot before, trigger, snapshot after; the maps must match.
/// KILLS: half-created-index, garbage-repair-on-reject.
/// ABSORBS: none (kept solo).
#[test]
fn failed_opens_leave_filesystem_untouched() {
    fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut map = BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(current) = stack.pop() {
            for entry in std::fs::read_dir(&current).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let key = path.strip_prefix(dir).unwrap().to_path_buf();
                    map.insert(key, std::fs::read(&path).unwrap());
                }
            }
        }
        map
    }
    let temp = TempDir::new().unwrap();
    std::fs::write(temp.path().join("a.rs"), b"fn alpha() {}\n").unwrap();
    let garbage = write_garbage_db(temp.path(), "garbage.db");
    let blocker = temp.path().join("blocker");
    std::fs::write(&blocker, b"i am a file, not a directory").unwrap();
    let before = snapshot(temp.path());
    let _ = err_of(Indexer::new(IndexOptions {
        root: temp.path().join("nosuch-root"),
        index_path: Some(temp.path().join("index.db")),
        ..IndexOptions::default()
    }));
    assert_eq!(
        snapshot(temp.path()),
        before,
        "missing-root open must not touch the filesystem"
    );
    let _ = err_of(Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(garbage),
        ..IndexOptions::default()
    }));
    assert_eq!(
        snapshot(temp.path()),
        before,
        "garbage-db open must not touch the filesystem"
    );
    let _ = err_of(IndexStore::open(temp.path(), Some(&blocker.join("index.db"))));
    assert_eq!(
        snapshot(temp.path()),
        before,
        "blocked-dir open must not touch the filesystem"
    );
}

/// INTENT: MR4 — failed writes leave committed store state intact: a previously
/// indexed file keeps its exact hash and the line count is unchanged after an
/// over-max batch, a cancelled update, and a cancelled full index, all of which
/// report the same `Other` discriminant.
/// KILLS: committed-row-clobber-on-failure.
/// ABSORBS: none (kept solo).
#[test]
fn failed_writes_preserve_committed_rows() {
    let temp = TempDir::new().unwrap();
    let good = temp.path().join("good.rs");
    std::fs::write(&good, b"fn greet() {}\n").unwrap();
    let db = temp.path().join("index.db");
    let mut indexer = indexer_at(temp.path(), &db);
    indexer.index_all().expect("commit the good file");
    let committed_hash = indexer
        .store()
        .file_hash("good.rs")
        .expect("store readable");
    assert!(
        committed_hash.is_some(),
        "control: good file must be committed"
    );
    let committed_lines = indexer
        .store()
        .indexed_line_count()
        .expect("line count readable");
    let over_max = vec![good.clone(); MAX_INCREMENTAL_PATHS + 1];
    let e1 = err_of(indexer.update_paths(&over_max).map(|_| ()));
    let cancel = Arc::new(AtomicBool::new(true));
    indexer.set_cancel(Arc::clone(&cancel));
    let e2 = err_of(indexer.update_paths(std::slice::from_ref(&good)).map(|_| ()));
    let e3 = err_of(indexer.index_all().map(|_| ()));
    for (op, err) in [
        ("over-max batch", &e1),
        ("cancelled update", &e2),
        ("cancelled index_all", &e3),
    ] {
        assert_eq!(
            discriminant(err),
            2,
            "{op} must fail as Other, got {err:?}"
        );
    }
    assert_eq!(
        indexer.store().file_hash("good.rs").expect("store readable"),
        committed_hash,
        "failed writes must preserve the committed hash"
    );
    assert_eq!(
        indexer
            .store()
            .indexed_line_count()
            .expect("line count readable"),
        committed_lines,
        "failed writes must preserve the committed line count"
    );
}

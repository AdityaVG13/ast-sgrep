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
//! - MR3 fail-closed: a poisoned store/cancelled op returns `Err` through
//!   every lane — never `Ok` with partial or empty results misreported as
//!   success.
//! - MR4 error-before-side-effect: a failed open/write leaves the filesystem
//!   and all committed store rows exactly as they were.
//!
//! Discriminants (and sqlite `ErrorCode`) only; no message-text asserts.

use ast_sgrep_core::{
    search_pattern, IndexOptions, IndexStore, Indexer, SearchOptions, Searcher, StoreError,
    MAX_INCREMENTAL_PATHS, MAX_QUERY_CHARS,
};
use rusqlite::ErrorCode;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tempfile::TempDir;

/// 0 = Database, 1 = Io, 2 = Other. Total: every `StoreError` maps to one.
fn discriminant(err: &StoreError) -> u8 {
    match err {
        StoreError::Database(_) => 0,
        StoreError::Io(_) => 1,
        StoreError::Other(_) => 2,
    }
}

fn err_of<T>(result: Result<T, StoreError>) -> StoreError {
    match result {
        Ok(_) => panic!("expected Err, got Ok"),
        Err(err) => err,
    }
}

fn sqlite_code(err: &StoreError) -> Option<ErrorCode> {
    match err {
        StoreError::Database(rusqlite::Error::SqliteFailure(code, _)) => Some(code.code),
        _ => None,
    }
}

fn mem_searcher(root: &Path) -> Searcher {
    let store = IndexStore::open_in_memory(root).expect("in-memory store");
    Searcher::with_store(
        store,
        SearchOptions {
            root: root.to_path_buf(),
            limit: 10,
            use_embed: false,
            ..SearchOptions::default()
        },
    )
}

fn write_garbage_db(dir: &Path, name: &str) -> PathBuf {
    let db = dir.join(name);
    std::fs::write(&db, b"this is not a sqlite database file; garbage").unwrap();
    db
}

fn indexer_at(root: &Path, db: &Path) -> Indexer {
    Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        ..IndexOptions::default()
    })
    .expect("indexer")
}

fn searcher_at(root: &Path, db: &Path) -> Result<Searcher, StoreError> {
    Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        ..SearchOptions::default()
    })
}

/// MR1: the same corrupt-bytes fault through the store open, the read-layer
/// constructor, and the write-layer constructor yields the same discriminant
/// AND the same sqlite kind — no layer converts or downgrades it.
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

/// MR1: a file blocking the index directory fails identically through the
/// store open and the write-layer constructor (both `Other` by the deliberate
/// `create_dir_all` Io→Other map).
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

/// MR1: one oversize query through every lexical search lane — including the
/// multi-pattern fan-in, which delegates to `search` — yields `Other`
/// everywhere. No lane answers, truncates, or converts.
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

/// MR1: one dropped table (`pattern_nodes`) observed through the depth query,
/// the status-adjacent free function, the prefixed search lane, and the
/// multi-pattern fan-in yields `Database` everywhere with the same sqlite
/// kind — the fault is identical, so the discriminant must be too.
#[test]
fn dropped_table_same_discriminant_across_depth_and_search_lanes() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open_in_memory(temp.path()).expect("in-memory store");
    store
        .connection()
        .execute_batch("DROP TABLE pattern_nodes")
        .unwrap();
    let via_depth = err_of(store.pattern_node_count().map(|_| ()));
    let via_free_fn =
        err_of(search_pattern("greet_user", &store, temp.path(), None, 10).map(|_| ()));
    let searcher = Searcher::with_store(
        IndexStore::open_in_memory(temp.path()).expect("second store"),
        SearchOptions {
            root: temp.path().to_path_buf(),
            limit: 10,
            use_embed: false,
            ..SearchOptions::default()
        },
    );
    searcher
        .store()
        .connection()
        .execute_batch("DROP TABLE pattern_nodes")
        .unwrap();
    let via_search = err_of(searcher.search("pattern:greet_user").map(|_| ()));
    let via_multi =
        err_of(searcher.search_multi_pattern(&["greet_user".to_string()]).map(|_| ()));
    for (lane, err) in [
        ("depth query", &via_depth),
        ("free function", &via_free_fn),
        ("prefixed search", &via_search),
        ("multi-pattern", &via_multi),
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
}

/// MR1 (status path): dropping a table the status probe counts fails the
/// status probe as `Database` — the same discriminant the depth query for a
/// dropped table yields. Status never answers `Ok` over a broken schema.
#[test]
fn dropped_table_fails_status_with_database_discriminant() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open_in_memory(temp.path()).expect("in-memory store");
    assert!(
        store.status().is_ok(),
        "control: status on a healthy store must be Ok"
    );
    store
        .connection()
        .execute_batch("DROP TABLE symbols")
        .unwrap();
    let via_status = err_of(store.status().map(|_| ()));
    assert_eq!(
        discriminant(&via_status),
        0,
        "dropped table via status must fail as Database, got {via_status:?}"
    );
    assert!(
        sqlite_code(&via_status).is_some(),
        "status failure must carry a sqlite kind, got {via_status:?}"
    );
}

/// MR2: the same four triggers, run twice against fresh fixtures, yield the
/// identical discriminant sequence (and identical sqlite kinds). Error
/// identity is a pure function of the trigger — no run-to-run drift.
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

/// MR2: a failure never poisons later success through the same handle — an
/// `Err` followed by a valid call is `Ok`, and the `Err` repeated is the
/// same discriminant. Errors are stateless rejections, not handle poison.
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

/// MR3: with `pattern_nodes` dropped, every pattern lane returns `Err` —
/// never `Ok` with silently empty hits. Control first: the same three lanes
/// answer `Ok` on the healthy store, so the `Err`s are the poison, not the
/// query.
#[test]
fn poisoned_store_fails_closed_across_pattern_lanes() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open_in_memory(temp.path()).expect("in-memory store");
    assert!(
        search_pattern("greet_user", &store, temp.path(), None, 10).is_ok(),
        "control: free function on healthy store must be Ok"
    );
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
    store
        .connection()
        .execute_batch("DROP TABLE pattern_nodes")
        .unwrap();
    searcher
        .store()
        .connection()
        .execute_batch("DROP TABLE pattern_nodes")
        .unwrap();
    let errs = [
        (
            "free function",
            err_of(search_pattern("greet_user", &store, temp.path(), None, 10).map(|_| ())),
        ),
        (
            "prefixed search",
            err_of(searcher.search("pattern:greet_user").map(|_| ())),
        ),
        (
            "multi-pattern",
            err_of(
                searcher
                    .search_multi_pattern(&["greet_user".to_string()])
                    .map(|_| ()),
            ),
        ),
    ];
    for (lane, err) in &errs {
        assert_eq!(
            discriminant(err),
            0,
            "poisoned store via {lane} must fail closed as Database, got {err:?}"
        );
    }
}

/// MR3: a set cancel flag fails every write entry point as `Other` — full
/// index and incremental update agree, and neither reports `Ok` stats for
/// work it did not do.
#[test]
fn cancelled_writes_fail_closed_with_same_discriminant() {
    let temp = TempDir::new().unwrap();
    let rel = temp.path().join("a.rs");
    std::fs::write(&rel, b"fn alpha() {}\n").unwrap();
    let db = temp.path().join("index.db");
    let mut indexer = indexer_at(temp.path(), &db);
    let cancel = Arc::new(AtomicBool::new(true));
    indexer.set_cancel(Arc::clone(&cancel));
    let via_index_all = err_of(indexer.index_all().map(|_| ()));
    let via_update = err_of(indexer.update_paths(std::slice::from_ref(&rel)).map(|_| ()));
    assert_eq!(
        (
            discriminant(&via_index_all),
            discriminant(&via_update)
        ),
        (2, 2),
        "cancelled writes must fail as Other through both entries, got {via_index_all:?} / {via_update:?}"
    );
    cancel.store(false, Ordering::SeqCst);
}

/// MR4: failed opens leave the filesystem byte-identical — no half-created
/// index file, no repaired garbage, no directory entries added or removed.
/// Snapshot before, trigger, snapshot after; the maps must match exactly.
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

/// MR4: failed writes leave committed store state intact — a previously
/// indexed file keeps its exact hash and the line count is unchanged after
/// an over-max batch, a cancelled update, and a cancelled full index, all
/// of which report the same `Other` discriminant.
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

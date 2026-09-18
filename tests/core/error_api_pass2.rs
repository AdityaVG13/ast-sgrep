//! E2 error-propagation oracles for ast-sgrep-core.
//!
//! E1 pins taxonomy (each `StoreError` variant is reachable). E2 pins
//! PROPAGATION: a depth error must surface through every layer with the
//! discriminant (and sqlite/io kind) the layer contract promises — never
//! `Ok`, never a panic, never a silent downgrade. Discriminants and
//! downcast kinds only (`matches!`); no message-text asserts.
//!
//! Site inventory (from `map_err` / `.ok()` / `?` grep over
//! `crates/ast-sgrep-core/src`); each row names its pinning test:
//!
//! - `io_bounds::RootDir::open` rustix `map_err(io::Error)` + `?` (Io
//!   preserved) → `indexer_new_missing_root_surfaces_io`,
//!   `indexer_new_file_as_root_surfaces_io`.
//! - `Indexer::new` `canonicalize().unwrap_or(...)` swallow (harmless:
//!   `RootDir::open` fails after) → `indexer_new_missing_root_surfaces_io`.
//! - `Searcher::new` `canonicalize().map_err(... Other ...)` (deliberate
//!   Io→Other with context) → `searcher_new_missing_root_converts_to_other`.
//! - `store::sqlite::open_inner` `create_dir_all.map_err(... Other ...)`
//!   (deliberate Io→Other) → `store_open_uncreatable_index_dir_converts_to_other`.
//! - `store::sqlite::open_inner` `Connection::open ?` (`#[from]` Database,
//!   kind preserved) + `init_schema ?` → `db_garbage_kind_preserved_*`,
//!   `db_garbage_through_indexer_new_without_force_stays_database`.
//! - `open_inner` read-only missing-db guard (Other, fail-closed) →
//!   `missing_db_through_searcher_new_fails_closed`.
//! - `pattern::search_pattern` `store.pattern_node_count()?` →
//!   `db_dropped_table_through_search_pattern_stays_database`.
//! - `Indexer::update_paths` over-max gate before `ignore.clear()`/indexing
//!   → `update_paths_over_max_rejects_before_side_effect`.
//! - `Indexer::check_cancel ?` at `index_all` entry and per update path →
//!   `cancelled_index_all_rejects_before_side_effect`,
//!   `cancelled_update_paths_rejects_before_side_effect`.
//! - `update_paths` per-file `index_failure` counted isolation
//!   (`files_failed += 1`, batch stays `Ok`) → `per_file_failure_is_counted`.
//!
//! Deliberately NOT pinned here: `Searcher::index_gen` / `cached_stamp_parts`
//! `.ok()?` sites (fail-open to recompute by documented design, hdwh — no
//! deterministic public trigger for a PRAGMA failure), `pattern.rs` walk
//! `File::open(path).ok()?` skip-unreadable (walk-level file skip, covered
//! by walk-behavior suites), `Durability::from_env` `.ok().unwrap_or_default`
//! (env parsing, not error propagation).

use ast_sgrep_core::{
    search_pattern, IndexOptions, IndexStore, Indexer, SearchOptions, Searcher, StoreError,
    MAX_INCREMENTAL_PATHS,
};
use rusqlite::ErrorCode;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tempfile::TempDir;

fn err_of<T>(result: Result<T, StoreError>) -> StoreError {
    match result {
        Ok(_) => panic!("expected Err, got Ok"),
        Err(err) => err,
    }
}

/// Public caller-side replica of the crate-private `is_corrupt_database`
/// predicate. E2 proves corruption stays detectable through the `Database`
/// discriminant + rusqlite kind alone, at every layer.
fn is_corrupt_kind(err: &StoreError) -> bool {
    matches!(
        err,
        StoreError::Database(rusqlite::Error::SqliteFailure(code, _))
            if matches!(
                code.code,
                ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase
            )
    )
}

fn sqlite_code(err: &StoreError) -> Option<ErrorCode> {
    match err {
        StoreError::Database(rusqlite::Error::SqliteFailure(code, _)) => Some(code.code),
        _ => None,
    }
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

/// `RootDir::open` ENOENT propagates through `Indexer::new` as `Io` (the
/// `canonicalize().unwrap_or` above it swallows nothing observable: the open
/// fails after). The db path must not be created: the root check precedes
/// any store side effect.
#[test]
fn indexer_new_missing_root_surfaces_io() {
    let temp = TempDir::new().unwrap();
    let missing = temp.path().join("nosuch-root");
    let db = temp.path().join("index.db");
    let err = err_of(Indexer::new(IndexOptions {
        root: missing,
        index_path: Some(db.clone()),
        ..IndexOptions::default()
    }));
    match &err {
        StoreError::Io(io) => assert_eq!(
            io.kind(),
            std::io::ErrorKind::NotFound,
            "missing root must surface NotFound kind, got {err:?}"
        ),
        other => panic!("missing root must fail as Io, got {other:?}"),
    }
    assert!(
        !db.exists(),
        "failed root open must not create the index file"
    );
}

/// A regular file as root fails `RootDir::open` (`O_DIRECTORY`) and still
/// surfaces as `Io` — not `Other`, not `Ok`, not a panic.
#[test]
fn indexer_new_file_as_root_surfaces_io() {
    let temp = TempDir::new().unwrap();
    let file_root = temp.path().join("file.rs");
    std::fs::write(&file_root, b"fn main() {}\n").unwrap();
    let err = err_of(Indexer::new(IndexOptions {
        root: file_root,
        index_path: Some(temp.path().join("index.db")),
        ..IndexOptions::default()
    }));
    assert!(
        matches!(err, StoreError::Io(_)),
        "file-as-root must fail as Io, got {err:?}"
    );
}

/// Same depth error as above through the read layer: `Searcher::new`
/// deliberately converts the `canonicalize` Io into `Other` with context
/// (search/mod.rs map site). Fail-closed, never silent `Ok`.
#[test]
fn searcher_new_missing_root_converts_to_other() {
    let temp = TempDir::new().unwrap();
    let err = err_of(Searcher::new(SearchOptions {
        root: temp.path().join("nosuch-root"),
        ..SearchOptions::default()
    }));
    assert!(
        matches!(err, StoreError::Other(_)),
        "Searcher::new missing root must fail as Other, got {err:?}"
    );
}

/// `open_inner` maps `create_dir_all` Io into `Other` (sqlite/mod.rs map
/// site). A file blocking the index directory therefore fails as `Other`,
/// deterministically, instead of leaking a raw Io or panicking.
#[test]
fn store_open_uncreatable_index_dir_converts_to_other() {
    let temp = TempDir::new().unwrap();
    let blocker = temp.path().join("blocker");
    std::fs::write(&blocker, b"i am a file, not a directory").unwrap();
    let err = err_of(IndexStore::open(
        temp.path(),
        Some(&blocker.join("index.db")),
    ));
    assert!(
        matches!(err, StoreError::Other(_)),
        "blocked index dir must fail as Other, got {err:?}"
    );
}

/// Garbage bytes fail `open` as `Database` with the sqlite kind intact:
/// `NotADatabase`, detectable as corruption through the public downcast.
/// E1 pins the discriminant; E2 pins the kind preservation.
#[test]
fn db_garbage_kind_preserved_through_store_open() {
    let temp = TempDir::new().unwrap();
    let db = write_garbage_db(temp.path(), "index.db");
    let err = err_of(IndexStore::open(temp.path(), Some(&db)));
    assert_eq!(
        sqlite_code(&err),
        Some(ErrorCode::NotADatabase),
        "garbage db must preserve NotADatabase kind, got {err:?}"
    );
    assert!(
        is_corrupt_kind(&err),
        "garbage db must stay corruption-detectable, got {err:?}"
    );
}

/// The same depth failure through `Searcher::new` (readonly open +
/// readonly configure + readonly schema probe): still `Database` with the
/// kind preserved — the read layer adds no conversion.
#[test]
fn db_garbage_through_searcher_new_stays_database() {
    let temp = TempDir::new().unwrap();
    let db = write_garbage_db(temp.path(), "index.db");
    let err = err_of(Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(db),
        ..SearchOptions::default()
    }));
    assert!(
        matches!(err, StoreError::Database(_)),
        "garbage db through Searcher::new must stay Database, got {err:?}"
    );
    assert!(
        is_corrupt_kind(&err),
        "garbage db kind must survive the read layer, got {err:?}"
    );
}

/// Without `force_reindex`, `Indexer::new` propagates the open `Database`
/// error untouched (no recovery, no `Ok`) and leaves the file byte-identical:
/// rejection precedes any repair side effect.
#[test]
fn db_garbage_through_indexer_new_without_force_stays_database() {
    let temp = TempDir::new().unwrap();
    let db = write_garbage_db(temp.path(), "index.db");
    let before = std::fs::read(&db).unwrap();
    let err = err_of(Indexer::new(IndexOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(db.clone()),
        ..IndexOptions::default()
    }));
    assert!(
        matches!(err, StoreError::Database(_)),
        "garbage db through Indexer::new must stay Database, got {err:?}"
    );
    assert!(
        is_corrupt_kind(&err),
        "garbage db kind must survive Indexer::new, got {err:?}"
    );
    assert_eq!(
        std::fs::read(&db).unwrap(),
        before,
        "rejected open must not mutate the db file"
    );
}

/// A sqlite failure below `search_pattern` (`pattern_node_count()?`)
/// propagates as `Database` — the search lane never degrades it to `Ok`
/// empty or `Other`. Control first: the same call on the healthy store is
/// `Ok`, so the `Err` is the poisoned depth query, not the pattern.
#[test]
fn db_dropped_table_through_search_pattern_stays_database() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open_in_memory(temp.path()).expect("in-memory store");
    assert_eq!(
        store.pattern_node_count().expect("table exists"),
        0,
        "fresh store must have an empty pattern_nodes table"
    );
    assert!(
        search_pattern("greet_user", &store, temp.path(), None, 10).is_ok(),
        "control: healthy store must answer Ok"
    );
    store
        .connection()
        .execute_batch("DROP TABLE pattern_nodes")
        .unwrap();
    assert!(
        matches!(
            store.pattern_node_count(),
            Err(StoreError::Database(_))
        ),
        "dropped table must fail direct count as Database"
    );
    let err = err_of(search_pattern("greet_user", &store, temp.path(), None, 10));
    assert!(
        matches!(
            err,
            StoreError::Database(rusqlite::Error::SqliteFailure(_, _))
        ),
        "dropped table through search_pattern must stay Database(SqliteFailure), got {err:?}"
    );
}

/// The over-max gate fires before `ignore.clear()` and before any file is
/// touched: `Other` plus a store that never saw the batch.
#[test]
fn update_paths_over_max_rejects_before_side_effect() {
    let temp = TempDir::new().unwrap();
    let rel = temp.path().join("a.rs");
    std::fs::write(&rel, b"fn alpha() {}\n").unwrap();
    let db = temp.path().join("index.db");
    let mut indexer = indexer_at(temp.path(), &db);
    let paths = vec![rel; MAX_INCREMENTAL_PATHS + 1];
    let err = err_of(indexer.update_paths(&paths).map(|_| ()));
    assert!(
        matches!(err, StoreError::Other(_)),
        "over-max batch must fail as Other, got {err:?}"
    );
    assert_eq!(
        indexer.store().file_hash("a.rs").expect("store readable"),
        None,
        "rejected batch must not index its first valid path"
    );
}

/// Cancel-before-walk: `index_all` checks the flag at entry, returns `Other`,
/// and indexes nothing even though an indexable file is present.
#[test]
fn cancelled_index_all_rejects_before_side_effect() {
    let temp = TempDir::new().unwrap();
    std::fs::write(temp.path().join("a.rs"), b"fn alpha() {}\n").unwrap();
    let db = temp.path().join("index.db");
    let mut indexer = indexer_at(temp.path(), &db);
    let cancel = Arc::new(AtomicBool::new(false));
    indexer.set_cancel(Arc::clone(&cancel));
    cancel.store(true, Ordering::SeqCst);
    let err = err_of(indexer.index_all().map(|_| ()));
    assert!(
        matches!(err, StoreError::Other(_)),
        "cancelled index_all must fail as Other, got {err:?}"
    );
    assert_eq!(
        indexer.store().file_hash("a.rs").expect("store readable"),
        None,
        "cancelled index_all must index nothing"
    );
}

/// Per-path cancel: `update_paths` checks the flag before each path, so a
/// cancelled single-file update returns `Other` without indexing the file.
#[test]
fn cancelled_update_paths_rejects_before_side_effect() {
    let temp = TempDir::new().unwrap();
    let rel = temp.path().join("a.rs");
    std::fs::write(&rel, b"fn alpha() {}\n").unwrap();
    let db = temp.path().join("index.db");
    let mut indexer = indexer_at(temp.path(), &db);
    let cancel = Arc::new(AtomicBool::new(false));
    indexer.set_cancel(Arc::clone(&cancel));
    cancel.store(true, Ordering::SeqCst);
    let err = err_of(indexer.update_paths(std::slice::from_ref(&rel)).map(|_| ()));
    assert!(
        matches!(err, StoreError::Other(_)),
        "cancelled update_paths must fail as Other, got {err:?}"
    );
    assert_eq!(
        indexer.store().file_hash("a.rs").expect("store readable"),
        None,
        "cancelled update_paths must index nothing"
    );
}

/// Per-file isolation degrades loudly, not silently: a binary `.rs` file
/// fails its own path while its healthy sibling indexes; the batch stays
/// `Ok` with the failure COUNTED in `files_failed` (plus a stderr line) —
/// neither a batch `Err` that drops the sibling nor a silent skip.
#[test]
fn per_file_failure_is_counted_not_silent() {
    let temp = TempDir::new().unwrap();
    let good = temp.path().join("good.rs");
    std::fs::write(&good, b"fn greet() {}\n").unwrap();
    let blob = temp.path().join("blob.rs");
    std::fs::write(&blob, [0xffu8, 0xfe, 0x00, 0x61]).unwrap();
    let db = temp.path().join("index.db");
    let mut indexer = indexer_at(temp.path(), &db);
    let stats = indexer
        .update_paths(&[good, blob])
        .expect("per-file failure must not fail the batch");
    assert_eq!(stats.files_indexed, 1, "healthy sibling must index");
    assert_eq!(stats.files_failed, 1, "binary file must be counted failed");
    assert!(
        indexer
            .store()
            .file_hash("good.rs")
            .expect("store readable")
            .is_some(),
        "healthy sibling must be indexed"
    );
    assert_eq!(
        indexer.store().file_hash("blob.rs").expect("store readable"),
        None,
        "failed file must leave no row"
    );
}

/// Search-before-index fails closed as `Other` through `Searcher::new`
/// (readonly missing-db guard) — never an `Ok` searcher silently answering
/// empty. Control: after indexing, the same constructor is `Ok`, proving
/// the `Other` was the missing db, not the root.
#[test]
fn missing_db_through_searcher_new_fails_closed() {
    let temp = TempDir::new().unwrap();
    std::fs::write(temp.path().join("a.rs"), b"fn alpha() {}\n").unwrap();
    let db = temp.path().join("index.db");
    let err = err_of(Searcher::new(SearchOptions {
        root: temp.path().to_path_buf(),
        index_path: Some(db.clone()),
        ..SearchOptions::default()
    }));
    assert!(
        matches!(err, StoreError::Other(_)),
        "missing db through Searcher::new must fail as Other, got {err:?}"
    );
    let mut indexer = indexer_at(temp.path(), &db);
    indexer.index_all().expect("index the fixture");
    assert!(
        Searcher::new(SearchOptions {
            root: temp.path().to_path_buf(),
            index_path: Some(db),
            ..SearchOptions::default()
        })
        .is_ok(),
        "control: Searcher::new must succeed once the db exists"
    );
}

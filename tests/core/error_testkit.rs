#![allow(dead_code)]
//! Shared kit for the consolidated core error-API suites (`error_api_pass1..4`,
//! included via `#[path = "error_testkit.rs"]`).
//!
//! Reuse rule: anything the testkit crate already provides is imported from
//! `ast_sgrep_testkit` at the use site (`err_of`, `write_garbage`,
//! `truncate_file`, `remove_sqlite_sidecars` from `testkit::fault`) — never
//! redefined here. The `StoreError` discriminant/kind projections are
//! re-exported from testkit below (single source of truth); this module
//! holds only the area-local fixture constructors. Plain keep-comments mark
//! area-local helpers.
//!
//! The `allow(dead_code)` is load-bearing: each pass file uses a subset of
//! this kit, and an unused helper in one target must not warn in another.

pub use ast_sgrep_testkit::{
    is_corrupt_kind, sqlite_code, store_error_discriminant as discriminant,
};

use ast_sgrep_core::{IndexOptions, IndexStore, Indexer, SearchOptions, Searcher, StoreError};
use std::path::{Path, PathBuf};

// Keep local: in-memory-store searcher wired for error-ingress tests
// (explicit root, embed off). Error-API-specific options; not a testkit shape.
pub fn mem_searcher(root: &Path) -> Searcher {
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

// Keep local: thin (dir, name) adapter over the canonical testkit fault
// builder `ast_sgrep_testkit::write_garbage`; keeps error-test call sites
// stable without duplicating the sentinel bytes.
pub fn write_garbage_db(dir: &Path, name: &str) -> PathBuf {
    let db = dir.join(name);
    ast_sgrep_testkit::write_garbage(&db);
    db
}

// Keep local: write-layer constructor over an explicit db path.
pub fn indexer_at(root: &Path, db: &Path) -> Indexer {
    Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        ..IndexOptions::default()
    })
    .expect("indexer")
}

// Keep local: read-layer constructor over an explicit db path.
pub fn searcher_at(root: &Path, db: &Path) -> Result<Searcher, StoreError> {
    Searcher::new(SearchOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        ..SearchOptions::default()
    })
}

// Keep local (E4 drills): populated-file body; the committed-hash anchor every
// rebuild drill reproduces to prove no-half-writes.
const FIXTURE: &[u8] = b"fn alpha() {}\nfn greet_user() {}\n";

// Keep local (E4 drills): populate a real file-backed store and drop the
// writer; returns the committed `file_hash` so drills can prove no-half-writes
// afterward.
pub fn populate(root: &Path, db: &Path) -> Option<String> {
    std::fs::write(root.join("a.rs"), FIXTURE).unwrap();
    let mut indexer = Indexer::new(IndexOptions {
        root: root.to_path_buf(),
        index_path: Some(db.to_path_buf()),
        ..IndexOptions::default()
    })
    .expect("populate indexer");
    indexer.index_all().expect("populate index_all");
    let hash = indexer.store().file_hash("a.rs").expect("hash readable");
    assert!(hash.is_some(), "populated file must commit a hash");
    hash
}

// Keep local: total teardown — the db itself plus sqlite sidecars (testkit's
// `remove_sqlite_sidecars` keeps the main db; drills need a truly clean
// rebuild, so the main file goes too).
pub fn remove_db_files(db: &Path) {
    let _ = std::fs::remove_file(db);
    ast_sgrep_testkit::remove_sqlite_sidecars(db);
}

// Keep local (E4 drills): clean-state proof shared by the rebuild drills —
// the rebuilt store serves search + status and reproduces the exact committed
// hash.
pub fn assert_clean_state(root: &Path, db: &Path, committed: &Option<String>) {
    let searcher = searcher_at(root, db).expect("clean Searcher::new must succeed");
    assert!(
        searcher.search("alpha").is_ok(),
        "clean search after recovery must succeed"
    );
    assert!(
        searcher.store().status().is_ok(),
        "clean status after recovery must succeed"
    );
    assert_eq!(
        searcher.store().file_hash("a.rs").expect("hash readable"),
        *committed,
        "rebuilt store must reproduce the committed hash (no half-writes)"
    );
}

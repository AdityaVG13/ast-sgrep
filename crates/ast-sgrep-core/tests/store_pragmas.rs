use std::path::Path;

use ast_sgrep_core::IndexStore;
use tempfile::TempDir;

#[test]
fn index_store_applies_wal_and_busy_timeout() {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path();
    let store = IndexStore::open(root, None).expect("open index");

    let journal_mode: String = store
        .connection()
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("journal_mode");
    assert_eq!(journal_mode.to_ascii_lowercase(), "wal");

    let synchronous: i64 = store
        .connection()
        .query_row("PRAGMA synchronous", [], |row| row.get(0))
        .expect("synchronous");
    assert_eq!(synchronous, 1, "NORMAL synchronous mode");

    let foreign_keys: i64 = store
        .connection()
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .expect("foreign_keys");
    assert_eq!(foreign_keys, 1);

    let busy_ms: i64 = store
        .connection()
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .expect("busy_timeout");
    assert_eq!(busy_ms, 5_000);

    let integrity = ast_sgrep_core::store::integrity_check(store.connection()).expect("check");
    assert_eq!(integrity, "ok");

    assert!(store.db_path().starts_with(root.join(".asgrep")));
    assert!(Path::new(&store.db_path()).exists());
}

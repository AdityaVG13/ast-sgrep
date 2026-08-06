use ast_sgrep_core::IndexStore;
use std::path::Path;
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

#[test]
fn file_tx_restores_synchronous_normal_after_commit_and_rollback() {
    let temp = TempDir::new().expect("tempdir");
    let store = IndexStore::open(temp.path(), None).expect("open index");
    let sync = |s: &IndexStore| -> i64 {
        s.connection()
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .expect("synchronous")
    };
    assert_eq!(sync(&store), 1, "open defaults to NORMAL");

    store.begin_file_tx().expect("begin");
    store.commit_file_tx().expect("commit");
    assert_eq!(sync(&store), 1, "commit restores NORMAL");

    store.begin_file_tx().expect("begin2");
    store.rollback_file_tx().expect("rollback");
    assert_eq!(sync(&store), 1, "rollback restores NORMAL");
}

#[test]
fn bulk_tx_rollback_restores_synchronous_normal() {
    let temp = TempDir::new().expect("tempdir");
    let store = IndexStore::open(temp.path(), None).expect("open index");
    store.begin_bulk_tx().expect("begin bulk");
    store.rollback_bulk_tx().expect("rollback bulk");
    let synchronous: i64 = store
        .connection()
        .query_row("PRAGMA synchronous", [], |row| row.get(0))
        .expect("synchronous");
    assert_eq!(synchronous, 1, "bulk rollback restores NORMAL");
}

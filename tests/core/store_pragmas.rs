use ast_sgrep_core::IndexStore;
use ast_sgrep_testkit::isolated_index_session;

#[test]
fn index_store_applies_wal_and_busy_timeout() {
    // Private on-disk SQLite; explicit index_path (ignores ASGREP_INDEX_PATH).
    let session = isolated_index_session();
    let store = session.open_store();
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
    assert_eq!(store.db_path(), session.index_path);
    assert!(
        session.index_path.is_file(),
        "real on-disk db must exist at {}",
        session.index_path.display()
    );
}

#[test]
fn file_tx_restores_synchronous_normal_after_commit_and_rollback() {
    let session = isolated_index_session();
    let store = session.open_store();
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
    let session = isolated_index_session();
    let store = session.open_store();
    store.begin_bulk_tx().expect("begin bulk");
    store.rollback_bulk_tx().expect("rollback bulk");
    let synchronous: i64 = store
        .connection()
        .query_row("PRAGMA synchronous", [], |row| row.get(0))
        .expect("synchronous");
    assert_eq!(synchronous, 1, "bulk rollback restores NORMAL");
}

/// 0obi: each durability profile must hold its documented pragma both at rest
/// and inside a write batch. `fast-unsafe` is the only path to OFF.
#[test]
fn durability_profiles_control_synchronous_pragma() {
    use ast_sgrep_core::store::Durability;

    let sync = |store: &IndexStore| -> i64 {
        store
            .connection()
            .query_row("PRAGMA synchronous", [], |row| row.get(0))
            .expect("synchronous")
    };

    for (profile, steady, in_write) in [
        (Durability::Strict, 2_i64, 2_i64),
        (Durability::Balanced, 1, 1),
        (Durability::FastUnsafe, 1, 0),
    ] {
        let session = isolated_index_session();
        let store = session.open_store_with_durability(profile);
        assert_eq!(store.durability(), profile);
        assert_eq!(sync(&store), steady, "{profile:?} at rest");

        // Bulk write batch.
        store.begin_bulk_tx().expect("begin bulk");
        assert_eq!(sync(&store), in_write, "{profile:?} inside bulk tx");
        store.commit_bulk_tx().expect("commit bulk");
        assert_eq!(sync(&store), steady, "{profile:?} after bulk commit");

        // Per-file write batch.
        store.begin_file_tx().expect("begin file");
        assert_eq!(sync(&store), in_write, "{profile:?} inside file tx");
        store.rollback_file_tx().expect("rollback file");
        assert_eq!(sync(&store), steady, "{profile:?} after file rollback");

        // WAL is required by every profile.
        let journal: String = store
            .connection()
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .expect("journal_mode");
        assert_eq!(journal.to_ascii_lowercase(), "wal", "{profile:?} journal");

        // The active profile is visible to operators.
        assert_eq!(store.status().expect("status").durability, profile.as_str());
    }
}

/// 0obi: the default must be `balanced`, and nothing may reach OFF implicitly.
#[test]
fn default_durability_never_reaches_synchronous_off() {
    use ast_sgrep_core::store::Durability;

    assert_eq!(Durability::default(), Durability::Balanced);
    assert_eq!(Durability::default().write_pragma(), "NORMAL");
    assert_ne!(Durability::Strict.write_pragma(), "OFF");
    assert_eq!(Durability::FastUnsafe.write_pragma(), "OFF");

    // Only the explicit opt-in spelling selects the unsafe profile.
    assert_eq!(
        Durability::parse("fast-unsafe"),
        Some(Durability::FastUnsafe)
    );
    assert_eq!(Durability::parse("balanced"), Some(Durability::Balanced));
    assert_eq!(Durability::parse("strict"), Some(Durability::Strict));
    // An unknown value must not silently downgrade durability.
    assert_eq!(Durability::parse("off"), None);
    assert_eq!(Durability::parse(""), None);

    let session = isolated_index_session();
    let store = session.open_store();
    assert_eq!(store.durability(), Durability::Balanced);
    store.begin_bulk_tx().expect("begin bulk");
    let during: i64 = store
        .connection()
        .query_row("PRAGMA synchronous", [], |row| row.get(0))
        .expect("synchronous");
    store.commit_bulk_tx().expect("commit bulk");
    assert_ne!(
        during, 0,
        "default indexing must never run with synchronous=OFF"
    );
}

#[test]
fn open_readonly_does_not_take_write_lock() {
    let session = isolated_index_session();
    let _writer = session.open_store();
    drop(_writer);
    let store =
        ast_sgrep_core::IndexStore::open_readonly(&session.corpus_root, Some(&session.index_path))
            .expect("readonly open");
    assert!(store.is_read_only());
    let busy_ms: i64 = store
        .connection()
        .query_row("PRAGMA busy_timeout", [], |row| row.get(0))
        .expect("busy_timeout");
    assert_eq!(busy_ms, 250);
    let query_only: i64 = store
        .connection()
        .query_row("PRAGMA query_only", [], |row| row.get(0))
        .expect("query_only");
    assert_eq!(query_only, 1);
    let err = store
        .connection()
        .execute("INSERT INTO meta(key, value) VALUES('x', 'y')", [])
        .expect_err("readonly insert");
    let msg = err.to_string().to_ascii_lowercase();
    assert!(
        msg.contains("readonly") || msg.contains("read-only") || msg.contains("query_only"),
        "unexpected insert error: {err}"
    );
}

use super::*;
use crate::store::Durability;
use tempfile::TempDir;

struct RestoreFailGuard;
impl Drop for RestoreFailGuard {
    fn drop(&mut self) {
        FORCE_RESTORE_SYNC_FAILURE.with(|c| c.set(false));
    }
}

fn force_restore_failure() -> RestoreFailGuard {
    FORCE_RESTORE_SYNC_FAILURE.with(|c| c.set(true));
    RestoreFailGuard
}

struct CommitFailGuard;
impl Drop for CommitFailGuard {
    fn drop(&mut self) {
        FORCE_COMMIT_FAILURE.with(|c| c.set(false));
    }
}

fn force_commit_failure() -> CommitFailGuard {
    FORCE_COMMIT_FAILURE.with(|c| c.set(true));
    CommitFailGuard
}

struct BeginFailGuard;
impl Drop for BeginFailGuard {
    fn drop(&mut self) {
        FORCE_BEGIN_FAILURE.with(|c| c.set(false));
    }
}

fn force_begin_failure() -> BeginFailGuard {
    FORCE_BEGIN_FAILURE.with(|c| c.set(true));
    BeginFailGuard
}

fn sync_mode(store: &IndexStore) -> i64 {
    store
        .connection()
        .query_row("PRAGMA synchronous", [], |row| row.get(0))
        .expect("PRAGMA synchronous")
}

#[test]
fn file_tx_commit_surfaces_restore_synchronous_failure() {
    let temp = TempDir::new().unwrap();
    let store =
        IndexStore::open_with_durability(temp.path(), None, Durability::FastUnsafe).unwrap();
    store.begin_file_tx().unwrap();
    assert_eq!(sync_mode(&store), 0, "FastUnsafe write batch uses OFF");
    let _guard = force_restore_failure();
    let err = store
        .commit_file_tx()
        .expect_err("restore failure must not be swallowed");
    assert!(
        err.to_string().contains("restore_synchronous"),
        "unexpected error: {err}"
    );
    // Tx bookkeeping cleared even when restore fails.
    assert!(store.connection().is_autocommit());
    assert_eq!(store.file_tx_depth.get(), 0);
}

#[test]
fn file_tx_rollback_surfaces_restore_synchronous_failure() {
    let temp = TempDir::new().unwrap();
    let store =
        IndexStore::open_with_durability(temp.path(), None, Durability::FastUnsafe).unwrap();
    store.begin_file_tx().unwrap();
    let _guard = force_restore_failure();
    let err = store
        .rollback_file_tx()
        .expect_err("restore failure must not be swallowed on rollback");
    assert!(
        err.to_string().contains("restore_synchronous"),
        "unexpected error: {err}"
    );
    assert_eq!(store.file_tx_depth.get(), 0);
}

#[test]
fn bulk_tx_commit_surfaces_restore_synchronous_failure() {
    let temp = TempDir::new().unwrap();
    let store =
        IndexStore::open_with_durability(temp.path(), None, Durability::FastUnsafe).unwrap();
    store.begin_bulk_tx().unwrap();
    let _guard = force_restore_failure();
    let err = store
        .commit_bulk_tx()
        .expect_err("restore failure must not be swallowed on bulk commit");
    assert!(
        err.to_string().contains("restore_synchronous"),
        "unexpected error: {err}"
    );
    assert!(store.connection().is_autocommit());
}

#[test]
fn bulk_tx_rollback_surfaces_restore_synchronous_failure() {
    let temp = TempDir::new().unwrap();
    let store =
        IndexStore::open_with_durability(temp.path(), None, Durability::FastUnsafe).unwrap();
    store.begin_bulk_tx().unwrap();
    let _guard = force_restore_failure();
    let err = store
        .rollback_bulk_tx()
        .expect_err("restore failure must not be swallowed on bulk rollback");
    assert!(
        err.to_string().contains("restore_synchronous"),
        "unexpected error: {err}"
    );
    assert!(store.connection().is_autocommit());
}

#[test]
fn file_tx_commit_failure_rolls_back_and_clears_bookkeeping() {
    let temp = TempDir::new().unwrap();
    let store =
        IndexStore::open_with_durability(temp.path(), None, Durability::FastUnsafe).unwrap();
    store.begin_file_tx().unwrap();
    let guard = force_commit_failure();
    let err = store
        .commit_file_tx()
        .expect_err("forced COMMIT failure must surface");
    drop(guard);

    assert!(err.to_string().contains("COMMIT forced failure"));
    assert!(store.connection().is_autocommit());
    assert_eq!(store.file_tx_depth.get(), 0);
    assert_eq!(sync_mode(&store), 1, "steady synchronous mode restored");
    store.begin_file_tx().expect("next transaction can begin");
    store.rollback_file_tx().expect("next transaction can end");
}

#[test]
fn fast_unsafe_begin_failure_restores_safe_steady_state() {
    let temp = TempDir::new().unwrap();
    let store =
        IndexStore::open_with_durability(temp.path(), None, Durability::FastUnsafe).unwrap();
    let guard = force_begin_failure();
    let file_error = store
        .begin_file_tx()
        .expect_err("forced file BEGIN failure must surface");
    assert!(file_error.to_string().contains("BEGIN forced failure"));
    assert!(store.connection().is_autocommit());
    assert_eq!(sync_mode(&store), 1, "file admission restored NORMAL");

    let bulk_error = store
        .begin_bulk_tx()
        .expect_err("forced bulk BEGIN failure must surface");
    drop(guard);
    assert!(bulk_error.to_string().contains("BEGIN forced failure"));
    assert!(store.connection().is_autocommit());
    assert!(!store.bulk_tx_active.get());
    assert_eq!(sync_mode(&store), 1, "bulk admission restored NORMAL");
}

#[test]
fn bulk_tx_commit_failure_rolls_back_and_clears_bookkeeping() {
    let temp = TempDir::new().unwrap();
    let store =
        IndexStore::open_with_durability(temp.path(), None, Durability::FastUnsafe).unwrap();
    store.begin_bulk_tx().unwrap();
    let guard = force_commit_failure();
    let err = store
        .commit_bulk_tx()
        .expect_err("forced COMMIT failure must surface");
    drop(guard);

    assert!(err.to_string().contains("COMMIT forced failure"));
    assert!(store.connection().is_autocommit());
    assert!(!store.bulk_tx_active.get());
    assert_eq!(sync_mode(&store), 1, "steady synchronous mode restored");
    store.begin_bulk_tx().expect("next transaction can begin");
    store.rollback_bulk_tx().expect("next transaction can end");
}

#[test]
fn nested_bulk_tx_does_not_end_transaction_it_does_not_own() {
    let temp = TempDir::new().unwrap();
    let store = IndexStore::open(temp.path(), None).unwrap();
    store.connection().execute_batch("BEGIN IMMEDIATE").unwrap();

    store.begin_bulk_tx().unwrap();
    store.commit_bulk_tx().unwrap();

    assert!(
        !store.connection().is_autocommit(),
        "bulk helper must not commit its caller's transaction"
    );
    store.connection().execute_batch("ROLLBACK").unwrap();
}

/// Pass9 residual of d2a1.2: product `index_all` used `let _ = rollback_bulk_tx()`
/// after a write Err. `apply_bulk_write_result` must surface restore failure
/// instead of returning only the original write error.
#[test]
fn apply_bulk_write_result_prefers_restore_failure_over_write_err() {
    let temp = TempDir::new().unwrap();
    let store =
        IndexStore::open_with_durability(temp.path(), None, Durability::FastUnsafe).unwrap();
    store.begin_bulk_tx().unwrap();
    let _guard = force_restore_failure();
    let write_err = crate::StoreError::Other("simulated bulk write failure".into());
    let err = store
        .apply_bulk_write_result(Err(write_err))
        .expect_err("restore failure must win over write Err");
    assert!(
        err.to_string().contains("restore_synchronous"),
        "swallowed restore behind write err: {err}"
    );
    assert!(
        !err.to_string().contains("simulated bulk write"),
        "must not prefer original write err when restore fails: {err}"
    );
    assert!(store.connection().is_autocommit());
}

#[test]
fn apply_bulk_write_result_returns_write_err_when_rollback_ok() {
    let temp = TempDir::new().unwrap();
    let store =
        IndexStore::open_with_durability(temp.path(), None, Durability::FastUnsafe).unwrap();
    store.begin_bulk_tx().unwrap();
    let write_err = crate::StoreError::Other("simulated bulk write failure".into());
    let err = store
        .apply_bulk_write_result(Err(write_err))
        .expect_err("write Err must surface when rollback succeeds");
    assert!(
        err.to_string().contains("simulated bulk write"),
        "unexpected error: {err}"
    );
    assert!(store.connection().is_autocommit());
    // Steady pragma restored after successful rollback path.
    let sync: i64 = store
        .connection()
        .query_row("PRAGMA synchronous", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        sync, 1,
        "FastUnsafe steady restores to NORMAL between batches"
    );
}

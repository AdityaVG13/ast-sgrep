//! Drift guard for the `index_db_path` fork (untangle-campaign F39).
//!
//! INTENT: `ast_sgrep_testkit::index_db_path(root)` duplicates the default
//! arm of `ast_sgrep_core::index_db_path` ON PURPOSE — blind dedupe is
//! explicitly NOT wanted. The core resolver consults `ASGREP_INDEX_PATH` /
//! `ASGREP_USE_CACHE` and the filesystem, so it cannot pin a hermetic layout;
//! the testkit fork is the fixed `<root>/.asgrep/index.db` constructor only
//! (see the `INTENT` comment on the testkit fork site in
//! `crates/ast-sgrep-testkit/src/fixture.rs`).
//!
//! KILLS: the "helpful" dedupe that delegates the testkit fork to the core
//! resolver — `fork_is_intentional_testkit_ignores_env_overrides` fails
//! because the testkit path would follow `ASGREP_INDEX_PATH` instead of
//! staying on the fixed default arm. Failure-first: pins current intent,
//! passes now, fails on dedupe.

use ast_sgrep_core::store::{INDEX_DB, INDEX_DIR};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Clear the override env inputs the core resolver consults, returning the
/// previous values for restore. Call only under `env_lock()`.
fn clear_override_env() -> (Option<std::ffi::OsString>, Option<std::ffi::OsString>) {
    let old = (
        std::env::var_os("ASGREP_INDEX_PATH"),
        std::env::var_os("ASGREP_USE_CACHE"),
    );
    std::env::remove_var("ASGREP_INDEX_PATH");
    std::env::remove_var("ASGREP_USE_CACHE");
    old
}

fn restore_override_env(old: (Option<std::ffi::OsString>, Option<std::ffi::OsString>)) {
    match old.0 {
        Some(v) => std::env::set_var("ASGREP_INDEX_PATH", v),
        None => std::env::remove_var("ASGREP_INDEX_PATH"),
    }
    match old.1 {
        Some(v) => std::env::set_var("ASGREP_USE_CACHE", v),
        None => std::env::remove_var("ASGREP_USE_CACHE"),
    }
}

#[test]
fn fork_exists_both_resolvers_agree_on_default_arm() {
    let _guard = env_lock().lock().unwrap();
    let old = clear_override_env();
    // Fresh root: no `.asgrep/index.db` on disk, no override inputs, so the
    // core resolver takes its default arm — the duplicated join.
    let root = tempfile::tempdir().unwrap();
    assert_eq!(
        ast_sgrep_core::index_db_path(root.path(), None),
        ast_sgrep_testkit::index_db_path(root.path()),
    );
    restore_override_env(old);
}

#[test]
fn fork_matches_documented_reason_fixed_default_arm() {
    // Structural pin, no env inputs involved: the testkit fork IS the
    // `<root>/.asgrep/index.db` default arm, spelled with core's constants
    // so a segment rename breaks this rather than drifting silently.
    let root = Path::new("/tmp/asgrep-index-db-path-drift-root");
    assert_eq!(
        ast_sgrep_testkit::index_db_path(root),
        root.join(INDEX_DIR).join(INDEX_DB),
    );
}

#[test]
fn fork_is_intentional_testkit_ignores_env_overrides() {
    let _guard = env_lock().lock().unwrap();
    let old = clear_override_env();
    std::env::set_var(
        "ASGREP_INDEX_PATH",
        "/tmp/asgrep-index-db-path-drift-decoy.db",
    );
    let root = tempfile::tempdir().unwrap();
    // Core honors the override while the hermetic fork stays fixed. A dedupe
    // delegating the testkit fn to the core resolver flips the first assert.
    assert_ne!(
        ast_sgrep_core::index_db_path(root.path(), None),
        ast_sgrep_testkit::index_db_path(root.path()),
    );
    assert_eq!(
        ast_sgrep_testkit::index_db_path(root.path()),
        root.path().join(INDEX_DIR).join(INDEX_DB),
    );
    restore_override_env(old);
}

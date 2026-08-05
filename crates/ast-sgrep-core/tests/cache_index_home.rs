//! Refuse HOME-unset shared /tmp cache fallback (i5ef).
use ast_sgrep_core::store::try_index_db_path;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
fn use_cache_without_home_fails_closed() {
    let _guard = env_lock().lock().unwrap();
    let old_home = std::env::var_os("HOME");
    let old_xdg = std::env::var_os("XDG_CACHE_HOME");
    let old_user = std::env::var_os("USERPROFILE");
    let old_use = std::env::var_os("ASGREP_USE_CACHE");
    std::env::remove_var("HOME");
    std::env::remove_var("XDG_CACHE_HOME");
    std::env::remove_var("USERPROFILE");
    std::env::set_var("ASGREP_USE_CACHE", "1");
    let err = try_index_db_path(Path::new("/tmp/asgrep-i5ef-root"), None).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("HOME") || msg.contains("XDG_CACHE_HOME") || msg.contains("/tmp"),
        "expected fail-closed message, got {msg}"
    );
    // restore
    match old_home {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }
    match old_xdg {
        Some(v) => std::env::set_var("XDG_CACHE_HOME", v),
        None => std::env::remove_var("XDG_CACHE_HOME"),
    }
    match old_user {
        Some(v) => std::env::set_var("USERPROFILE", v),
        None => std::env::remove_var("USERPROFILE"),
    }
    match old_use {
        Some(v) => std::env::set_var("ASGREP_USE_CACHE", v),
        None => std::env::remove_var("ASGREP_USE_CACHE"),
    }
}

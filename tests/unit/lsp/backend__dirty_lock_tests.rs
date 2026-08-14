use super::{resolve_lsp_index_path, resolve_lsp_index_path_with_cache, LspBackend};
use crate::support::AsgrepSettings;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

#[test]
fn dirty_buffers_poison_recovers_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let backend = LspBackend::new(temp.path().to_path_buf());
    let dirty = Arc::clone(&backend.dirty_buffers);
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let _guard = dirty.lock().unwrap();
        panic!("intentional dirty lock poison");
    }));
    assert!(
        backend.dirty_buffers.is_poisoned(),
        "setup: lock should be poisoned"
    );
    backend
        .remember_dirty("src/a.rs", "fn a() {}\n")
        .expect("poison must not permanently brick dirty map");
    assert!(
        !backend.dirty_buffers.is_poisoned(),
        "clear_poison after recover"
    );
    assert_eq!(
        backend.dirty_map().get("src/a.rs").map(String::as_str),
        Some("fn a() {}\n")
    );
}

#[test]
fn relative_index_path_is_allowed_under_workspace_without_opt_in() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().canonicalize().unwrap();
    let path = resolve_lsp_index_path(&root, "state/index.db", false).unwrap();
    assert_eq!(path, root.join("state/index.db"));

    let mut backend = LspBackend::new(root.clone());
    backend
        .apply_settings(AsgrepSettings {
            index_path: Some("state/index.db".into()),
            ..AsgrepSettings::default()
        })
        .expect("relative indexPath under workspace");
    assert_eq!(
        backend.index_path.as_ref(),
        Some(&root.join("state/index.db"))
    );
}

#[test]
fn relative_index_path_escape_is_rejected_without_opt_in() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().canonicalize().unwrap();
    let error = resolve_lsp_index_path(&root, "../escape.db", false)
        .expect_err("parent-dir escape must not write outside the workspace");
    assert!(
        error.to_string().contains("outside the workspace"),
        "{error}"
    );
}

#[test]
fn absolute_index_path_inside_workspace_is_allowed_without_opt_in() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().canonicalize().unwrap();
    let inside = root.join("index.db");
    let path = resolve_lsp_index_path(&root, inside.to_str().unwrap(), false).unwrap();
    assert_eq!(path, inside);
}

#[test]
fn absolute_index_path_outside_workspace_requires_opt_in() {
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = workspace.path().canonicalize().unwrap();
    let escaped = outside.path().join("index.db");

    let error = resolve_lsp_index_path(&root, escaped.to_str().unwrap(), false)
        .expect_err("untrusted absolute path must not plant a DB outside the folder");
    assert!(
        error.to_string().contains("ASGREP_ALLOW_EXTERNAL_INDEX=1"),
        "{error}"
    );

    let allowed = resolve_lsp_index_path(&root, escaped.to_str().unwrap(), true).unwrap();
    let expected = outside
        .path()
        .canonicalize()
        .unwrap()
        .join("index.db");
    assert_eq!(allowed, expected);
}

#[test]
fn absolute_index_path_under_asgrep_cache_is_allowed_without_opt_in() {
    let workspace = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let root = workspace.path().canonicalize().unwrap();
    let cache_home = cache.path().join("asgrep");
    let cached = cache_home.join("abc").join("index.db");
    let path = resolve_lsp_index_path_with_cache(
        &root,
        cached.to_str().unwrap(),
        false,
        Some(cache_home.clone()),
    )
    .unwrap();
    assert_eq!(path, cached);
}

#[test]
fn trusted_relative_index_path_resolves_under_workspace() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().canonicalize().unwrap();
    let path = resolve_lsp_index_path(&root, "state/index.db", true).unwrap();
    assert_eq!(path, root.join("state/index.db"));

    let mut backend = LspBackend::new(root.clone());
    backend.index_path = Some(path);
    assert_eq!(backend.index_options().index_path, backend.index_path);
    assert_eq!(backend.search_options(1).index_path, backend.index_path);
}

#[test]
fn default_index_path_uses_private_cache() {
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().canonicalize().unwrap();
    let backend = LspBackend::new_cached(root.clone()).unwrap();
    let index_path = backend.index_path.expect("private cache path");
    assert!(!index_path.starts_with(root));
    assert!(index_path.ends_with("index.db"));
}

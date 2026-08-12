use super::{resolve_lsp_index_path, LspBackend};
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
fn custom_index_path_requires_explicit_trusted_operator_opt_in() {
    let workspace = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = workspace.path().canonicalize().unwrap();

    let error = resolve_lsp_index_path(&root, "state/index.db", false)
        .expect_err("all custom paths must require explicit opt-in");
    assert!(error.to_string().contains("disabled by default"));

    let allowed = resolve_lsp_index_path(
        &root,
        outside.path().join("index.db").to_str().unwrap(),
        true,
    )
    .unwrap();
    assert_eq!(allowed, outside.path().join("index.db"));
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

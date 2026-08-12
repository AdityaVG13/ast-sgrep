use super::*;
use ast_sgrep_core::force_sidecar_rebuild_err;
use tempfile::TempDir;

#[test]
fn index_repo_invalidates_searcher_on_index_err() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap();
    std::fs::write(root.join("lib.rs"), "fn hello() {}\n").unwrap();
    let mut session = CodeModeSession::new(SessionConfig {
        root: root.clone(),
        index_path: None,
        limit: 8,
        use_embed: false,
        ..SessionConfig::default()
    });
    drop(
        session
            .searcher_for(root.clone(), 8)
            .expect("warm searcher"),
    );
    assert!(
        session.searcher_cache_occupied(),
        "precondition: searcher cache warm"
    );

    let _fail = force_sidecar_rebuild_err();
    let err = session
        .index_repo(&json!({}))
        .expect_err("forced sidecar rebuild must surface as index_repo Err");
    assert!(
        err.to_string().contains("forced sidecar rebuild failure"),
        "unexpected error: {err}"
    );
    assert!(
        !session.searcher_cache_occupied(),
        "searcher cache must clear on index_repo Err after possible disk mutation"
    );
}

#[test]
fn external_writer_generation_invalidates_warm_searcher() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap();
    std::fs::write(root.join("lib.rs"), "fn hello() {}\n").unwrap();
    let session = CodeModeSession::new(SessionConfig {
        root: root.clone(),
        index_path: None,
        limit: 8,
        use_embed: false,
        ..SessionConfig::default()
    });
    drop(
        session
            .searcher_for(root.clone(), 8)
            .expect("warm searcher"),
    );
    assert!(
        session.searcher_cache_occupied(),
        "precondition: searcher cache warm"
    );

    let bumped = ast_sgrep_core::bump_writer_generation(&root, None).unwrap();
    assert!(bumped >= 1);

    drop(
        session
            .searcher_for(root, 8)
            .expect("reopen after stamp bump"),
    );
    let gen = session
        .searcher_cache
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|(k, _)| k.writer_generation));
    assert_eq!(gen, Some(bumped));
}

#[test]
fn nested_root_external_writer_invalidates_warm_searcher() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().canonicalize().unwrap();
    let nested = workspace.join("pkg");
    std::fs::create_dir(&nested).unwrap();
    std::fs::write(nested.join("lib.rs"), "fn hello() {}\n").unwrap();
    let session = CodeModeSession::new(SessionConfig {
        root: workspace.clone(),
        index_path: None,
        limit: 8,
        use_embed: false,
        ..SessionConfig::default()
    });
    drop(
        session
            .searcher_for(nested.clone(), 8)
            .expect("warm searcher on nested root"),
    );
    assert!(
        session.searcher_cache_occupied(),
        "precondition: searcher cache warm"
    );

    let bumped = ast_sgrep_core::bump_writer_generation(&nested, None).unwrap();
    assert_eq!(
        ast_sgrep_core::read_writer_generation(&workspace, None),
        0,
        "workspace stamp must stay untouched"
    );

    drop(
        session
            .searcher_for(nested, 8)
            .expect("reopen after nested stamp bump"),
    );
    let gen = session
        .searcher_cache
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|(k, _)| k.writer_generation));
    assert_eq!(gen, Some(bumped));
}

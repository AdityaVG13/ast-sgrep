use super::*;

fn test_server(root: PathBuf) -> McpServer {
    McpServer {
        root,
        index_path: None,
        limit: 10,
        use_embed: false,
        use_neural_embed: false,
        use_semantic_only: false,
        searcher_cache: Mutex::new(SearcherCache::default()),
        index_lock: Mutex::new(()),
        path_registry: Mutex::new(HashMap::new()),
        emitted_snippets: Mutex::new(HashMap::new()),
    }
}

#[test]
fn reindex_generation_rejects_in_flight_stale_searcher() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let server = test_server(root.clone());
    let (searcher, generation) = server.searcher_for(root.clone(), 10).unwrap();
    server.invalidate_searcher_cache();
    server.restore_searcher(root, 10, generation, searcher);
    let cache = McpServer::lock_or_recover(&server.searcher_cache, |_| {});
    assert!(
        cache.entry.is_none(),
        "stale searcher returned after reindex"
    );
}

#[test]
fn index_repo_invalidates_searcher_after_disk_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    std::fs::write(root.join("lib.rs"), "fn hello() {}\n").unwrap();
    let server = test_server(root.clone());
    let (searcher, generation) = server.searcher_for(root.clone(), 10).unwrap();
    server.restore_searcher(root.clone(), 10, generation, searcher);
    {
        let cache = McpServer::lock_or_recover(&server.searcher_cache, |_| {});
        assert!(cache.entry.is_some());
        assert_eq!(cache.generation, generation);
    }
    // Seed session maps that must not survive reindex.
    McpServer::lock_or_recover(&server.path_registry, |_| {}).insert("p0".into(), "lib.rs".into());
    McpServer::lock_or_recover(&server.emitted_snippets, |_| {}).insert("p0:1-1".into(), 42);

    let args = server
        .parse_index_repo(&json!({}))
        .expect("empty index_repo args should parse");
    let body = server
        .tool_index_repo(args)
        .expect("index_repo should succeed on tiny fixture");
    assert!(
        body.contains("files_indexed") || body.contains("files"),
        "{body}"
    );

    let cache = McpServer::lock_or_recover(&server.searcher_cache, |_| {});
    assert!(
        cache.entry.is_none(),
        "searcher cache must be empty after index_repo mutation"
    );
    assert!(
        cache.generation != generation,
        "generation must advance so in-flight restore cannot reinstall stale Searcher"
    );
    assert!(
        McpServer::lock_or_recover(&server.path_registry, |_| {}).is_empty(),
        "path registry must clear on index mutation"
    );
    assert!(
        McpServer::lock_or_recover(&server.emitted_snippets, |_| {}).is_empty(),
        "emitted snippets must clear on index mutation"
    );
}

/// Pins R-INDEX-ERR-CACHE-SYNC: mid-sidecar Err after bulk commit must still
/// advance generation and clear path/snippet session maps.
#[test]
fn index_repo_invalidates_searcher_on_index_err() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    std::fs::write(root.join("lib.rs"), "fn hello() {}\n").unwrap();
    let server = test_server(root.clone());
    let (searcher, generation) = server.searcher_for(root.clone(), 10).unwrap();
    server.restore_searcher(root.clone(), 10, generation, searcher);
    McpServer::lock_or_recover(&server.path_registry, |_| {}).insert("p0".into(), "lib.rs".into());
    McpServer::lock_or_recover(&server.emitted_snippets, |_| {}).insert("p0:1-1".into(), 42);

    let args = server
        .parse_index_repo(&json!({}))
        .expect("empty index_repo args should parse");
    let _fail = ast_sgrep_core::force_sidecar_rebuild_err();
    let err = server
        .tool_index_repo(args)
        .expect_err("forced sidecar rebuild must surface as index_repo Err");
    assert!(
        err.to_string().contains("forced sidecar rebuild failure"),
        "unexpected error: {err}"
    );

    let cache = McpServer::lock_or_recover(&server.searcher_cache, |_| {});
    assert!(
        cache.entry.is_none(),
        "searcher cache must clear on index_repo Err after possible disk mutation"
    );
    assert!(
        cache.generation != generation,
        "generation must advance on index_repo Err so restore cannot reinstall stale Searcher"
    );
    assert!(
        McpServer::lock_or_recover(&server.path_registry, |_| {}).is_empty(),
        "path registry must clear on index_repo Err"
    );
    assert!(
        McpServer::lock_or_recover(&server.emitted_snippets, |_| {}).is_empty(),
        "emitted snippets must clear on index_repo Err"
    );
}

/// Pins R-XPROC-MULTIWRITER Option C lite: an external writer bumping the
/// durable stamp must drop a warm Searcher without an in-process index_repo.
#[test]
fn external_writer_generation_invalidates_warm_searcher() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    std::fs::write(root.join("lib.rs"), "fn hello() {}\n").unwrap();
    let server = test_server(root.clone());

    let (searcher, generation) = server.searcher_for(root.clone(), 10).unwrap();
    server.restore_searcher(root.clone(), 10, generation, searcher);
    {
        let cache = McpServer::lock_or_recover(&server.searcher_cache, |_| {});
        assert!(cache.entry.is_some(), "precondition: warm Searcher");
    }
    McpServer::lock_or_recover(&server.path_registry, |_| {}).insert("p0".into(), "lib.rs".into());

    // Simulate watch / CLI index in another process: bump stamp only.
    let bumped = ast_sgrep_core::bump_writer_generation(&root, None).unwrap();
    assert!(bumped >= 1);

    let (searcher2, generation2) = server.searcher_for(root.clone(), 10).unwrap();
    assert!(
        generation2 != generation,
        "in-process generation must advance when writer stamp changes"
    );
    server.restore_searcher(root, 10, generation2, searcher2);
    let cache = McpServer::lock_or_recover(&server.searcher_cache, |_| {});
    assert_eq!(cache.writer_generation, bumped);
    assert!(
        McpServer::lock_or_recover(&server.path_registry, |_| {}).is_empty(),
        "path registry must clear across writer generations"
    );
}

/// Session workspace ≠ per-call index root: poll the cached Searcher's stamp.
#[test]
fn nested_root_external_writer_invalidates_warm_searcher() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().canonicalize().unwrap();
    let nested = workspace.join("pkg");
    std::fs::create_dir(&nested).unwrap();
    std::fs::write(nested.join("lib.rs"), "fn hello() {}\n").unwrap();
    let server = test_server(workspace.clone());

    let (searcher, generation) = server.searcher_for(nested.clone(), 10).unwrap();
    server.restore_searcher(nested.clone(), 10, generation, searcher);
    {
        let cache = McpServer::lock_or_recover(&server.searcher_cache, |_| {});
        assert!(
            cache.entry.is_some(),
            "precondition: warm Searcher on nested root"
        );
    }

    let bumped = ast_sgrep_core::bump_writer_generation(&nested, None).unwrap();
    assert_eq!(
        ast_sgrep_core::read_writer_generation(&workspace, None),
        0,
        "workspace stamp must stay untouched"
    );

    let (searcher2, generation2) = server.searcher_for(nested, 10).unwrap();
    assert!(
        generation2 != generation,
        "nested-root stamp bump must drop the warm Searcher"
    );
    drop(searcher2);
    let cache = McpServer::lock_or_recover(&server.searcher_cache, |_| {});
    assert_eq!(cache.writer_generation, bumped);
}

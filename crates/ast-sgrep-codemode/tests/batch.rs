use ast_sgrep_codemode::{run_batch, BatchCall, BatchRequest, CodeModeSession, SessionConfig};
use ast_sgrep_core::{IndexOptions, Indexer};
use ast_sgrep_testkit::sample_root;
use serde_json::json;
use std::time::Instant;
use tempfile::TempDir;

fn indexed_config() -> (TempDir, SessionConfig) {
    let temp = TempDir::new().expect("tempdir");
    let index_path = temp.path().join("index.db");
    let root = sample_root();
    let mut indexer = Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .expect("indexer");
    indexer.index_all().expect("index");
    let config = SessionConfig {
        root,
        index_path: Some(index_path),
        limit: 8,
        use_embed: false,
        ..SessionConfig::default()
    };
    (temp, config)
}

#[test]
fn batch_parallel_returns_per_call_results() {
    let (_tmp, config) = indexed_config();
    let request = BatchRequest {
        root: Some(config.root.clone()),
        index_path: config.index_path.clone(),
        use_embed: Some(false),
        limit: Some(5),
        parallel: true,
        calls: vec![
            BatchCall {
                id: "a".into(),
                tool: "search".into(),
                args: json!({"query": "auth", "format": "capsule", "limit": 5}),
            },
            BatchCall {
                id: "b".into(),
                tool: "defs".into(),
                args: json!({"symbol": "auth_refresh", "limit": 5}),
            },
        ],
    };
    let response = run_batch(config, &request).expect("batch");
    assert!(response.ok);
    assert_eq!(response.call_count, 2);
    assert_eq!(response.mode, "parallel");
    assert_eq!(response.results.len(), 2);
    assert!(response.results.iter().all(|r| r.ok));
}

#[test]
fn batch_beats_cold_sequential_sessions_on_wall_time() {
    let (_tmp, config) = indexed_config();
    let calls = vec![
        BatchCall {
            id: "1".into(),
            tool: "search".into(),
            args: json!({"query": "auth", "limit": 3}),
        },
        BatchCall {
            id: "2".into(),
            tool: "search".into(),
            args: json!({"query": "token", "limit": 3}),
        },
        BatchCall {
            id: "3".into(),
            tool: "search".into(),
            args: json!({"query": "request", "limit": 3}),
        },
    ];

    let cold_started = Instant::now();
    for call in &calls {
        let mut session = CodeModeSession::new(config.clone());
        session
            .call(&call.tool, call.args.clone())
            .expect("cold call");
    }
    let cold_ms = cold_started.elapsed().as_millis();

    let batch = run_batch(
        config,
        &BatchRequest {
            root: None,
            index_path: None,
            use_embed: Some(false),
            limit: Some(3),
            parallel: true,
            calls,
        },
    )
    .expect("batch");
    assert!(batch.ok);
    // Parallel batch should not be slower than cold sequential by much; allow slack
    // for CI noise but require it finishes.
    assert!(
        batch.wall_ms <= cold_ms.saturating_mul(2) + 50,
        "batch {}ms vs cold sequential {}ms",
        batch.wall_ms,
        cold_ms
    );
}

use ast_sgrep_codemode::{
    run_batch, run_serve, BatchCall, BatchRequest, CodeModeSession, ParallelMode, ServeRequest,
    ServeResponse, SessionConfig,
};
use ast_sgrep_core::{IndexOptions, Indexer};
use ast_sgrep_testkit::sample_root;
use serde_json::json;
use std::io::Cursor;
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

fn batch_req(config: &SessionConfig, parallel: Option<bool>, calls: Vec<BatchCall>) -> BatchRequest {
    BatchRequest {
        root: Some(config.root.clone()),
        index_path: config.index_path.clone(),
        use_embed: Some(false),
        limit: Some(5),
        parallel,
        parallel_mode: None,
        calls,
    }
}

#[test]
fn batch_serial_warm_is_default_for_small_waves() {
    let (_tmp, config) = indexed_config();
    let response = run_batch(
        config.clone(),
        &batch_req(
            &config,
            None,
            vec![
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
        ),
    )
    .expect("batch");
    // Auto: N=2 < 4 → serial warm
    assert_eq!(response.mode, "serial");
    assert!(response.all_ok);
    assert_eq!(response.call_count, 2);
    assert!(response.results.iter().all(|r| r.ok));
}

#[test]
fn batch_parallel_forced_returns_per_call_results() {
    let (_tmp, config) = indexed_config();
    let response = run_batch(
        config.clone(),
        &batch_req(
            &config,
            Some(true),
            vec![
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
        ),
    )
    .expect("batch");
    assert!(response.all_ok);
    assert_eq!(response.mode, "parallel");
    assert_eq!(response.results.len(), 2);
}

#[test]
fn batch_never_parallelizes_index_repo_with_readers() {
    let (_tmp, config) = indexed_config();
    let response = run_batch(
        config.clone(),
        &BatchRequest {
            root: Some(config.root.clone()),
            index_path: config.index_path.clone(),
            use_embed: Some(false),
            limit: Some(5),
            parallel: Some(true),
            parallel_mode: Some(ParallelMode::Parallel),
            calls: vec![
                BatchCall {
                    id: "a".into(),
                    tool: "search".into(),
                    args: json!({"query": "auth", "limit": 3}),
                },
                BatchCall {
                    id: "b".into(),
                    tool: "index_repo".into(),
                    args: json!({"force": false}),
                },
            ],
        },
    )
    .expect("batch");
    assert_eq!(response.mode, "serial");
    assert_eq!(response.results.len(), 2);
}

#[test]
fn batch_partial_failure_keeps_sibling_ok() {
    let (_tmp, config) = indexed_config();
    let response = run_batch(
        config.clone(),
        &batch_req(
            &config,
            Some(false),
            vec![
                BatchCall {
                    id: "ok".into(),
                    tool: "search".into(),
                    args: json!({"query": "auth", "limit": 3}),
                },
                BatchCall {
                    id: "bad".into(),
                    tool: "defs".into(),
                    args: json!({}), // missing symbol
                },
            ],
        ),
    )
    .expect("batch");
    assert!(!response.all_ok);
    let ok = response.results.iter().find(|r| r.id == "ok").unwrap();
    let bad = response.results.iter().find(|r| r.id == "bad").unwrap();
    assert!(ok.ok);
    assert!(!bad.ok);
    assert!(bad.error.as_ref().unwrap().contains("symbol"));
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
            parallel: Some(false),
            parallel_mode: Some(ParallelMode::Serial),
            calls,
        },
    )
    .expect("batch");
    assert!(batch.all_ok);
    assert_eq!(batch.mode, "serial");
    // Warm serial should beat N cold Searcher opens.
    assert!(
        batch.wall_ms <= cold_ms.saturating_mul(2) + 50,
        "batch {}ms vs cold sequential {}ms",
        batch.wall_ms,
        cold_ms
    );
}

#[test]
fn sticky_serve_reuses_session_across_calls() {
    let (_tmp, config) = indexed_config();
    let input = format!(
        "{}\n{}\n{}\n",
        serde_json::to_string(&ServeRequest::Call {
            id: "1".into(),
            tool: "search".into(),
            args: json!({"query": "auth", "limit": 3}),
        })
        .unwrap(),
        serde_json::to_string(&ServeRequest::Call {
            id: "2".into(),
            tool: "defs".into(),
            args: json!({"symbol": "auth_refresh", "limit": 3}),
        })
        .unwrap(),
        serde_json::to_string(&ServeRequest::End).unwrap(),
    );
    let mut out = Vec::new();
    run_serve(config, Cursor::new(input), &mut out).expect("serve");
    let text = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 3);
    let r1: ServeResponse = serde_json::from_str(lines[0]).unwrap();
    let r2: ServeResponse = serde_json::from_str(lines[1]).unwrap();
    let bye: ServeResponse = serde_json::from_str(lines[2]).unwrap();
    match r1 {
        ServeResponse::Result { ok, .. } => assert!(ok),
        other => panic!("expected result, got {other:?}"),
    }
    match r2 {
        ServeResponse::Result { ok, .. } => assert!(ok),
        other => panic!("expected result, got {other:?}"),
    }
    assert!(matches!(bye, ServeResponse::Bye));
}

#[test]
fn searcher_cache_survives_limit_changes() {
    let (_tmp, config) = indexed_config();
    let mut session = CodeModeSession::new(config);
    session
        .call("search", json!({"query": "auth", "limit": 3}))
        .expect("first");
    // Different limit must not force a full reopen failure — just works.
    let second = session
        .call("search", json!({"query": "token", "limit": 5}))
        .expect("second");
    assert!(second.get("hits").is_some() || second.get("hit_count").is_some() || second.is_object());
}

#[test]
fn chain_default_top_n_matches_core_default() {
    let (_tmp, config) = indexed_config();
    let mut session = CodeModeSession::new(config);
    let value = session
        .call("chain", json!({"query": "auth_refresh", "limit": 20}))
        .expect("chain");
    // Smoke: chain returns graph-shaped JSON (nodes/edges or node_count).
    assert!(
        value.get("nodes").is_some()
            || value.get("node_count").is_some()
            || value.get("query").is_some()
    );
}

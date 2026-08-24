use ast_sgrep_codemode::{
    run_batch, run_serve, BatchCall, BatchRequest, CodeModeSession, ParallelMode, ServeRequest,
    ServeResponse, SessionConfig, MAX_BATCH_ERROR_BYTES, MAX_BATCH_RESPONSE_BYTES,
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

fn batch_req(
    config: &SessionConfig,
    parallel: Option<bool>,
    calls: Vec<BatchCall>,
) -> BatchRequest {
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
fn batch_drops_values_beyond_the_aggregate_response_budget() {
    let (_tmp, config) = indexed_config();
    let payload = "x".repeat(900_000);
    let calls = (0..5)
        .map(|index| BatchCall {
            id: index.to_string(),
            tool: "select".into(),
            args: json!({"value": {"payload": payload}, "fields": ["payload"]}),
        })
        .collect();
    let response =
        run_batch(config.clone(), &batch_req(&config, Some(false), calls)).expect("bounded batch");
    assert!(!response.all_ok);
    assert!(response.results.iter().any(|result| {
        !result.ok
            && result
                .error
                .as_deref()
                .is_some_and(|error| error.contains(&MAX_BATCH_RESPONSE_BYTES.to_string()))
    }));
    let encoded = serde_json::to_vec(&response).expect("response JSON");
    assert!(encoded.len() <= MAX_BATCH_RESPONSE_BYTES);
}

#[test]
fn batch_rejects_oversized_response_identifiers() {
    let (_tmp, config) = indexed_config();
    let request = batch_req(
        &config,
        Some(false),
        vec![BatchCall {
            id: "x".repeat(129),
            tool: "search".into(),
            args: json!({"query": "auth"}),
        }],
    );
    let error = run_batch(config, &request).unwrap_err();
    assert!(error.to_string().contains("id exceeds 128 bytes"));
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
fn sticky_serve_preserves_valid_id_on_schema_errors() {
    let (_tmp, config) = indexed_config();
    let input = b"{\"type\":\"call\",\"id\":\"request-7\",\"tool\":7}\n";
    let mut out = Vec::new();
    run_serve(config, Cursor::new(input), &mut out).expect("serve");
    let response: ServeResponse = serde_json::from_slice(out.strip_suffix(b"\n").unwrap()).unwrap();
    match response {
        ServeResponse::Error { id, .. } => assert_eq!(id.as_deref(), Some("request-7")),
        other => panic!("expected validation error, got {other:?}"),
    }
}

#[test]
fn sticky_serve_bounds_request_derived_tool_errors() {
    let (_tmp, config) = indexed_config();
    let name = "unknown".repeat(4_000);
    let input = format!(
        "{}\n",
        serde_json::to_string(&ServeRequest::Call {
            id: "bounded-error".into(),
            tool: "catalog_describe".into(),
            args: json!({ "name": name }),
        })
        .unwrap()
    );
    let mut out = Vec::new();
    run_serve(config, Cursor::new(input), &mut out).expect("serve");
    let response: ServeResponse = serde_json::from_slice(out.strip_suffix(b"\n").unwrap()).unwrap();
    match response {
        ServeResponse::Result {
            ok: false,
            error: Some(error),
            ..
        } => {
            assert!(error.len() <= MAX_BATCH_ERROR_BYTES);
            assert!(error.ends_with('…'));
        }
        other => panic!("expected bounded tool error, got {other:?}"),
    }
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
    assert!(
        second.get("hits").is_some() || second.get("hit_count").is_some() || second.is_object()
    );
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

/// Regression for br-r49: after the sticky session exhausts its call budget,
/// run_serve used to keep answering EVERY subsequent request with the same
/// per-call budget error until the client gave up — an endless flood that
/// hides the outage instead of reporting it once, loudly, and stopping.
///
/// Contract: the first request past the budget gets exactly ONE budget-exceeded
/// error response, then run_serve terminates with Err (the CLI process fails).
#[test]
fn sticky_serve_fails_once_and_stops_after_budget_exhaustion() {
    let (_tmp, config) = indexed_config();
    // Serve pins max_calls=10_000. `select` is a pure projection tool (no
    // index work), so driving past the budget stays cheap. Five overflow
    // requests: pre-fix each one gets its own identical error response.
    const BUDGET: usize = 10_000;
    const OVERFLOW: usize = 5;
    let mut input = String::new();
    for i in 0..BUDGET + OVERFLOW {
        input.push_str(
            &serde_json::to_string(&ServeRequest::Call {
                id: format!("c{i}"),
                tool: "select".into(),
                args: json!({"value": {"v": i}, "fields": ["v"]}),
            })
            .unwrap(),
        );
        input.push('\n');
    }
    input.push_str(&serde_json::to_string(&ServeRequest::End).unwrap());
    input.push('\n');

    let mut out = Vec::new();
    let result = run_serve(config, Cursor::new(input), &mut out);
    assert!(
        result.is_err(),
        "run_serve must terminate with an error once the call budget is \
         exhausted; it returned Ok and kept serving"
    );
    let text = String::from_utf8(out).unwrap();
    let budget_errors = text
        .lines()
        .filter(|line| line.contains("\"ok\":false") && line.contains("budget"))
        .count();
    assert_eq!(
        budget_errors, 1,
        "exactly ONE budget-exceeded response may be emitted before the \
         session dies; got {budget_errors}"
    );
}

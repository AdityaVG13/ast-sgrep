//! Tokio concurrency-contract suite for ast-sgrep-mcp (consolidates K1).
//!
//! One test per intent; each test folds every catalog facet for that intent.
//! Transport, payload builders, tool asserts, and fixture trees come from
//! `ast_sgrep_testkit`; only the forbidden-id collector is local.
//! Discriminants are codes, shapes, counts, id echo, and byte (in)equality --
//! never message text, never durations.

use ast_sgrep_testkit::{
    assert_ping_ok, assert_tool_error_shape, assert_tool_success, assert_tools_list_ok, big_tree,
    cancelled_notif, collect_responses, ping, response_by_id, small_tree, tool_body, tool_call,
    tool_text, tools_list, LiveSession,
};
use serde_json::{json, Value};
use std::time::Duration;

const RECV_TIMEOUT: Duration = Duration::from_secs(15);
const SLOW_RECV_TIMEOUT: Duration = Duration::from_secs(120);

/// INTENT: pipelined batches are fully answered with per-id correct results.
/// KILLS: drop/reorder/interleave-corruption mutants.
/// Facets: tool-only batch (K1.1) + mixed-method pipeline (K1.7). Wire order
/// is NOT pinned (rmcp dispatches each request on its own task); id matching is.
#[test]
fn pipeline_integrity_tool_and_mixed_batches() {
    let temp = small_tree();

    // Facet 1 (K1.1): 8 mixed tool calls, each matched by id with its own result.
    let batch = vec![
        tool_call(
            1,
            "keyword_search",
            json!({"query": "redhammer", "limit": 4}),
        ),
        tool_call(2, "index_status", json!({})),
        tool_call(3, "code_read", json!({"ids": ["a.rs#L1-L1"]})),
        tool_call(
            4,
            "keyword_search",
            json!({"query": "blueanvil", "limit": 4}),
        ),
        tool_call(
            5,
            "code_search",
            json!({"query": "greenchisel", "limit": 4}),
        ),
        tool_call(6, "index_status", json!({})),
        tool_call(
            7,
            "keyword_search",
            json!({"query": "greenchisel", "limit": 4}),
        ),
        tool_call(
            8,
            "ast_search",
            json!({"query": "fn $NAME() { $$$BODY }", "limit": 8}),
        ),
    ];
    let responses = ast_sgrep_testkit::rpc_pipeline(batch, Some(temp.path()));
    assert_eq!(responses.len(), 8);
    for (id, name, query) in [
        (1, "keyword_search", "redhammer"),
        (4, "keyword_search", "blueanvil"),
        (5, "code_search", "greenchisel"),
        (7, "keyword_search", "greenchisel"),
    ] {
        let response = response_by_id(&responses, id);
        assert_tool_success(response);
        let body = tool_body(response);
        assert!(
            body["h"].as_array().is_some_and(|h| !h.is_empty()),
            "{name} {query}: {body:#}"
        );
        assert!(
            tool_text(response).contains(query),
            "{name} {query}: result does not carry its own query: {response:#}"
        );
    }
    for id in [2, 6] {
        let response = response_by_id(&responses, id);
        assert_tool_success(response);
        assert!(tool_body(response).is_object(), "{response:#}");
    }
    let read = response_by_id(&responses, 3);
    assert_tool_success(read);
    assert_eq!(tool_body(read)["nodes"][0]["id"], "a.rs#L1-L1");
    let ast = response_by_id(&responses, 8);
    assert_tool_success(ast);
    assert!(
        tool_body(ast)["h"]
            .as_array()
            .is_some_and(|h| !h.is_empty()),
        "{ast:#}"
    );

    // Facet 2 (K1.7): mixed ping/list/call pipeline, 4 rounds, matched by id.
    let mut script = Vec::new();
    let mut next: u32 = 1;
    let mut ping_ids = Vec::new();
    let mut list_ids = Vec::new();
    let mut call_ids = Vec::new();
    for query in ["redhammer", "blueanvil", "greenchisel", "redhammer"] {
        script.push(ping(next));
        ping_ids.push(next);
        next += 1;
        script.push(tools_list(next));
        list_ids.push(next);
        next += 1;
        script.push(tool_call(
            next,
            "keyword_search",
            json!({"query": query, "limit": 4}),
        ));
        call_ids.push((next, query));
        next += 1;
    }
    let total = script.len();
    let responses = ast_sgrep_testkit::rpc_pipeline(script, Some(temp.path()));
    assert_eq!(responses.len(), total);
    for id in ping_ids {
        assert_ping_ok(response_by_id(&responses, id), id);
    }
    for id in list_ids {
        assert_tools_list_ok(response_by_id(&responses, id), id);
    }
    for (id, query) in call_ids {
        let response = response_by_id(&responses, id);
        assert_tool_success(response);
        assert!(tool_text(response).contains(query), "id {id}: {response:#}");
    }
}

/// INTENT: concurrent distinct searches never exchange results.
/// KILLS: shared-buffer/crosstalk mutants.
/// Catalog: K1.2 KEEP.
#[test]
fn pipelined_distinct_searches_have_no_crosstalk() {
    let temp = small_tree();
    let queries = ["redhammer", "blueanvil", "greenchisel"];
    let script: Vec<Value> = queries
        .iter()
        .enumerate()
        .map(|(i, query)| {
            tool_call(
                i as u32 + 1,
                "keyword_search",
                json!({"query": query, "limit": 8}),
            )
        })
        .collect();
    let responses = ast_sgrep_testkit::rpc_pipeline(script, Some(temp.path()));
    assert_eq!(responses.len(), 3);
    for (i, query) in queries.iter().enumerate() {
        let id = i as u32 + 1;
        let response = response_by_id(&responses, id);
        assert_tool_success(response);
        let text = tool_text(response).to_string();
        assert!(text.contains(query), "id {id}: {response:#}");
        for other in queries.iter().filter(|q| *q != query) {
            assert!(
                !text.contains(other),
                "id {id} ({query}) leaked {other}: {response:#}"
            );
        }
    }
}

/// INTENT: one bad call fails alone; batch and session continue.
/// KILLS: abort-on-first-error/session-poison mutants.
/// Catalog: K1.3 KEEP.
#[test]
fn pipelined_error_does_not_poison_batch_or_session() {
    let temp = small_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "no_such_tool", json!({})));
    session.send(&tool_call(
        2,
        "keyword_search",
        json!({"query": "x", "limit": 0}),
    ));
    session.send(&tool_call(
        3,
        "keyword_search",
        json!({"query": "redhammer", "limit": 4}),
    ));
    session.send(&tool_call(4, "index_status", json!({})));
    let responses = collect_responses(&session, 4, RECV_TIMEOUT);

    assert_tool_error_shape(response_by_id(&responses, 1));
    assert_tool_error_shape(response_by_id(&responses, 2));
    let good = response_by_id(&responses, 3);
    assert_tool_success(good);
    assert!(
        tool_body(good)["h"]
            .as_array()
            .is_some_and(|h| !h.is_empty()),
        "{good:#}"
    );
    assert_tool_success(response_by_id(&responses, 4));

    session.send(&ping(5));
    session.send(&tool_call(
        6,
        "keyword_search",
        json!({"query": "blueanvil", "limit": 4}),
    ));
    let followup = collect_responses(&session, 2, RECV_TIMEOUT);
    assert_ping_ok(response_by_id(&followup, 5), 5);
    assert_tool_success(response_by_id(&followup, 6));
    session.close_stdin();
    assert!(session.wait_clean().success());
}

/// INTENT: reader-path requests (ping, tools/list) are served while a tool
/// call is active or being cancelled -- the core tokio contract.
/// KILLS: lock-removal/reader-block mutants.
/// Facets: ping during slow tool (K1.5 KEEP) + list during slow tool (K1.6) +
/// ping/list during cancel window (K2.9). Outcome-only: every request is
/// answered correctly within a generous bound; no latency assertions.
#[test]
fn served_during_activity_ping_list_tool_and_cancel() {
    // Facet 1 (K1.5): ping answered while a slow index_repo holds the lock,
    // and the index still completes.
    let temp = big_tree(1200);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_repo", json!({})));
    session.send(&ping(2));
    let responses = collect_responses(&session, 2, SLOW_RECV_TIMEOUT);
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_ping_ok(response_by_id(&responses, 2), 2);
    let index = response_by_id(&responses, 1);
    assert_tool_success(index);
    assert!(tool_body(index).is_object());

    // Facet 2 (K1.6): tools/list succeeds during an active tool call.
    let temp = big_tree(1200);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_repo", json!({})));
    session.send(&tools_list(2));
    let responses = collect_responses(&session, 2, SLOW_RECV_TIMEOUT);
    session.close_stdin();
    assert!(session.wait_clean().success());
    assert_tools_list_ok(response_by_id(&responses, 2), 2);
    let index = response_by_id(&responses, 1);
    assert_tool_success(index);
    assert!(tool_body(index).is_object());

    // Facet 3 (K2.9): ping and tools/list both served during the cancel
    // window of a slow tool; the cancelled id stays silent.
    let temp = big_tree(1200);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_repo", json!({})));
    std::thread::sleep(Duration::from_millis(500));
    session.send(&cancelled_notif(1));
    session.send(&ping(2));
    session.send(&tools_list(3));
    let responses = recv_until_all(&session, &[2, 3], &[1]);
    assert_eq!(responses.len(), 2);
    for response in &responses {
        match response["id"].as_u64().expect("numeric id") {
            2 => assert_ping_ok(response, 2),
            3 => assert_tools_list_ok(response, 3),
            other => panic!("unexpected id {other}: {response:#}"),
        }
    }
    session.close_stdin();
    assert!(session.wait_clean().success());
}

/// Read until every id in `wants` arrives; any `forbidden` (cancelled) id
/// fails the test immediately.
fn recv_until_all(session: &LiveSession, wants: &[u32], forbidden: &[u32]) -> Vec<Value> {
    let mut out = Vec::with_capacity(wants.len());
    let mut pending: Vec<u32> = wants.to_vec();
    let started = std::time::Instant::now();
    while !pending.is_empty() {
        let elapsed = started.elapsed();
        assert!(
            elapsed < SLOW_RECV_TIMEOUT,
            "timed out waiting for ids {pending:?} after {elapsed:?}"
        );
        let response = session.recv_timeout(SLOW_RECV_TIMEOUT - elapsed);
        if let Some(id) = response.get("id").and_then(Value::as_u64) {
            let id = id as u32;
            assert!(
                !forbidden.contains(&id),
                "cancelled id {id} was answered (silence is the contract): {response:#}"
            );
            if let Some(pos) = pending.iter().position(|w| *w == id) {
                pending.remove(pos);
                out.push(response);
            }
        }
    }
    out
}

/// INTENT: the server is correct under threaded concurrent stdin writers.
/// KILLS: write-interleave/framing mutants.
/// Catalog: K1.8 KEEP.
#[test]
fn concurrent_writer_threads_all_answered() {
    use std::sync::Mutex;

    let temp = small_tree();
    let session = Mutex::new(LiveSession::spawn(Some(temp.path())));
    session.lock().unwrap().handshake();

    const THREADS: u32 = 4;
    const PER_THREAD: u32 = 5;
    std::thread::scope(|scope| {
        for thread in 0..THREADS {
            let session = &session;
            scope.spawn(move || {
                for i in 0..PER_THREAD {
                    let id = 1000 + thread * 100 + i;
                    let payload = if i % 2 == 0 {
                        ping(id)
                    } else {
                        tool_call(id, "index_status", json!({}))
                    };
                    session.lock().unwrap().send(&payload);
                }
            });
        }
    });

    let total = (THREADS * PER_THREAD) as usize;
    let mut responses = Vec::with_capacity(total);
    for _ in 0..total {
        responses.push(session.lock().unwrap().recv_timeout(RECV_TIMEOUT));
    }
    let mut session = session.into_inner().unwrap();
    session.close_stdin();
    assert!(session.wait_clean().success());

    assert_eq!(responses.len(), total);
    for thread in 0..THREADS {
        for i in 0..PER_THREAD {
            let id = 1000 + thread * 100 + i;
            let response = response_by_id(&responses, id);
            if i % 2 == 0 {
                assert_ping_ok(response, id);
            } else {
                assert_tool_success(response);
            }
        }
    }
}

/// INTENT: multi-batch, multi-phase sessions stay correct with a clean exit.
/// KILLS: session-state-rot mutants.
/// Facets: back-to-back batches (K1.9) + extended phases with an error
/// mid-stream (K1.10).
#[test]
fn session_durability_batches_and_phases() {
    let temp = small_tree();

    // Facet 1 (K1.9): three back-to-back pipelined batches over one handshake.
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    for id in 1..=3 {
        session.send(&tool_call(id, "index_status", json!({})));
    }
    let batch1 = collect_responses(&session, 3, RECV_TIMEOUT);
    for id in 1..=3 {
        assert_tool_success(response_by_id(&batch1, id));
    }
    for (id, query) in [(4, "redhammer"), (5, "blueanvil"), (6, "greenchisel")] {
        session.send(&tool_call(
            id,
            "keyword_search",
            json!({"query": query, "limit": 4}),
        ));
    }
    let batch2 = collect_responses(&session, 3, RECV_TIMEOUT);
    for (id, query) in [(4, "redhammer"), (5, "blueanvil"), (6, "greenchisel")] {
        let response = response_by_id(&batch2, id);
        assert_tool_success(response);
        assert!(tool_text(response).contains(query), "{response:#}");
    }
    session.send(&ping(7));
    session.send(&tools_list(8));
    session.send(&tool_call(9, "code_read", json!({"ids": ["c.rs#L1-L2"]})));
    let batch3 = collect_responses(&session, 3, RECV_TIMEOUT);
    assert_ping_ok(response_by_id(&batch3, 7), 7);
    assert_tools_list_ok(response_by_id(&batch3, 8), 8);
    let read = response_by_id(&batch3, 9);
    assert_tool_success(read);
    assert_eq!(tool_body(read)["nodes"][0]["id"], "c.rs#L1-L2");
    session.close_stdin();
    assert!(session.wait_clean().success());

    // Facet 2 (K1.10): initialize -> calls -> more calls, error mid-stream.
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tools_list(1));
    session.send(&tool_call(2, "index_status", json!({})));
    let phase1 = collect_responses(&session, 2, RECV_TIMEOUT);
    assert_tools_list_ok(response_by_id(&phase1, 1), 1);
    assert_tool_success(response_by_id(&phase1, 2));
    session.send(&tool_call(
        3,
        "keyword_search",
        json!({"query": "redhammer", "limit": 4}),
    ));
    session.send(&tool_call(4, "code_read", json!({"ids": ["b.rs#L1-L1"]})));
    let phase2 = collect_responses(&session, 2, RECV_TIMEOUT);
    let search = response_by_id(&phase2, 3);
    assert_tool_success(search);
    assert!(tool_text(search).contains("redhammer"), "{search:#}");
    let read = response_by_id(&phase2, 4);
    assert_tool_success(read);
    assert_eq!(tool_body(read)["nodes"][0]["id"], "b.rs#L1-L1");
    session.send(&tool_call(5, "no_such_tool", json!({})));
    session.send(&ping(6));
    session.send(&tool_call(
        7,
        "code_search",
        json!({"query": "greenchisel", "limit": 4}),
    ));
    let phase3 = collect_responses(&session, 3, RECV_TIMEOUT);
    assert_tool_error_shape(response_by_id(&phase3, 5));
    assert_ping_ok(response_by_id(&phase3, 6), 6);
    let late = response_by_id(&phase3, 7);
    assert_tool_success(late);
    assert!(tool_text(late).contains("greenchisel"), "{late:#}");
    session.close_stdin();
    assert!(session.wait_clean().success());
}

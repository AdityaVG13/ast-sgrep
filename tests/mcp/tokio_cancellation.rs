//! Tokio cancellation/EOF-contract suite for ast-sgrep-mcp (consolidates K2).
//!
//! One test per intent; each test folds every catalog facet for that intent.
//! Transport, payload builders, tool asserts, and fixture trees come from
//! `ast_sgrep_testkit`; only the silence probes are local. Discriminants are
//! exit codes, response shapes, id echo, and presence/absence of responses --
//! never message text, never durations.

use ast_sgrep_testkit::{
    assert_ping_ok, assert_tool_success, big_tree, cancelled_notif, ping, small_tree, tool_body,
    tool_call, tool_text, tools_list, LiveSession,
};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const SLOW_RECV_TIMEOUT: Duration = Duration::from_secs(120);
/// Quiet window asserting a cancelled in-flight id is never answered. Must
/// exceed the remaining uncancelled index time (~5s for [`BIG_FILES`]) so a
/// missed cancel still lands inside the window and fails the test.
const CANCEL_QUIET_WINDOW: Duration = Duration::from_secs(20);
/// Quiet window for a cancelled queued call: had the cancel missed, the fast
/// queued tool would answer within milliseconds of the slow tool finishing.
const QUEUED_QUIET_WINDOW: Duration = Duration::from_secs(5);
/// Quiet window for stray cancels: a (buggy) reply to one would be instant,
/// since no work backs it.
const STRAY_QUIET_WINDOW: Duration = Duration::from_secs(1);
/// Delay before sending `cancelled` so the slow tool is genuinely mid-call.
const CANCEL_DELAY: Duration = Duration::from_millis(500);
/// Tree width whose uncancelled `index_repo` takes ~5s in debug, giving the
/// 500ms cancel a wide mid-call margin on fast machines.
const BIG_FILES: usize = 2500;
/// Smaller tree for abort drills: the abort must land mid-call, and the
/// post-EOF wind-down must fit the harness wait bound.
const ABORT_FILES: usize = 1200;

/// `None` on timeout only; a closed stdout while the session must be live is
/// re-raised as a failure, not a quiet window. (Testkit `recv_timeout` panics
/// on timeout; the panic message discriminates the two cases.)
fn try_recv(session: &LiveSession, timeout: Duration) -> Option<Value> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        session.recv_timeout(timeout)
    })) {
        Ok(value) => Some(value),
        Err(payload) => {
            let timed_out = payload
                .downcast_ref::<String>()
                .is_some_and(|msg| msg.contains("timed out"))
                || payload
                    .downcast_ref::<&str>()
                    .is_some_and(|msg| msg.contains("timed out"));
            if timed_out {
                None
            } else {
                std::panic::resume_unwind(payload)
            }
        }
    }
}

/// Read until the response with `want` arrives; any response bearing a
/// `forbidden` (cancelled) id fails the test immediately.
fn recv_until(session: &LiveSession, want: u32, forbidden: &[u32]) -> Value {
    let started = Instant::now();
    loop {
        let elapsed = started.elapsed();
        assert!(
            elapsed < SLOW_RECV_TIMEOUT,
            "timed out waiting for id {want} after {elapsed:?}"
        );
        let response = session.recv_timeout(SLOW_RECV_TIMEOUT - elapsed);
        if let Some(id) = response.get("id").and_then(Value::as_u64) {
            assert!(
                !forbidden.contains(&(id as u32)),
                "cancelled id {id} was answered (silence is the contract): {response:#}"
            );
            if id as u32 == want {
                return response;
            }
        }
    }
}

/// Read until every id in `wants` arrives; any `forbidden` id fails the test.
fn recv_until_all(session: &LiveSession, wants: &[u32], forbidden: &[u32]) -> Vec<Value> {
    let mut out = Vec::with_capacity(wants.len());
    let mut pending: Vec<u32> = wants.to_vec();
    let started = Instant::now();
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

/// Assert no response bearing any `forbidden` id arrives within `window`.
/// Anything else arriving is ignored: only the cancelled ids are the
/// contract under test.
fn assert_no_id_for(session: &LiveSession, forbidden: &[u32], window: Duration) {
    let started = Instant::now();
    while started.elapsed() < window {
        let remaining = window - started.elapsed();
        match try_recv(session, remaining) {
            None => return,
            Some(response) => {
                if let Some(id) = response.get("id").and_then(Value::as_u64) {
                    assert!(
                        !forbidden.contains(&(id as u32)),
                        "cancelled id {id} answered {} after cancel (silence is the contract): {response:#}",
                        started.elapsed().as_secs_f32()
                    );
                }
            }
        }
    }
}

/// Assert the server emits nothing at all within `window`.
fn assert_silent(session: &LiveSession, window: Duration) {
    assert_eq!(
        try_recv(session, window),
        None,
        "server emitted output for a stray cancel"
    );
}

/// INTENT: a cancelled call is never answered -- in-flight, queued, or one of
/// several -- while unaffected work completes normally.
/// KILLS: cancel-ignored / cancel-all / cancel-wrong-id mutants.
/// Facets: in-flight suppression + ping in the same window (K2.5 KEEP) +
/// queued-call suppression (K2.6) + selective cancel with sibling intact
/// (K2.7). Outcome-only: silence of the cancelled ids; no latency assertions.
#[test]
fn cancel_suppression_inflight_queued_and_selective() {
    // Facet 1 (K2.5): cancelling an in-flight index_repo suppresses that
    // call's response entirely, while a ping in the same window is answered
    // and the session stays usable.
    let temp = big_tree(BIG_FILES);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_repo", json!({})));
    std::thread::sleep(CANCEL_DELAY);
    session.send(&cancelled_notif(1));
    session.send(&ping(2));
    let ping_response = recv_until(&session, 2, &[1]);
    assert_ping_ok(&ping_response, 2);
    // The cancelled id must stay silent well past the point where an
    // uncancelled index would have answered.
    assert_no_id_for(&session, &[1], CANCEL_QUIET_WINDOW);
    session.send(&tool_call(3, "index_status", json!({})));
    let status = recv_until(&session, 3, &[1]);
    assert_tool_success(&status);
    assert!(tool_body(&status).is_object(), "{status:#}");
    session.close_stdin();
    assert!(session.wait_clean().success());

    // Facet 2 (K2.6): a call cancelled while queued on the tool lock is never
    // answered, and the slow tool ahead of it still succeeds.
    let temp = big_tree(BIG_FILES);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_repo", json!({})));
    std::thread::sleep(CANCEL_DELAY);
    session.send(&tool_call(2, "index_status", json!({})));
    std::thread::sleep(CANCEL_DELAY);
    session.send(&cancelled_notif(2));
    let index = recv_until(&session, 1, &[2]);
    assert_tool_success(&index);
    assert!(tool_body(&index).is_object(), "{index:#}");
    // Had the cancel missed, the fast queued call would answer within
    // milliseconds of the index finishing.
    assert_no_id_for(&session, &[2], QUEUED_QUIET_WINDOW);
    session.send(&ping(3));
    assert_ping_ok(&recv_until(&session, 3, &[2]), 3);
    session.close_stdin();
    assert!(session.wait_clean().success());

    // Facet 3 (K2.7): cancelling one queued call leaves a sibling queued call
    // intact: the slow tool and the sibling both succeed.
    let temp = big_tree(BIG_FILES);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_repo", json!({})));
    std::thread::sleep(CANCEL_DELAY);
    session.send(&tool_call(2, "index_status", json!({})));
    session.send(&tool_call(3, "index_status", json!({})));
    std::thread::sleep(CANCEL_DELAY);
    session.send(&cancelled_notif(2));
    let responses = recv_until_all(&session, &[1, 3], &[2]);
    assert_eq!(responses.len(), 2);
    for response in &responses {
        assert_tool_success(response);
        assert!(tool_body(response).is_object(), "{response:#}");
    }
    assert_no_id_for(&session, &[2], QUEUED_QUIET_WINDOW);
    session.close_stdin();
    assert!(session.wait_clean().success());
}

/// INTENT: the session stays fully usable after a cancel: re-index succeeds
/// and the rebuilt index is queryable.
/// KILLS: cancel-poisons-session mutants.
/// Catalog: K2.8 KEEP.
#[test]
fn session_usable_after_cancel_reindex_and_search() {
    let temp = big_tree(BIG_FILES);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_repo", json!({})));
    std::thread::sleep(CANCEL_DELAY);
    session.send(&cancelled_notif(1));
    session.send(&ping(2));
    assert_ping_ok(&recv_until(&session, 2, &[1]), 2);

    session.send(&tool_call(3, "index_repo", json!({})));
    let reindex = recv_until(&session, 3, &[1]);
    assert_tool_success(&reindex);
    assert!(tool_body(&reindex).is_object(), "{reindex:#}");

    session.send(&tool_call(
        4,
        "keyword_search",
        json!({"query": "k2requelch_7", "limit": 4}),
    ));
    let search = recv_until(&session, 4, &[1]);
    assert_tool_success(&search);
    let body = tool_body(&search);
    assert!(
        body["h"].as_array().is_some_and(|h| !h.is_empty()),
        "{search:#}"
    );
    assert!(tool_text(&search).contains("k2requelch_7"), "{search:#}");

    session.close_stdin();
    assert!(session.wait_clean().success());
}

/// INTENT: stray cancels are silent and harmless, and a pre-cancel for an
/// unissued id does not poison the later call reusing that id.
/// KILLS: error-on-unknown-cancel / pre-cancel-poison mutants.
/// Catalog: K2.10 KEEP (covers K6 stray + K7 pre-cancel).
#[test]
fn stray_cancels_are_silent_and_harmless() {
    let temp = tempfile::tempdir().unwrap();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();

    session.send(&cancelled_notif(999));
    assert_silent(&session, STRAY_QUIET_WINDOW);
    session.send(&ping(10));
    assert_ping_ok(&session.recv(), 10);

    session.send(&tool_call(11, "index_status", json!({})));
    let status = session.recv();
    assert_eq!(status["id"], 11, "{status:#}");
    assert_tool_success(&status);
    session.send(&cancelled_notif(11));
    assert_silent(&session, STRAY_QUIET_WINDOW);

    session.send(&cancelled_notif(11));
    session.send(&cancelled_notif(11));
    session.send(&cancelled_notif(4242));
    assert_silent(&session, STRAY_QUIET_WINDOW);

    // Pre-cancel for an id never issued: the later call with that id still
    // runs normally.
    session.send(&cancelled_notif(12));
    session.send(&tool_call(12, "index_status", json!({})));
    let late = session.recv();
    assert_eq!(late["id"], 12, "{late:#}");
    assert_tool_success(&late);

    session.send(&ping(13));
    assert_ping_ok(&session.recv(), 13);
    session.close_stdin();
    assert!(session.wait_clean().success());
}

/// INTENT: EOF/abort at any point -- before init, idle, mid-call, after a
/// cancel, mid-burst -- ends the server with exit 0, never a hang or failure.
/// KILLS: hang-on-eof / abort-hang / exit-code mutants.
/// Facets: pre-init EOF (K2.1) + idle EOF (K2.2) + mid-call abort (K2.3) +
/// EOF right after cancel (K2.4) + stdin slammed shut mid-burst (K4.3).
#[test]
fn eof_and_abort_handling_all_phases_exit_ok() {
    // Facet 1 (K2.1): EOF before initialize exits 0 -- not a server failure.
    let temp = tempfile::tempdir().unwrap();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "EOF before initialize exited {status}");

    // Facet 2 (K2.2): EOF mid-session while idle ends gracefully with exit 0.
    let temp = tempfile::tempdir().unwrap();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&ping(1));
    assert_ping_ok(&session.recv(), 1);
    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "EOF mid-session exited {status}");

    // Facet 3 (K2.3): client abort (stdin close) mid-tool-call terminates
    // the server with exit 0 within a bounded wait.
    let temp = big_tree(ABORT_FILES);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_repo", json!({})));
    std::thread::sleep(CANCEL_DELAY);
    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "abort mid-call exited {status}");

    // Facet 4 (K2.4): EOF immediately after cancelling an in-flight call
    // still exits 0.
    let temp = big_tree(ABORT_FILES);
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    session.send(&tool_call(1, "index_repo", json!({})));
    std::thread::sleep(CANCEL_DELAY);
    session.send(&cancelled_notif(1));
    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "EOF after cancel exited {status}");

    // Facet 5 (K4.3): stdin slammed shut mid-burst with zero reads still ends
    // the server with exit 0 -- no hang, no failure code.
    let temp = small_tree();
    let mut session = LiveSession::spawn(Some(temp.path()));
    session.handshake();
    for request in burst_script() {
        session.send(&request);
    }
    // Slam: close stdin immediately without reading a single response. The
    // harness reader thread drains stdout, so an unread burst never wedges
    // the server on a full pipe.
    session.close_stdin();
    let status = session.wait_clean();
    assert!(status.success(), "abort mid-burst exited {status}");
}

const QUERIES: [&str; 3] = ["redhammer", "blueanvil", "greenchisel"];
const READ_IDS: [&str; 3] = ["a.rs#L1-L1", "b.rs#L1-L1", "c.rs#L1-L2"];

/// 24 mixed requests (ids 1..=24) for the mid-burst slam facet.
fn burst_script() -> Vec<Value> {
    let mut script = Vec::with_capacity(24);
    for (i, id) in (1..=24u32).enumerate() {
        let cycle = QUERIES[i % QUERIES.len()];
        let payload = match i % 6 {
            0 => tool_call(id, "keyword_search", json!({"query": cycle, "limit": 4})),
            1 => tool_call(id, "index_status", json!({})),
            2 => tool_call(id, "code_search", json!({"query": cycle, "limit": 4})),
            3 => ping(id),
            4 => tools_list(id),
            _ => tool_call(
                id,
                "code_read",
                json!({"ids": [READ_IDS[i % READ_IDS.len()]]}),
            ),
        };
        script.push(payload);
    }
    script
}

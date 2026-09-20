//! E4 end-to-end error drills for ast-sgrep-codemode.
//!
//! E1 pins the `CallError` taxonomy TABLE, E2 pins PROPAGATION, E3 pins
//! RELATIONS. E4 pins DRILLS: full session flows — a working session executing
//! calls/plans/batches, an injected fault mid-flow, then the end-to-end
//! `CallError` discriminant AND the documented post-fault state (sticky
//! budget, atomic batch, resumable plan). Discriminant assertions via
//! `matches!` only (never message text). Deterministic, tempfile fixtures,
//! no new deps.
//!
//! Drill map (fault -> post-fault state -> test):
//! - bad tool args mid-batch over sticky serve; per-call isolation, Bye
//!   -> e4_serve_batch_fault_mid_stream_isolates_and_resumes
//! - unknown tool mid-plan; corrected plan runs (resumable plan)
//!   -> e4_unknown_tool_in_plan_resumes_with_corrected_plan
//! - budget exhaustion mid-plan; frozen count, repeat payload (sticky budget)
//!   -> e4_budget_exhaustion_mid_plan_sticks_terminal
//! - session root deleted mid-flow; pure tools unaffected, repair resumes
//!   -> e4_deleted_root_fault_scoped_and_recovers_after_repair
//! - edits[] batch with a failing entry; zero writes (atomic batch), retry ok
//!   -> e4_atomic_edit_batch_zero_writes_then_resumes
//! - chained double fault: unknown-tool then budget on one session
//!   -> e4_chained_double_fault_unknown_then_budget
//! - oversized response mid-flow; session resumes after the rejection
//!   -> e4_oversized_response_fault_then_resumes
//!
//! Folded out (MERGE verdict; pinned at its anchor, not here):
//! - e4_plan_prefix_applied_then_remainder_resumes -> e3_plan_fail_closed_never_ok (pass3)

#[path = "error_testkit.rs"]
mod error_testkit;

use ast_sgrep_codemode::{parse_plan, run_plan, CallError, ServeRequest, ServeResponse};
use error_testkit::{batch_call, serve_lines, serve_request_line, session_at};
use serde_json::{json, Value};

/// INTENT=mid-stream batch fault isolated per-call, stream reaches Bye.
/// KILLS=isolation-breach, stream-abort.
/// ABSORBS=none.
#[test]
fn e4_serve_batch_fault_mid_stream_isolates_and_resumes() {
    // Full sticky-serve flow: good call, batch with bad args in the middle,
    // good call, End. The fault stays per-call isolated (neighbors ok), the
    // worker keeps serving, and the stream reaches Bye (resumable).
    let temp = tempfile::tempdir().expect("tempdir");
    let good = |id: &str| ServeRequest::Call {
        id: id.to_string(),
        tool: "catalog_search".to_string(),
        args: json!({"query": "search"}),
    };
    let mut input = serve_request_line(&good("g0"));
    input.push_str(&serve_request_line(&ServeRequest::Batch {
        id: "b0".to_string(),
        calls: vec![
            batch_call("bg0", "catalog_search", json!({"query": "search"})),
            batch_call("bb", "select", json!({})),
            batch_call("bg1", "catalog_search", json!({"query": "find"})),
        ],
        parallel_mode: None,
    }));
    input.push_str(&serve_request_line(&good("g1")));
    input.push_str(&serve_request_line(&ServeRequest::End));

    let (result, lines) = serve_lines(input, temp.path());
    assert!(result.is_ok(), "serve survives the mid-stream fault");
    assert_eq!(lines.len(), 4);

    for (line, expect_id) in [(&lines[0], "g0"), (&lines[2], "g1")] {
        let response: ServeResponse = serde_json::from_str(line).expect("result line");
        match response {
            ServeResponse::Result {
                id,
                ok,
                value,
                error,
            } => {
                assert_eq!(id, expect_id);
                assert!(ok, "good call {expect_id} tainted by the fault");
                assert!(value.is_some());
                assert!(error.is_none());
            }
            other => panic!("tool outcome must be Result, got {other:?}"),
        }
    }

    // BatchResult carries wall_ms: u128, which serde_json cannot deserialize;
    // pin the envelope via its `type` tag discriminant plus field shapes.
    let batch: Value = serde_json::from_str(&lines[1]).expect("batch line");
    assert_eq!(
        batch.get("type").and_then(Value::as_str),
        Some("batch_result")
    );
    assert_eq!(batch.get("id").and_then(Value::as_str), Some("b0"));
    assert_eq!(batch.get("all_ok").and_then(Value::as_bool), Some(false));
    assert_eq!(batch.get("mode").and_then(Value::as_str), Some("serial"));
    let rows = batch
        .get("results")
        .and_then(Value::as_array)
        .expect("rows");
    assert_eq!(rows.len(), 3);
    let pattern: Vec<bool> = rows
        .iter()
        .map(|row| row.get("ok").and_then(Value::as_bool).expect("ok flag"))
        .collect();
    assert_eq!(pattern, vec![true, false, true]);
    for row in rows {
        let ok = row.get("ok").and_then(Value::as_bool).expect("ok flag");
        assert_eq!(row.get("value").is_some(), ok);
        assert_eq!(row.get("error").is_some(), !ok);
    }

    let last: ServeResponse = serde_json::from_str(&lines[3]).expect("bye line");
    assert!(matches!(last, ServeResponse::Bye), "got {last:?}");
}

/// INTENT=UnknownTool mid-plan fails, same session resumes corrected plan.
/// KILLS=session-poison-after-UnknownTool.
/// ABSORBS=none.
#[test]
fn e4_unknown_tool_in_plan_resumes_with_corrected_plan() {
    // Working session runs a good plan, then a plan with an unknown tool
    // mid-plan fails end-to-end as UnknownTool with the executed prefix pinned
    // — and the same session resumes with the corrected plan (resumable plan).
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    let warmup = parse_plan(&json!({"steps": [
        {"id": "w", "tool": "catalog_search", "args": {"query": "search"}},
    ]}))
    .expect("warmup plan parses");
    let ok = run_plan(&mut session, &warmup).expect("warmup plan succeeds");
    assert!(ok.ok);
    assert_eq!(session.call_count(), 1);

    let faulty = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "no-such-tool", "args": {}},
        {"id": "c", "tool": "catalog_search", "args": {"query": "find"}},
    ]}))
    .expect("faulty plan parses");
    let err = run_plan(&mut session, &faulty).expect_err("unknown step must fail");
    assert!(matches!(err, CallError::UnknownTool(_)), "got {err:?}");
    assert_eq!(session.call_count(), 3);
    assert!(!session.exhausted());

    let corrected = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "c", "tool": "catalog_search", "args": {"query": "find"}},
    ]}))
    .expect("corrected plan parses");
    let ok = run_plan(&mut session, &corrected).expect("session resumes after UnknownTool");
    assert!(ok.ok);
    assert_eq!(session.call_count(), 5);
}

/// INTENT=budget aborts mid-plan then sticks terminal (non-resumable).
/// KILLS=budget-recovery-after-exhaustion.
/// ABSORBS=none.
/// OVERLAP=e2 budget pins (adds mid-plan flow).
#[test]
fn e4_budget_exhaustion_mid_plan_sticks_terminal() {
    // Working session nearly spends its budget on a good plan; the next plan
    // aborts mid-flow with BudgetExhausted — then the budget sticks terminal:
    // same payload on repeat, frozen counter, exhausted (sticky budget, the
    // one fault class that is NOT resumable).
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    session.max_calls = 3;
    let warmup = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "catalog_search", "args": {"query": "find"}},
    ]}))
    .expect("warmup plan parses");
    assert!(run_plan(&mut session, &warmup).expect("warmup succeeds").ok);
    assert_eq!(session.call_count(), 2);
    assert!(!session.exhausted());

    let over = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "catalog_search", "args": {"query": "find"}},
    ]}))
    .expect("over-budget plan parses");
    let err = run_plan(&mut session, &over).expect_err("plan must stop at budget");
    assert!(matches!(err, CallError::BudgetExhausted(3)), "got {err:?}");
    assert_eq!(session.call_count(), 3);
    assert!(session.exhausted());

    for _ in 0..2 {
        let err = session
            .call("catalog_search", json!({"query": "search"}))
            .expect_err("spent budget must keep failing");
        assert!(matches!(err, CallError::BudgetExhausted(3)), "got {err:?}");
    }
    assert_eq!(session.call_count(), 3);
    assert!(session.exhausted());
}

/// INTENT=root deletion fails bound calls as Other, pure tools green, reindex resumes.
/// KILLS=fault-scope-breach, stale-searcher-after-repair.
/// ABSORBS=none.
#[test]
fn e4_deleted_root_fault_scoped_and_recovers_after_repair() {
    // Working session over an indexed root; the root is deleted mid-flow so
    // root-touching calls fail end-to-end as Other while pure tools stay
    // green (fault scoping) — then files are restored plus a reindex (which
    // drops the stale Searcher) and the same session serves reads again
    // (resumable after repair).
    let temp = tempfile::tempdir().expect("tempdir");
    let proj = temp.path().join("proj");
    std::fs::create_dir_all(&proj).expect("mkdir");
    std::fs::write(proj.join("a.rs"), "fn alpha() {}\nfn beta() {}\n").expect("write");
    let mut session = session_at(&proj);
    assert!(session.call("index_repo", json!({})).is_ok());
    let window = session
        .call("read", json!({"path": "a.rs", "start": 1, "end": 2}))
        .expect("read before fault");
    assert_eq!(window.get("count").and_then(Value::as_u64), Some(1));
    assert_eq!(session.call_count(), 2);

    std::fs::remove_dir_all(&proj).expect("delete root");
    assert!(!proj.exists());
    let err = session
        .call("search", json!({"query": "alpha"}))
        .expect_err("root-touching call must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    assert!(
        session
            .call("catalog_search", json!({"query": "search"}))
            .is_ok(),
        "pure tools must survive root deletion"
    );
    assert_eq!(session.call_count(), 4);

    std::fs::create_dir_all(&proj).expect("repair root");
    std::fs::write(proj.join("a.rs"), "fn alpha() {}\nfn beta() {}\n").expect("rewrite");
    assert!(
        session.call("index_repo", json!({})).is_ok(),
        "reindex must rebuild the repaired root"
    );
    let window = session
        .call("read", json!({"path": "a.rs", "start": 1, "end": 2}))
        .expect("repaired root must serve reads");
    assert_eq!(window.get("count").and_then(Value::as_u64), Some(1));
    let text = window["windows"][0]["text"].as_str().expect("text");
    assert!(text.contains("fn alpha"), "{text}");
    assert_eq!(session.call_count(), 6);
    assert!(!session.exhausted());
}

/// INTENT=failing edits[] entry fails whole call, zero writes, retry applies.
/// KILLS=partial-batch-write.
/// ABSORBS=none.
#[test]
fn e4_atomic_edit_batch_zero_writes_then_resumes() {
    // A working session fires one edit call with an edits[] batch whose second
    // entry cannot match: the whole call fails as Other and NEITHER file is
    // touched (atomic batch — two-phase commit, zero writes on any failure).
    // The corrected batch then applies on the same session (resumable).
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("a.txt"), "alpha one\n").expect("write a");
    std::fs::write(temp.path().join("b.txt"), "beta two\n").expect("write b");
    let mut session = session_at(temp.path());
    assert!(session
        .call("catalog_search", json!({"query": "search"}))
        .is_ok());

    let err = session
        .call(
            "edit",
            json!({"edits": [
                {"path": "a.txt", "oldText": "alpha one", "newText": "alpha 1"},
                {"path": "b.txt", "oldText": "not-present-anywhere", "newText": "x"},
            ]}),
        )
        .expect_err("partially failing edits[] must fail");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    assert_eq!(
        std::fs::read_to_string(temp.path().join("a.txt")).expect("reread a"),
        "alpha one\n",
        "atomic batch must leave the valid entry unapplied"
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("b.txt")).expect("reread b"),
        "beta two\n"
    );

    let applied = session
        .call(
            "edit",
            json!({"edits": [
                {"path": "a.txt", "oldText": "alpha one", "newText": "alpha 1"},
                {"path": "b.txt", "oldText": "beta two", "newText": "beta 2"},
            ]}),
        )
        .expect("corrected batch must apply");
    assert_eq!(applied.get("ok").and_then(Value::as_bool), Some(true));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("a.txt")).expect("reread a"),
        "alpha 1\n"
    );
    assert_eq!(
        std::fs::read_to_string(temp.path().join("b.txt")).expect("reread b"),
        "beta 2\n"
    );
    assert_eq!(session.call_count(), 3);
}

/// INTENT=UnknownTool then, after resume spends budget, terminal BudgetExhausted in order.
/// KILLS=fault-order-swap, non-sticky-terminal.
/// ABSORBS=none.
#[test]
fn e4_chained_double_fault_unknown_then_budget() {
    // Chained double fault on one session: an unknown-tool plan fault first
    // (UnknownTool, resumable), then — after resuming work spends the rest of
    // the budget — a terminal BudgetExhausted (sticky). Both discriminants in
    // order, then the frozen terminal state.
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    session.max_calls = 5;
    let warmup = parse_plan(&json!({"steps": [
        {"id": "w", "tool": "catalog_search", "args": {"query": "search"}},
    ]}))
    .expect("warmup plan parses");
    assert!(run_plan(&mut session, &warmup).expect("warmup succeeds").ok);
    assert_eq!(session.call_count(), 1);

    let faulty = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "no-such-tool", "args": {}},
    ]}))
    .expect("faulty plan parses");
    let first = run_plan(&mut session, &faulty).expect_err("fault 1 must fail");
    assert!(matches!(first, CallError::UnknownTool(_)), "got {first:?}");
    assert_eq!(session.call_count(), 3);
    assert!(!session.exhausted());

    let resume = parse_plan(&json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "catalog_search", "args": {"query": "find"}},
    ]}))
    .expect("resume plan parses");
    assert!(run_plan(&mut session, &resume).expect("resume succeeds").ok);
    assert_eq!(session.call_count(), 5);
    assert!(session.exhausted());

    let second = session
        .call("catalog_search", json!({"query": "search"}))
        .expect_err("fault 2 must fail");
    assert!(
        matches!(second, CallError::BudgetExhausted(5)),
        "got {second:?}"
    );
    assert_eq!(session.call_count(), 5);
}

/// INTENT=oversized response fails Other closed (never truncated-ok), session resumes.
/// KILLS=truncated-ok, session-poison.
/// ABSORBS=none.
/// OVERLAP=oracle oversized pin (adds resume + budget-charge).
#[test]
fn e4_oversized_response_fault_then_resumes() {
    // A working session whose call would return more than the per-call cap
    // fails end-to-end as Other (fail closed, never truncated-ok) — and the
    // same session keeps serving afterwards (resumable; the rejection still
    // consumes its budget unit like any other dispatched call).
    let temp = tempfile::tempdir().expect("tempdir");
    let mut session = session_at(temp.path());
    assert!(session
        .call("select", json!({"value": {"v": 1}, "fields": ["v"]}))
        .is_ok());

    let blob = "x".repeat(ast_sgrep_core::MAX_STDIN_LINE_BYTES + 1024);
    let err = session
        .call(
            "select",
            json!({"value": {"blob": blob}, "fields": ["blob"]}),
        )
        .expect_err("oversized response must fail closed");
    assert!(matches!(err, CallError::Other(_)), "got {err:?}");
    assert_eq!(session.call_count(), 2);

    assert!(
        session
            .call("catalog_search", json!({"query": "search"}))
            .is_ok(),
        "session must resume after the oversized rejection"
    );
    assert_eq!(session.call_count(), 3);
    assert!(!session.exhausted());
}

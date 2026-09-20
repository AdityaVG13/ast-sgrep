//! budget numeric contract: fresh session + huge budget + monotonicity + trip.
//!
//! Single-test contract suite absorbing every session-budget numeric from
//! `numerical_pass{1,2,3,4}` plus the fresh-session half of the split
//! constants test. Pure transforms only — no index I/O.

use ast_sgrep_codemode::{CallError, CodeModeSession};
use ast_sgrep_testkit as testkit;
use ast_sgrep_testkit::{hits_fixture, scored_hits5, scored_hits6};
use serde_json::json;

// Keep file-local: budget-saturation driver — serves identical pure calls
// until the budget trips and returns calls served, asserting the trip
// discriminant is always BudgetExhausted. Single-suite driver; no testkit
// driver fits this shape and no sibling needs it.
fn serve_until_exhausted(session: &mut CodeModeSession, attempts: usize) -> usize {
    let mut served = 0;
    for _ in 0..attempts {
        match session.call("filter_hits", json!({"hits": scored_hits5()})) {
            Ok(_) => served += 1,
            Err(e) => {
                assert!(
                    matches!(e, CallError::BudgetExhausted(_)),
                    "trip must be budget, got {e:?}"
                );
                break;
            }
        }
    }
    served
}

/// INTENT=session-budget numerics: fresh session is 64-budget/0-used/not-exhausted, usize::MAX budget never trips or overflows on few calls, served(b) is non-decreasing and exactly b, and a 4-tool mixed flow trips BudgetExhausted(4) on the 5th call with the counter pinned.
/// KILLS=constant-value (fresh-budget literal), budget-overflow (saturating-add/exhausted-predicate), non-monotonic-budget (inverted-cap), budget-accounting (counter-increment/exhausted-predicate, per-tool-miscount/payload).
/// ABSORBS=budget_constants_and_fresh_session_are_hand_computed (fresh-session half), session_huge_budget_never_exhausts_on_few_calls, budget_larger_never_serves_fewer_calls, budget_served_never_exceeds_budget, budget_mixed_tool_flow_trips_at_exact_count
#[test]
fn budget_contract() {
    // §1 ABSORBED: fresh-session half — 64-call budget, zero consumed,
    // not exhausted.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let session = testkit::session_at(temp.path());
        assert_eq!(session.max_calls, 64);
        assert_eq!(session.call_count(), 0);
        assert!(!session.exhausted());
    }
    // §2 ABSORBED: session_huge_budget_never_exhausts_on_few_calls —
    // max_calls=usize::MAX serves one pure call (counter→1), never trips.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut session = testkit::session_at(temp.path());
        session.max_calls = usize::MAX;
        assert!(!session.exhausted());
        assert_eq!(session.call_count(), 0);
        let out = session
            .call("filter_hits", json!({"hits": hits_fixture()}))
            .expect("huge budget runs");
        assert_eq!(out["hit_count"], json!(3));
        assert_eq!(session.call_count(), 1);
        assert!(!session.exhausted());
    }
    // §3 ABSORBED: budget_larger_never_serves_fewer_calls — served(b)
    // non-decreasing over budgets 1..8 with saturating attempts.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut prev_served = 0;
        for budget in [1usize, 2, 3, 5, 8] {
            let mut session = testkit::session_at(temp.path());
            session.max_calls = budget;
            let served = serve_until_exhausted(&mut session, 10);
            assert!(
                served >= prev_served,
                "budget {budget} served {served} < {prev_served}"
            );
            assert_eq!(session.call_count(), served);
            prev_served = served;
        }
    }
    // §4 ABSORBED: budget_served_never_exceeds_budget — served(b)==b
    // exactly (cap binds, one call each, exhausted after).
    {
        let temp = tempfile::tempdir().expect("tempdir");
        for budget in [1usize, 2, 4, 7] {
            let mut session = testkit::session_at(temp.path());
            session.max_calls = budget;
            let served = serve_until_exhausted(&mut session, 20);
            assert!(served <= budget, "served {served} exceeds budget {budget}");
            assert_eq!(served, budget);
            assert!(session.exhausted());
        }
    }
    // §5 ABSORBED: budget_mixed_tool_flow_trips_at_exact_count — 4
    // DISTINCT pure tools consume 1..4; the 5th trips BudgetExhausted(4)
    // and the counter stays pinned at 4.
    {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut session = testkit::session_at(temp.path());
        session.max_calls = 4;
        session
            .call("catalog_search", json!({"query": "search"}))
            .expect("step 1 runs");
        assert_eq!(session.call_count(), 1);
        assert!(!session.exhausted());
        session
            .call("filter_hits", json!({"hits": scored_hits6()}))
            .expect("step 2 runs");
        assert_eq!(session.call_count(), 2);
        session
            .call("select", json!({"value": [{"a": 1}], "fields": ["a"]}))
            .expect("step 3 runs");
        assert_eq!(session.call_count(), 3);
        session
            .call("catalog_describe", json!({"name": "search"}))
            .expect("step 4 runs");
        assert_eq!(session.call_count(), 4);
        assert!(session.exhausted());
        let err = session
            .call("filter_hits", json!({"hits": scored_hits6()}))
            .expect_err("5th call must trip");
        assert!(matches!(err, CallError::BudgetExhausted(4)), "got {err:?}");
        assert_eq!(session.call_count(), 4);
        assert!(session.exhausted());
    }
}

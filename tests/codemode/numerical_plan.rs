//! plan numeric contract: empty-plan rejection under degenerate budgets.
//!
//! Single-test contract suite absorbing the plan half of the split
//! empty-envelope test from `numerical_pass2`. Pure plan harness only — no
//! index I/O, no file-local helpers.

use ast_sgrep_codemode::{CallError, parse_plan, run_plan};
use ast_sgrep_testkit as testkit;
use serde_json::json;

/// INTENT=empty plan rejects with InvalidArgs even when the session budget is 0 or usize::MAX (steps check precedes calls).
/// KILLS=error-discriminant + validation-order (InvalidArgs-vs-Ok, steps-check-first).
/// ABSORBS=empty_batch_and_empty_plan_reject_under_degenerate_budgets (plan half)
#[test]
fn plan_contract() {
    // ABSORBED: empty_batch plan half — zero-size totality plus validation
    // order: the steps check precedes budget consumption.
    let temp = tempfile::tempdir().expect("tempdir");
    let empty = parse_plan(&json!({"steps": []})).expect("empty parses");
    for max in [0usize, usize::MAX] {
        let mut session = testkit::session_at(temp.path());
        session.max_calls = max;
        let err = run_plan(&mut session, &empty).expect_err("empty plan never runs");
        assert!(matches!(err, CallError::InvalidArgs(_)), "got {err:?}");
    }
}

use ast_sgrep_cli::bench::*;

fn quarantine_history() -> serde_json::Value {
    serde_json::json!({
        "verdict": "quarantine_cv",
        "cv_pct": 926.0,
        "ratchet_ok": false,
    })
}

fn regression_history() -> serde_json::Value {
    serde_json::json!({
        "verdict": "reject_regression",
        "regression_pct": 42.0,
        "ratchet_ok": false,
    })
}

#[test]
fn ratchet_policy_quarantine_soft_regression_hard() {
    // One test, sequential env states: nothing else touches
    // ASGREP_BENCH_STRICT, so no inter-test race.
    let quarantined = Some(quarantine_history());
    let regressed = Some(regression_history());
    std::env::set_var("ASGREP_BENCH_STRICT", "1");
    assert!(
        enforce_bench_ratchet(&quarantined, "test").is_err(),
        "strict quarantine must fail hard"
    );
    assert!(
        enforce_bench_ratchet(&regressed, "test").is_err(),
        "regression must fail hard under strict"
    );
    std::env::remove_var("ASGREP_BENCH_STRICT");
    assert!(
        enforce_bench_ratchet(&quarantined, "test").is_ok(),
        "default quarantine must warn, not fail"
    );
    assert!(
        enforce_bench_ratchet(&regressed, "test").is_err(),
        "regression must fail hard by default"
    );
}

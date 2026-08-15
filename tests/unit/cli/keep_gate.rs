use super::*;

fn t() -> KeepThresholds {
    parse_thresholds(THRESHOLDS_JSON)
}

#[test]
fn thresholds_are_oom_tighter_than_fifty() {
    let th = t();
    assert!(th.primary_regression_pct <= 3.0);
    assert!(th.geomean_regression_pct <= 5.0);
    assert!(th.primary_regression_pct * 10.0 < 50.0);
}

#[test]
fn pass_at_primary_threshold() {
    let v = evaluate_keep(
        KeepSample {
            avg_ms: 103.0,
            cv_pct: 1.0,
            geomean_ms: None,
        },
        KeepPrior {
            avg_ms: Some(100.0),
            geomean_ms: None,
            placeholder: false,
        },
        t(),
    );
    assert_eq!(
        v,
        KeepVerdict::Keep {
            regression_pct: 3.0
        }
    );
}

#[test]
fn fail_above_primary_threshold() {
    let v = evaluate_keep(
        KeepSample {
            avg_ms: 103.1,
            cv_pct: 1.0,
            geomean_ms: None,
        },
        KeepPrior {
            avg_ms: Some(100.0),
            geomean_ms: None,
            placeholder: false,
        },
        t(),
    );
    match v {
        KeepVerdict::RejectRegression {
            kind, threshold, ..
        } => {
            assert_eq!(kind, "primary");
            assert_eq!(threshold, 3.0);
        }
        other => panic!("expected reject, got {other:?}"),
    }
}

#[test]
fn fail_above_geomean_threshold() {
    let v = evaluate_keep(
        KeepSample {
            avg_ms: 100.0,
            cv_pct: 1.0,
            geomean_ms: Some(106.0),
        },
        KeepPrior {
            avg_ms: Some(100.0),
            geomean_ms: Some(100.0),
            placeholder: false,
        },
        t(),
    );
    match v {
        KeepVerdict::RejectRegression { kind, .. } => assert_eq!(kind, "geomean"),
        other => panic!("expected geomean reject, got {other:?}"),
    }
}

#[test]
fn quarantine_when_cv_exceeds_five() {
    let v = evaluate_keep(
        KeepSample {
            avg_ms: 90.0,
            cv_pct: 5.01,
            geomean_ms: None,
        },
        KeepPrior {
            avg_ms: Some(100.0),
            geomean_ms: None,
            placeholder: false,
        },
        t(),
    );
    assert_eq!(v, KeepVerdict::QuarantineCv { cv_pct: 5.01 });
    assert!(v.is_hard_fail());
}

#[test]
fn placeholder_establishes_baseline_not_keep() {
    let v = evaluate_keep(
        KeepSample {
            avg_ms: 12.0,
            cv_pct: 1.0,
            geomean_ms: Some(12.0),
        },
        KeepPrior {
            avg_ms: None,
            geomean_ms: None,
            placeholder: true,
        },
        t(),
    );
    assert_eq!(v, KeepVerdict::EstablishBaseline);
    assert!(!v.is_hard_fail());
}

#[test]
fn sanitize_suite_label() {
    assert_eq!(
        sanitize_label("suite:sample:default"),
        "suite-sample-default"
    );
}

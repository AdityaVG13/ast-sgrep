use ast_sgrep_cli::keep_gate::*;

fn thresholds() -> KeepThresholds {
    KeepThresholds {
        primary_regression_pct: 3.0,
        geomean_regression_pct: 5.0,
        cv_ineligible_pct: 5.0,
    }
}

fn prior(avg_ms: f64) -> KeepPrior {
    KeepPrior {
        avg_ms: Some(avg_ms),
        geomean_ms: None,
        placeholder: false,
    }
}

#[test]
fn verdicts_quarantine_reject_baseline_keep() {
    let t = thresholds();
    // Noise wins over every comparison: quarantine even a fast sample.
    assert!(matches!(
        evaluate_keep(
            KeepSample {
                avg_ms: 1.0,
                cv_pct: 926.0,
                geomean_ms: None
            },
            prior(10.0),
            t
        ),
        KeepVerdict::QuarantineCv { .. }
    ));
    // No usable prior: establish, never reject.
    assert!(matches!(
        evaluate_keep(
            KeepSample {
                avg_ms: 50.0,
                cv_pct: 1.0,
                geomean_ms: None
            },
            KeepPrior {
                avg_ms: None,
                geomean_ms: None,
                placeholder: true
            },
            t
        ),
        KeepVerdict::EstablishBaseline
    ));
    // +10% primary average rejects at a 3% threshold.
    assert!(matches!(
        evaluate_keep(
            KeepSample {
                avg_ms: 11.0,
                cv_pct: 1.0,
                geomean_ms: None
            },
            prior(10.0),
            t
        ),
        KeepVerdict::RejectRegression {
            kind: "primary",
            ..
        }
    ));
    // +2% keeps.
    assert!(matches!(
        evaluate_keep(
            KeepSample {
                avg_ms: 10.2,
                cv_pct: 1.0,
                geomean_ms: None
            },
            prior(10.0),
            t
        ),
        KeepVerdict::Keep { .. }
    ));
    // Geomean +6% rejects even when the primary average keeps.
    assert!(matches!(
        evaluate_keep(
            KeepSample {
                avg_ms: 10.2,
                cv_pct: 1.0,
                geomean_ms: Some(10.6)
            },
            KeepPrior {
                avg_ms: Some(10.0),
                geomean_ms: Some(10.0),
                placeholder: false
            },
            t
        ),
        KeepVerdict::RejectRegression {
            kind: "geomean",
            ..
        }
    ));
}

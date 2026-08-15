//! Keep-gate that refuses to lie: committed history priors, −3%/−5% class
//! thresholds, cv quarantine, and host/SHA/profile attribution.
//!
//! Thresholds are loaded from `.bench-history/thresholds.json` (SSoT).

use serde_json::Value;
use std::path::{Path, PathBuf};

pub const THRESHOLDS_JSON: &str = include_str!("../../../.bench-history/thresholds.json");

#[derive(Debug, Clone, Copy)]
pub struct KeepThresholds {
    pub primary_regression_pct: f64,
    pub geomean_regression_pct: f64,
    pub cv_ineligible_pct: f64,
}

impl KeepThresholds {
    pub fn from_repo() -> Self {
        parse_thresholds(THRESHOLDS_JSON)
    }
}

pub fn parse_thresholds(json: &str) -> KeepThresholds {
    let v: Value = serde_json::from_str(json).unwrap_or_else(|_| serde_json::json!({}));
    KeepThresholds {
        primary_regression_pct: v["primary_regression_pct"].as_f64().unwrap_or(3.0),
        geomean_regression_pct: v["geomean_regression_pct"].as_f64().unwrap_or(5.0),
        cv_ineligible_pct: v["cv_ineligible_pct"].as_f64().unwrap_or(5.0),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct KeepSample {
    pub avg_ms: f64,
    pub cv_pct: f64,
    pub geomean_ms: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
pub struct KeepPrior {
    pub avg_ms: Option<f64>,
    pub geomean_ms: Option<f64>,
    pub placeholder: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum KeepVerdict {
    Keep {
        regression_pct: f64,
    },
    EstablishBaseline,
    RejectRegression {
        regression_pct: f64,
        threshold: f64,
        kind: &'static str,
    },
    QuarantineCv {
        cv_pct: f64,
    },
}

impl KeepVerdict {
    pub fn is_hard_fail(&self) -> bool {
        matches!(
            self,
            Self::RejectRegression { .. } | Self::QuarantineCv { .. }
        )
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Keep { .. } => "keep",
            Self::EstablishBaseline => "establish_baseline",
            Self::RejectRegression { .. } => "reject_regression",
            Self::QuarantineCv { .. } => "quarantine_cv",
        }
    }
}

pub fn evaluate_keep(
    sample: KeepSample,
    prior: KeepPrior,
    thresholds: KeepThresholds,
) -> KeepVerdict {
    if sample.cv_pct > thresholds.cv_ineligible_pct {
        return KeepVerdict::QuarantineCv {
            cv_pct: sample.cv_pct,
        };
    }
    let prior_avg = prior.avg_ms.filter(|ms| ms.is_finite() && *ms > 0.0);
    if prior.placeholder || prior_avg.is_none() {
        return KeepVerdict::EstablishBaseline;
    }
    let prior_avg = prior_avg.unwrap_or(0.0);
    let regression_pct = if prior_avg > 0.0 {
        ((sample.avg_ms - prior_avg) / prior_avg) * 100.0
    } else {
        0.0
    };
    if regression_pct > thresholds.primary_regression_pct {
        return KeepVerdict::RejectRegression {
            regression_pct,
            threshold: thresholds.primary_regression_pct,
            kind: "primary",
        };
    }
    if let (Some(geo), Some(prior_geo)) = (sample.geomean_ms, prior.geomean_ms) {
        if prior_geo > 0.0 && geo.is_finite() {
            let geo_reg = ((geo - prior_geo) / prior_geo) * 100.0;
            if geo_reg > thresholds.geomean_regression_pct {
                return KeepVerdict::RejectRegression {
                    regression_pct: geo_reg,
                    threshold: thresholds.geomean_regression_pct,
                    kind: "geomean",
                };
            }
        }
    }
    KeepVerdict::Keep { regression_pct }
}

pub fn prior_from_json(v: &Value) -> KeepPrior {
    KeepPrior {
        avg_ms: v["avg_search_ms"].as_f64(),
        geomean_ms: v["geomean_search_ms"].as_f64(),
        placeholder: v["placeholder"].as_bool().unwrap_or(false) || v["keep_eligible"] == false,
    }
}

pub fn history_dir() -> PathBuf {
    std::env::var_os("ASGREP_BENCH_HISTORY_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".bench-history"))
}

pub fn sanitize_label(label: &str) -> String {
    label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

pub fn committed_latest_path(label: &str) -> PathBuf {
    history_dir().join(format!("{}.latest.json", sanitize_label(label)))
}

pub fn run_snapshot_path(label: &str) -> PathBuf {
    history_dir().join(format!("{}.run.json", sanitize_label(label)))
}

pub fn load_committed_prior(label: &str) -> KeepPrior {
    let path = committed_latest_path(label);
    load_prior_file(&path)
}

pub fn load_prior_file(path: &Path) -> KeepPrior {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<Value>(&text)
            .map(|v| prior_from_json(&v))
            .unwrap_or(KeepPrior {
                avg_ms: None,
                geomean_ms: None,
                placeholder: true,
            }),
        Err(_) => KeepPrior {
            avg_ms: None,
            geomean_ms: None,
            placeholder: true,
        },
    }
}

pub fn geomean_ms(samples: &[f64]) -> Option<f64> {
    let pos: Vec<f64> = samples
        .iter()
        .copied()
        .filter(|x| x.is_finite() && *x > 0.0)
        .collect();
    if pos.is_empty() {
        return None;
    }
    let ln_sum: f64 = pos.iter().map(|x| x.ln()).sum();
    Some((ln_sum / pos.len() as f64).exp())
}

pub fn attribution() -> (String, String, String) {
    let host = std::env::var("ASGREP_BENCH_HOST")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown-host".to_string());
    let git_sha = std::env::var("ASGREP_BENCH_GIT_SHA")
        .or_else(|_| std::env::var("GITHUB_SHA"))
        .unwrap_or_else(|_| git_head().unwrap_or_else(|| "unknown-sha".to_string()));
    let profile = std::env::var("ASGREP_BENCH_PROFILE").unwrap_or_else(|_| "unknown".to_string());
    (host, git_sha, profile)
}

fn git_head() -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

pub fn bench_ratchet_enabled() -> bool {
    match std::env::var("ASGREP_BENCH_RATCHET") {
        Ok(v) if v == "0" || v.eq_ignore_ascii_case("false") => false,
        _ => true,
    }
}

pub fn history_commit_enabled() -> bool {
    matches!(
        std::env::var("ASGREP_BENCH_HISTORY_COMMIT").ok().as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

#[cfg(test)]
mod tests {
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
}

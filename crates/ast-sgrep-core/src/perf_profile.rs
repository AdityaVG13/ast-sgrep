//! Optional stage-attribution profiler for hot paths.
//!
//! Enable with `ASGREP_PERF_PROFILE=1` (boolish: 1/true/yes/on).
//! Optional sink: `ASGREP_PERF_PROFILE_PATH=/path/to/file.jsonl` (append).
//! Default sink: stderr, one JSON object per line.
//!
//! Events (skill contract names under `"event"`):
//! - `perf.profile.run_start`
//! - `perf.profile.sample_collected` (only for wall spans, not every per-file sample)
//! - `perf.profile.span_summary`
//! - `perf.profile.run_complete`
//!
//! Zero work when the flag is off (single `OnceLock` read after first check).

use crate::env_flag::env_flag;
use serde_json::json;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const MAX_SAMPLES_PER_SPAN: usize = 4_096;

static ENABLED: OnceLock<bool> = OnceLock::new();
static RUN_SEQ: AtomicU64 = AtomicU64::new(1);
static COLLECTOR: OnceLock<Mutex<Collector>> = OnceLock::new();

struct SpanAcc {
    category: &'static str,
    evidence: &'static str,
    samples_us: Vec<u64>,
    cumulative_us: u128,
}

struct Collector {
    /// Nested run depth (index then search can nest in theory).
    depth: u32,
    run_id: u64,
    run_label: &'static str,
    run_start: Option<Instant>,
    spans: HashMap<&'static str, SpanAcc>,
}

impl Collector {
    fn new() -> Self {
        Self {
            depth: 0,
            run_id: 0,
            run_label: "",
            run_start: None,
            spans: HashMap::new(),
        }
    }
}

fn collector() -> &'static Mutex<Collector> {
    COLLECTOR.get_or_init(|| Mutex::new(Collector::new()))
}

/// True when `ASGREP_PERF_PROFILE` is boolish-true. Cached for the process.
#[inline]
pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| env_flag("ASGREP_PERF_PROFILE"))
}

/// Process-scoped profiling run. Emits `run_start` on create and
/// `span_summary` + `run_complete` on drop when profiling is enabled.
pub struct Run {
    active: bool,
}

impl Run {
    pub fn start(label: &'static str) -> Self {
        if !enabled() {
            return Self { active: false };
        }
        let run_id = RUN_SEQ.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut g) = collector().lock() {
            g.depth = g.depth.saturating_add(1);
            if g.depth == 1 {
                g.run_id = run_id;
                g.run_label = label;
                g.run_start = Some(Instant::now());
                g.spans.clear();
                emit(json!({
                    "event": "perf.profile.run_start",
                    "run_id": run_id,
                    "label": label,
                    "ts_unix_ms": unix_ms(),
                }));
            }
        }
        Self { active: true }
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let Ok(mut g) = collector().lock() else {
            return;
        };
        if g.depth == 0 {
            return;
        }
        g.depth -= 1;
        if g.depth != 0 {
            return;
        }
        let run_id = g.run_id;
        let label = g.run_label;
        let wall_us = g
            .run_start
            .take()
            .map(|t| t.elapsed().as_micros() as u64)
            .unwrap_or(0);
        let mut names: Vec<&'static str> = g.spans.keys().copied().collect();
        names.sort_unstable();
        for name in names {
            let Some(acc) = g.spans.get(name) else {
                continue;
            };
            let summary = summarize(acc);
            emit(json!({
                "event": "perf.profile.span_summary",
                "run_id": run_id,
                "span": name,
                "cumulative_us": summary.cumulative_us,
                "count": summary.count,
                "p50_us": summary.p50_us,
                "p95_us": summary.p95_us,
                "category": acc.category,
                "evidence": acc.evidence,
            }));
        }
        g.spans.clear();
        emit(json!({
            "event": "perf.profile.run_complete",
            "run_id": run_id,
            "label": label,
            "wall_us": wall_us,
            "ts_unix_ms": unix_ms(),
        }));
    }
}

/// RAII wall-clock span. Records one sample on drop.
pub struct Span {
    name: &'static str,
    category: &'static str,
    evidence: &'static str,
    start: Option<Instant>,
}

impl Span {
    #[inline]
    pub fn start(name: &'static str, category: &'static str, evidence: &'static str) -> Self {
        if !enabled() {
            return Self {
                name,
                category,
                evidence,
                start: None,
            };
        }
        Self {
            name,
            category,
            evidence,
            start: Some(Instant::now()),
        }
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        let Some(start) = self.start.take() else {
            return;
        };
        let us = start.elapsed().as_micros() as u64;
        record_sample(self.name, self.category, self.evidence, us, true);
    }
}

/// Record one sample without RAII (for parallel hot loops).
#[inline]
pub fn record_sample(
    name: &'static str,
    category: &'static str,
    evidence: &'static str,
    us: u64,
    emit_sample_event: bool,
) {
    if !enabled() {
        return;
    }
    if let Ok(mut g) = collector().lock() {
        let acc = g.spans.entry(name).or_insert_with(|| SpanAcc {
            category,
            evidence,
            samples_us: Vec::new(),
            cumulative_us: 0,
        });
        acc.cumulative_us = acc.cumulative_us.saturating_add(u128::from(us));
        if acc.samples_us.len() < MAX_SAMPLES_PER_SPAN {
            acc.samples_us.push(us);
        }
    }
    if emit_sample_event {
        emit(json!({
            "event": "perf.profile.sample_collected",
            "span": name,
            "us": us,
            "category": category,
        }));
    }
}

struct Summary {
    cumulative_us: u64,
    count: usize,
    p50_us: u64,
    p95_us: u64,
}

fn summarize(acc: &SpanAcc) -> Summary {
    let mut samples = acc.samples_us.clone();
    samples.sort_unstable();
    let count = if samples.is_empty() {
        if acc.cumulative_us > 0 {
            1
        } else {
            0
        }
    } else {
        samples.len()
    };
    let p50_us = percentile_us(&samples, 50);
    let p95_us = percentile_us(&samples, 95);
    Summary {
        cumulative_us: u64::try_from(acc.cumulative_us.min(u128::from(u64::MAX))).unwrap_or(u64::MAX),
        count,
        p50_us,
        p95_us,
    }
}

fn percentile_us(sorted: &[u64], pct: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let n = sorted.len();
    // Nearest-rank on [0, n): idx = floor((n-1) * pct / 100).
    let idx = ((n - 1).saturating_mul(pct.min(100))) / 100;
    sorted[idx]
}

fn unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn emit(value: serde_json::Value) {
    let mut line = value.to_string();
    line.push('\n');
    if let Ok(path) = std::env::var("ASGREP_PERF_PROFILE_PATH") {
        if !path.is_empty() {
            if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
                let _ = f.write_all(line.as_bytes());
                return;
            }
        }
    }
    let mut err = std::io::stderr().lock();
    let _ = err.write_all(line.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_handles_empty_and_single() {
        assert_eq!(percentile_us(&[], 50), 0);
        assert_eq!(percentile_us(&[10], 50), 10);
        assert_eq!(percentile_us(&[10], 95), 10);
    }

    #[test]
    fn percentile_p95_near_tail() {
        let s: Vec<u64> = (1..=100).collect();
        assert_eq!(percentile_us(&s, 50), 50);
        assert_eq!(percentile_us(&s, 95), 95);
    }

    #[test]
    fn summarize_accumulates() {
        let acc = SpanAcc {
            category: "index",
            evidence: "test",
            samples_us: vec![10, 20, 30, 40],
            cumulative_us: 100,
        };
        let s = summarize(&acc);
        assert_eq!(s.count, 4);
        assert_eq!(s.cumulative_us, 100);
        assert_eq!(s.p50_us, 20);
    }

    #[test]
    fn disabled_span_is_noop() {
        // When flag is unset in the test process, Span/Run must not panic.
        // Do not force ENABLED: other tests may share the process.
        let _s = Span::start("test_span", "test", "unit");
        let _r = Run::start("test_run");
    }
}

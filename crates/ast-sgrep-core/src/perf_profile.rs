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
use std::cell::Cell;
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

thread_local! {
    static CURRENT_RUN_ID: Cell<Option<u64>> = const { Cell::new(None) };
}

struct SpanAcc {
    category: &'static str,
    evidence: &'static str,
    samples_us: Vec<u64>,
    sample_count: u64,
    cumulative_us: u128,
}

struct RunAcc {
    run_label: &'static str,
    run_start: Instant,
    spans: HashMap<&'static str, SpanAcc>,
}

struct Collector {
    runs: HashMap<u64, RunAcc>,
}

impl Collector {
    fn new() -> Self {
        Self {
            runs: HashMap::new(),
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
    run_id: Option<u64>,
    previous_run_id: Option<u64>,
}

impl Run {
    pub fn start(label: &'static str) -> Self {
        if !enabled() {
            return Self {
                run_id: None,
                previous_run_id: None,
            };
        }
        let run_id = RUN_SEQ.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut g) = collector().lock() {
            g.runs.insert(
                run_id,
                RunAcc {
                    run_label: label,
                    run_start: Instant::now(),
                    spans: HashMap::new(),
                },
            );
            let previous_run_id = CURRENT_RUN_ID.with(|current| current.replace(Some(run_id)));
            emit(json!({
                "event": "perf.profile.run_start",
                "run_id": run_id,
                "label": label,
                "ts_unix_ms": unix_ms(),
            }));
            return Self {
                run_id: Some(run_id),
                previous_run_id,
            };
        }
        Self {
            run_id: None,
            previous_run_id: None,
        }
    }

    /// Identifier used to attribute samples collected by worker threads that
    /// do not inherit the initiating thread's profiler context.
    pub(crate) fn id(&self) -> Option<u64> {
        self.run_id
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        let Some(run_id) = self.run_id.take() else {
            return;
        };
        CURRENT_RUN_ID.with(|current| {
            if current.get() == Some(run_id) {
                current.set(self.previous_run_id);
            }
        });
        let Some(mut run) = collector()
            .lock()
            .ok()
            .and_then(|mut collector| collector.runs.remove(&run_id))
        else {
            return;
        };
        let wall_us = run.run_start.elapsed().as_micros() as u64;
        let mut names: Vec<&'static str> = run.spans.keys().copied().collect();
        names.sort_unstable();
        for name in names {
            let Some(acc) = run.spans.get(name) else {
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
        run.spans.clear();
        emit(json!({
            "event": "perf.profile.run_complete",
            "run_id": run_id,
            "label": run.run_label,
            "wall_us": wall_us,
            "ts_unix_ms": unix_ms(),
        }));
    }
}

/// RAII wall-clock span. Records one sample on drop.
pub struct Span {
    run_id: Option<u64>,
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
                run_id: None,
                name,
                category,
                evidence,
                start: None,
            };
        }
        Self {
            run_id: CURRENT_RUN_ID.with(Cell::get),
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
        record_sample_for(
            self.run_id,
            self.name,
            self.category,
            self.evidence,
            us,
            true,
        );
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
    let run_id = CURRENT_RUN_ID.with(Cell::get);
    record_sample_for(run_id, name, category, evidence, us, emit_sample_event);
}

/// Record a sample for an explicitly captured run. This is used by worker
/// threads, which do not inherit thread-local profiler context.
#[inline]
pub(crate) fn record_sample_for(
    run_id: Option<u64>,
    name: &'static str,
    category: &'static str,
    evidence: &'static str,
    us: u64,
    emit_sample_event: bool,
) {
    let Some(run_id) = run_id else {
        return;
    };
    if let Ok(mut g) = collector().lock() {
        let Some(run) = g.runs.get_mut(&run_id) else {
            return;
        };
        let acc = run.spans.entry(name).or_insert_with(|| SpanAcc {
            category,
            evidence,
            samples_us: Vec::new(),
            sample_count: 0,
            cumulative_us: 0,
        });
        acc.sample_count = acc.sample_count.saturating_add(1);
        acc.cumulative_us = acc.cumulative_us.saturating_add(u128::from(us));
        if acc.samples_us.len() < MAX_SAMPLES_PER_SPAN {
            acc.samples_us.push(us);
        }
    }
    if emit_sample_event {
        emit(json!({
            "event": "perf.profile.sample_collected",
            "run_id": run_id,
            "span": name,
            "us": us,
            "category": category,
        }));
    }
}

struct Summary {
    cumulative_us: u64,
    count: u64,
    p50_us: u64,
    p95_us: u64,
}

fn summarize(acc: &SpanAcc) -> Summary {
    let mut samples = acc.samples_us.clone();
    samples.sort_unstable();
    let p50_us = percentile_us(&samples, 50);
    let p95_us = percentile_us(&samples, 95);
    Summary {
        cumulative_us: u64::try_from(acc.cumulative_us.min(u128::from(u64::MAX)))
            .unwrap_or(u64::MAX),
        count: acc.sample_count,
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

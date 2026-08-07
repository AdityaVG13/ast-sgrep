//! Structured JSON-line logging for high-risk integration tests.
//!
//! Emits one JSON object per line (default: stderr) with fields:
//! `ts`, `suite`, `test`, `phase` (`setup`|`act`|`assert`|`teardown`),
//! `event`, optional `data`, optional `duration_ms`.
//!
//! Logging is diagnostic only -- product tests must not assert on log content.
//! Unit tests of this module may capture via [`TestLogger::with_writer`].

use ast_sgrep_core::{IndexStats, IndexStatus};
use serde::Serialize;
use serde_json::{json, Value};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Test phase tags written into each log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TestPhase {
    Setup,
    Act,
    Assert,
    Teardown,
}

impl TestPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Act => "act",
            Self::Assert => "assert",
            Self::Teardown => "teardown",
        }
    }
}

/// File/symbol (and optional related) counts for [`TestLogger::index_snapshot`].
#[derive(Debug, Clone, Copy, Default)]
pub struct IndexSnapshot {
    pub file_count: usize,
    pub symbol_count: usize,
    pub line_count: Option<usize>,
    pub caller_count: Option<usize>,
    pub files_indexed: Option<usize>,
    pub symbols_extracted: Option<usize>,
    pub callers_extracted: Option<usize>,
}

impl From<&IndexStatus> for IndexSnapshot {
    fn from(s: &IndexStatus) -> Self {
        Self {
            file_count: s.file_count,
            symbol_count: s.symbol_count,
            line_count: Some(s.line_count),
            caller_count: Some(s.caller_count),
            files_indexed: None,
            symbols_extracted: None,
            callers_extracted: None,
        }
    }
}

impl From<&IndexStats> for IndexSnapshot {
    fn from(s: &IndexStats) -> Self {
        Self {
            file_count: s.files_indexed,
            symbol_count: s.symbols_extracted,
            line_count: None,
            caller_count: None,
            files_indexed: Some(s.files_indexed),
            symbols_extracted: Some(s.symbols_extracted),
            callers_extracted: Some(s.callers_extracted),
        }
    }
}

impl IndexSnapshot {
    fn to_data(self) -> Value {
        let mut map = serde_json::Map::new();
        map.insert("file_count".into(), json!(self.file_count));
        map.insert("symbol_count".into(), json!(self.symbol_count));
        if let Some(v) = self.line_count {
            map.insert("line_count".into(), json!(v));
        }
        if let Some(v) = self.caller_count {
            map.insert("caller_count".into(), json!(v));
        }
        if let Some(v) = self.files_indexed {
            map.insert("files_indexed".into(), json!(v));
        }
        if let Some(v) = self.symbols_extracted {
            map.insert("symbols_extracted".into(), json!(v));
        }
        if let Some(v) = self.callers_extracted {
            map.insert("callers_extracted".into(), json!(v));
        }
        Value::Object(map)
    }
}

/// JSON-line test logger. Default sink is stderr; use [`Self::with_writer`] in tests.
pub struct TestLogger {
    suite: String,
    test: String,
    phase: TestPhase,
    test_start: Instant,
    phase_start: Instant,
    out: Arc<Mutex<dyn Write + Send>>,
}

impl TestLogger {
    /// Create a logger that writes JSON lines to stderr.
    pub fn new(suite: impl Into<String>, test: impl Into<String>) -> Self {
        Self::with_writer(suite, test, io::stderr())
    }

    /// Create a logger that writes to an arbitrary [`Write`] (for unit tests).
    pub fn with_writer<W>(suite: impl Into<String>, test: impl Into<String>, writer: W) -> Self
    where
        W: Write + Send + 'static,
    {
        let now = Instant::now();
        Self {
            suite: suite.into(),
            test: test.into(),
            phase: TestPhase::Setup,
            test_start: now,
            phase_start: now,
            out: Arc::new(Mutex::new(writer)),
        }
    }

    /// Mark the start of a named test (resets timing; phase = setup).
    pub fn test_start(&mut self, test: impl Into<String>) {
        self.test = test.into();
        let now = Instant::now();
        self.test_start = now;
        self.phase_start = now;
        self.phase = TestPhase::Setup;
        self.emit("test_start", None, None);
    }

    /// Enter a phase; logs previous phase duration when switching.
    pub fn phase(&mut self, phase: TestPhase) {
        let duration_ms = elapsed_ms(self.phase_start);
        let prev = self.phase;
        self.phase = phase;
        self.phase_start = Instant::now();
        self.emit(
            "phase",
            Some(json!({ "from": prev.as_str(), "to": phase.as_str() })),
            Some(duration_ms),
        );
    }

    /// Log file/symbol counts from [`IndexStatus`], [`IndexStats`], or [`IndexSnapshot`].
    pub fn index_snapshot(&mut self, source: impl Into<IndexSnapshot>) {
        let snap: IndexSnapshot = source.into();
        self.emit("index_snapshot", Some(snap.to_data()), None);
    }

    /// Log an assertion-side observation (match count, query, etc.). Not a hard assert.
    pub fn assert_match(&mut self, event: &str, data: impl Serialize) {
        let data = serde_json::to_value(data).unwrap_or(Value::Null);
        self.emit(event, Some(data), None);
    }

    /// Mark test end with total duration; `ok` is recorded in `data`.
    pub fn test_end(&mut self, ok: bool) {
        let duration_ms = elapsed_ms(self.test_start);
        self.phase = TestPhase::Teardown;
        self.emit("test_end", Some(json!({ "ok": ok })), Some(duration_ms));
    }

    fn emit(&self, event: &str, data: Option<Value>, duration_ms: Option<u64>) {
        let line = json!({
            "ts": ts_millis(),
            "suite": self.suite,
            "test": self.test,
            "phase": self.phase.as_str(),
            "event": event,
            "data": data,
            "duration_ms": duration_ms,
        });
        // Drop null optional fields for cleaner lines.
        let mut obj = line.as_object().cloned().unwrap_or_default();
        if obj.get("data").is_some_and(|v| v.is_null()) {
            obj.remove("data");
        }
        if obj.get("duration_ms").is_some_and(|v| v.is_null()) {
            obj.remove("duration_ms");
        }
        if let Ok(mut w) = self.out.lock() {
            let _ = writeln!(w, "{}", Value::Object(obj));
            let _ = w.flush();
        }
    }
}

/// Start a suite-scoped logger writing to stderr (test name set later via [`TestLogger::test_start`]).
pub fn with_test_logging(suite: impl Into<String>) -> TestLogger {
    TestLogger::new(suite, "")
}

fn ts_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Shared in-memory sink so the logger can hold `dyn Write` while tests read back.
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("buf").write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.lock().expect("buf").flush()
        }
    }

    fn capture_logger(suite: &str, test: &str) -> (TestLogger, Arc<Mutex<Vec<u8>>>) {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let logger = TestLogger::with_writer(suite, test, SharedBuf(Arc::clone(&buf)));
        (logger, buf)
    }

    fn lines_from(buf: &Arc<Mutex<Vec<u8>>>) -> Vec<Value> {
        let raw = buf.lock().expect("buf");
        let s = String::from_utf8_lossy(&raw);
        s.lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str::<Value>(l).expect("valid json line"))
            .collect()
    }

    #[test]
    fn emits_valid_json_lines_with_required_fields() {
        let (mut log, buf) = capture_logger("testkit", "sample");
        log.test_start("sample");
        log.phase(TestPhase::Act);
        log.index_snapshot(IndexSnapshot {
            file_count: 3,
            symbol_count: 12,
            ..Default::default()
        });
        log.phase(TestPhase::Assert);
        log.assert_match(
            "assert_match",
            json!({ "query": "handle_request", "hits": 2 }),
        );
        log.test_end(true);

        let lines = lines_from(&buf);
        assert!(
            lines.len() >= 5,
            "expected several events, got {}",
            lines.len()
        );

        for line in &lines {
            assert!(line["ts"].as_u64().is_some(), "ts millis: {line}");
            assert_eq!(line["suite"], "testkit");
            assert_eq!(line["test"], "sample");
            assert!(line["phase"].as_str().is_some());
            assert!(line["event"].as_str().is_some());
            // Full line must re-parse as a JSON object (already did).
            assert!(line.is_object());
        }

        let start = lines
            .iter()
            .find(|l| l["event"] == "test_start")
            .expect("test_start");
        assert_eq!(start["phase"], "setup");

        let snap = lines
            .iter()
            .find(|l| l["event"] == "index_snapshot")
            .expect("index_snapshot");
        assert_eq!(snap["data"]["file_count"], 3);
        assert_eq!(snap["data"]["symbol_count"], 12);

        let end = lines
            .iter()
            .find(|l| l["event"] == "test_end")
            .expect("test_end");
        assert_eq!(end["data"]["ok"], true);
        assert!(end["duration_ms"].as_u64().is_some());
    }

    #[test]
    fn index_snapshot_from_status_and_stats() {
        let (mut log, buf) = capture_logger("testkit", "snap");
        log.test_start("snap");

        let status = IndexStatus {
            root: "/tmp".into(),
            index_path: "/tmp/index.db".into(),
            file_count: 5,
            line_count: 100,
            symbol_count: 40,
            caller_count: 8,
            import_count: 2,
            semantic_chunk_count: 0,
            embed_backend: None,
            embed_dim: None,
            embed_cache_entries: 0,
            embed_cache_capacity: 0,
            embed_cache_hits: 0,
            embed_cache_misses: 0,
            semantic_ivf_present: false,
            durability: "balanced".into(),
        };
        log.index_snapshot(&status);

        let stats = IndexStats {
            files_indexed: 5,
            files_skipped: 0,
            files_removed: 0,
            files_failed: 0,
            walk_errors: false,
            symbols_extracted: 40,
            callers_extracted: 8,
            imports_extracted: 2,
        };
        log.index_snapshot(&stats);
        log.test_end(true);

        let lines = lines_from(&buf);
        let snaps: Vec<_> = lines
            .iter()
            .filter(|l| l["event"] == "index_snapshot")
            .collect();
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[0]["data"]["file_count"], 5);
        assert_eq!(snaps[0]["data"]["symbol_count"], 40);
        assert_eq!(snaps[0]["data"]["line_count"], 100);
        assert_eq!(snaps[1]["data"]["files_indexed"], 5);
        assert_eq!(snaps[1]["data"]["symbols_extracted"], 40);
    }

    #[test]
    fn with_test_logging_suite_name() {
        // Smoke: constructor path (stderr). Does not assert on stderr contents.
        let mut log = with_test_logging("suite_smoke");
        log.test_start("noop");
        log.phase(TestPhase::Teardown);
        log.test_end(true);
    }

    #[test]
    fn writer_receives_newline_delimited_json() {
        let buf = Arc::new(Mutex::new(Vec::new()));
        {
            let mut log = TestLogger::with_writer("s", "t", SharedBuf(Arc::clone(&buf)));
            log.test_start("t");
            log.test_end(false);
        }
        let raw = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(raw.ends_with('\n'));
        for line in raw.lines() {
            let v: Value = serde_json::from_str(line).unwrap();
            assert!(v.get("event").is_some());
        }
    }
}

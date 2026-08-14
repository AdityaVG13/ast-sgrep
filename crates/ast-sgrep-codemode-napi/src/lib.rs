//! In-process Node-API bindings for ast-sgrep Code Mode.
//!
//! Same retrieval path as MCP (links `ast-sgrep-codemode` → core). Pi loads this
//! `.node` addon and never needs to spawn the `asgrep` CLI for Code Mode work.
//!
//! # Root policy (R-CM-ROOT-POLICY option A)
//!
//! Tool `root` overrides are jailed under the session workspace root — the same
//! `starts_with` / contained-in-root contract as MCP `sandbox_root`. NAPI does
//! not bypass this: every `Session::call` / `batch` goes through
//! `CodeModeSession::root_arg`.
//!
//! # Cancel (R-CM-SOFT-TIMEOUT-ORPHAN)
//!
//! Soft-timeout abort must not leave waiters parked on the session mutex or a
//! fail-fast `session is busy` gate. Tasks poll `try_lock` so a cancelled
//! waiter returns `operation cancelled` without taking the session. A call that
//! already holds the mutex may finish its current `session.call`.

#![deny(clippy::all)]

use ast_sgrep_codemode::{
    CodeModeSession, SessionConfig, MAX_BATCH_CALLS, MAX_BATCH_ERROR_BYTES, MAX_BATCH_ID_BYTES,
    MAX_BATCH_RESPONSE_BYTES, MAX_BATCH_TOOL_BYTES, MAX_BATCH_VALUE_BYTES,
};
use napi::bindgen_prelude::*;
use napi::{ScopedTask, Task};
use napi_derive::napi;
use serde::Serialize;
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

fn map_err(err: impl std::fmt::Display) -> Error {
    Error::from_reason(err.to_string())
}

#[napi(object)]
pub struct JsSessionConfig {
    pub root: Option<String>,
    pub index_path: Option<String>,
    pub limit: Option<u32>,
    pub use_embed: Option<bool>,
}

impl JsSessionConfig {
    fn into_rust(self) -> SessionConfig {
        let mut cfg = SessionConfig::default();
        if let Some(root) = self.root {
            cfg.root = PathBuf::from(root);
        }
        if let Some(index_path) = self.index_path {
            cfg.index_path = Some(PathBuf::from(index_path));
        }
        if let Some(limit) = self.limit {
            cfg.limit = (limit as usize).clamp(1, 500);
        }
        if let Some(use_embed) = self.use_embed {
            cfg.use_embed = use_embed;
        }
        cfg
    }
}

#[napi(object)]
pub struct JsBatchCall {
    pub id: String,
    pub tool: String,
    pub args: Option<Value>,
}

#[napi(object)]
#[derive(Serialize)]
pub struct JsBatchCallResult {
    pub id: String,
    pub ok: bool,
    pub value: Option<Value>,
    pub error: Option<String>,
}

#[napi(object)]
pub struct JsBatchResponse {
    pub all_ok: bool,
    pub results: Vec<JsBatchCallResult>,
    pub call_count: u32,
    pub wall_ms: u32,
    pub mode: String,
}

/// Warm in-process Code Mode session (one Searcher, reused across calls).
#[napi]
pub struct Session {
    inner: Arc<Mutex<CodeModeSession>>,
    call_count: Arc<AtomicU32>,
    root: String,
}

fn cancelled_error() -> Error {
    Error::from_reason("operation cancelled")
}

fn watch_cancel(signal: Option<&AbortSignal>, cancelled: &Arc<AtomicBool>) {
    let Some(signal) = signal else {
        return;
    };
    let cancelled = Arc::clone(cancelled);
    signal.on_abort(move || cancelled.store(true, Ordering::Release));
}

/// Abortable mutex acquire. A blocking `lock()` would keep cancelled waiters
/// parked on a pooled session after Code Mode's soft wall fires.
fn lock_session<'a>(
    inner: &'a Mutex<CodeModeSession>,
    cancelled: &AtomicBool,
) -> Result<MutexGuard<'a, CodeModeSession>> {
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(cancelled_error());
        }
        match inner.try_lock() {
            Ok(guard) => {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancelled_error());
                }
                return Ok(guard);
            }
            Err(TryLockError::WouldBlock) => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(TryLockError::Poisoned(_)) => {
                return Err(Error::from_reason("session lock poisoned"));
            }
        }
    }
}

pub struct SessionCallTask {
    inner: Arc<Mutex<CodeModeSession>>,
    call_count: Arc<AtomicU32>,
    tool: String,
    args: Value,
    cancelled: Arc<AtomicBool>,
}

#[napi]
impl<'task> ScopedTask<'task> for SessionCallTask {
    type Output = Value;
    type JsValue = Unknown<'task>;

    fn compute(&mut self) -> Result<Self::Output> {
        let mut session = lock_session(&self.inner, &self.cancelled)?;
        let result = session.call(&self.tool, std::mem::take(&mut self.args));
        // Saturate at u32::MAX; session cap 10_000 makes wrap unreachable (pb2w).
        self.call_count.store(
            session.call_count().min(u32::MAX as usize) as u32,
            Ordering::Relaxed,
        );
        result.map_err(map_err)
    }

    fn resolve(&mut self, env: &'task Env, output: Self::Output) -> Result<Self::JsValue> {
        env.to_js_value(&output)
    }
}

pub struct SessionBatchTask {
    inner: Arc<Mutex<CodeModeSession>>,
    call_count: Arc<AtomicU32>,
    calls: Vec<JsBatchCall>,
    cancelled: Arc<AtomicBool>,
}

#[napi]
impl Task for SessionBatchTask {
    type Output = JsBatchResponse;
    type JsValue = JsBatchResponse;

    fn compute(&mut self) -> Result<Self::Output> {
        let started = Instant::now();
        let mut session = lock_session(&self.inner, &self.cancelled)?;
        let mut all_ok = true;
        let mut results = Vec::with_capacity(self.calls.len());
        let mut response_bytes = MAX_BATCH_RESPONSE_BYTES - MAX_BATCH_VALUE_BYTES;
        for call in std::mem::take(&mut self.calls) {
            if self.cancelled.load(Ordering::Acquire) {
                return Err(cancelled_error());
            }
            let result = match session.call(
                &call.tool,
                call.args.unwrap_or(Value::Object(Default::default())),
            ) {
                Ok(value) => JsBatchCallResult {
                    id: call.id,
                    ok: true,
                    value: Some(value),
                    error: None,
                },
                Err(error) => JsBatchCallResult {
                    id: call.id,
                    ok: false,
                    value: None,
                    error: Some(bound_error(error.to_string())),
                },
            };
            let result = enforce_result_budget(result, &mut response_bytes)?;
            all_ok &= result.ok;
            results.push(result);
        }
        self.call_count.store(
            session.call_count().min(u32::MAX as usize) as u32,
            Ordering::Relaxed,
        );
        Ok(JsBatchResponse {
            all_ok,
            results,
            call_count: session.call_count().min(u32::MAX as usize) as u32,
            wall_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
            mode: "serial-napi".to_string(),
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(output)
    }
}

fn validate_session_batch(calls: &[JsBatchCall]) -> Result<()> {
    if calls.is_empty() {
        return Err(Error::from_reason("batch.calls must be non-empty"));
    }
    if calls.len() > MAX_BATCH_CALLS {
        return Err(Error::from_reason(format!(
            "batch.calls exceeds max {MAX_BATCH_CALLS}"
        )));
    }
    for call in calls {
        if call.id.is_empty() {
            return Err(Error::from_reason("batch call id must be non-empty"));
        }
        if call.id.len() > MAX_BATCH_ID_BYTES {
            return Err(Error::from_reason(format!(
                "batch call id exceeds {MAX_BATCH_ID_BYTES} bytes"
            )));
        }
        if call.tool.is_empty() {
            return Err(Error::from_reason("batch call tool must be non-empty"));
        }
        if call.tool.len() > MAX_BATCH_TOOL_BYTES {
            return Err(Error::from_reason(format!(
                "batch call tool exceeds {MAX_BATCH_TOOL_BYTES} bytes"
            )));
        }
    }
    Ok(())
}

#[derive(Default)]
struct CountingWriter(usize);

impl Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn encoded_len(value: &impl Serialize) -> std::result::Result<usize, serde_json::Error> {
    let mut writer = CountingWriter::default();
    serde_json::to_writer(&mut writer, value)?;
    Ok(writer.0)
}

fn enforce_result_budget(
    mut result: JsBatchCallResult,
    response_bytes: &mut usize,
) -> Result<JsBatchCallResult> {
    let bytes = encoded_len(&result).map_err(map_err)?;
    if response_bytes.saturating_add(bytes) <= MAX_BATCH_VALUE_BYTES {
        *response_bytes += bytes;
        return Ok(result);
    }
    result.ok = false;
    result.value = None;
    result.error = Some(format!(
        "codemode batch response exceeds {MAX_BATCH_RESPONSE_BYTES} bytes"
    ));
    *response_bytes = response_bytes.saturating_add(encoded_len(&result).map_err(map_err)?);
    Ok(result)
}

fn bound_error(mut error: String) -> String {
    if error.len() <= MAX_BATCH_ERROR_BYTES {
        return error;
    }
    let mut end = MAX_BATCH_ERROR_BYTES.saturating_sub(3);
    while end > 0 && !error.is_char_boundary(end) {
        end -= 1;
    }
    error.truncate(end);
    error.push('…');
    error
}

#[napi]
impl Session {
    #[napi(constructor)]
    pub fn new(config: Option<JsSessionConfig>) -> Result<Self> {
        let cfg = config.map(JsSessionConfig::into_rust).unwrap_or_default();
        let root = cfg.root.display().to_string();
        let mut session = CodeModeSession::new(cfg);
        // Pi sessions run many tool calls; match sticky serve budget.
        session.max_calls = 10_000;
        Ok(Self {
            inner: Arc::new(Mutex::new(session)),
            call_count: Arc::new(AtomicU32::new(0)),
            root,
        })
    }

    /// Dispatch one catalog tool on libuv's worker pool, never Node's event loop.
    #[napi]
    pub fn call(
        &self,
        tool: String,
        args: Option<Value>,
        signal: Option<AbortSignal>,
    ) -> Result<AsyncTask<SessionCallTask>> {
        let cancelled = Arc::new(AtomicBool::new(false));
        watch_cancel(signal.as_ref(), &cancelled);
        Ok(AsyncTask::with_optional_signal(
            SessionCallTask {
                inner: Arc::clone(&self.inner),
                call_count: Arc::clone(&self.call_count),
                tool,
                args: args.unwrap_or(Value::Object(Default::default())),
                cancelled,
            },
            signal,
        ))
    }

    /// Dispatch one serial warm batch in a single worker-pool task.
    #[napi]
    pub fn batch(
        &self,
        calls: Vec<JsBatchCall>,
        signal: Option<AbortSignal>,
    ) -> Result<AsyncTask<SessionBatchTask>> {
        validate_session_batch(&calls)?;
        let cancelled = Arc::new(AtomicBool::new(false));
        watch_cancel(signal.as_ref(), &cancelled);
        Ok(AsyncTask::with_optional_signal(
            SessionBatchTask {
                inner: Arc::clone(&self.inner),
                call_count: Arc::clone(&self.call_count),
                calls,
                cancelled,
            },
            signal,
        ))
    }

    /// How many tool calls this session has executed.
    #[napi(getter)]
    pub fn call_count(&self) -> u32 {
        self.call_count.load(Ordering::Relaxed)
    }

    /// Project root for this session.
    #[napi(getter)]
    pub fn root(&self) -> String {
        self.root.clone()
    }
}

/// Addon identity — Pi verifies this matches the extension contract.
#[napi]
pub fn binding_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// True when this addon was loaded (smoke for doctor / tests).
#[napi]
pub fn is_native() -> bool {
    true
}

/// Binding contract marker: all index/search work resolves through Promise-returning tasks.
#[napi]
pub fn async_api_version() -> u32 {
    1
}

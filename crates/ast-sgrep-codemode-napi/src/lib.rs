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
//! waiter returns `operation cancelled` without taking the session. Read/search
//! calls that already hold the mutex may finish. `index_repo` polls the abort
//! flag during walk/prepare and returns `operation cancelled` without committing.

#![deny(clippy::all)]

use ast_sgrep_codemode::{
    batch::{bound_error, validate_batch},
    session::encoded_len,
    CodeModeSession, SessionConfig, ToolName, MAX_BATCH_RESPONSE_BYTES, MAX_BATCH_VALUE_BYTES,
};
use napi::bindgen_prelude::*;
use napi::{JsValue, ScopedTask, Task};
use napi_derive::napi;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

fn map_err(err: impl std::fmt::Display) -> Error {
    Error::from_reason(err.to_string())
}

/// Sync `call_now` admission: exactly the bounded metadata and symbol lookups.
/// Membership keys on stable `ToolName` variants while every spelling comes
/// from codemode's `TOOL_RECORDS` table (`parse`/`as_str`), so a catalog
/// rename flows without a lockstep edit here; a newly added tool defaults to
/// the worker pool (fail-closed for the JS thread) until deliberately listed.
/// The gate stays literal-canonical on purpose — aliases of fast tools still
/// route to `call()` (pinned by `ffi_call_now::fast_slow_partition_is_exact`).
/// Search and other potentially expensive tools stay on libuv's worker pool.
fn is_fast_lookup(tool: &str) -> bool {
    ToolName::parse(tool).is_some_and(|name| {
        matches!(
            name,
            ToolName::Find
                | ToolName::Read
                | ToolName::Defs
                | ToolName::Callers
                | ToolName::Imports
                | ToolName::IndexStatus
                | ToolName::CatalogSearch
                | ToolName::CatalogDescribe
        ) && name.as_str() == tool
    })
}

/// Missing-args convention, single source (was one `unwrap_or` per call site).
fn default_args() -> Value {
    Value::Object(Default::default())
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
///
/// The executed-call total is served from the session itself
/// (`CodeModeSession::call_count`); no mirrored counter is kept here, so a
/// future tool path cannot leave a copy stale.
#[napi]
pub struct Session {
    inner: Arc<Mutex<CodeModeSession>>,
    root: String,
}

fn cancelled_error() -> Error {
    Error::from_reason("operation cancelled")
}

struct SessionCancelGuard<'a> {
    session: &'a mut CodeModeSession,
}

impl<'a> SessionCancelGuard<'a> {
    fn arm(session: &'a mut CodeModeSession, cancel: &Arc<AtomicBool>) -> Self {
        session.set_cancel(Some(Arc::clone(cancel)));
        Self { session }
    }
}

impl Drop for SessionCancelGuard<'_> {
    fn drop(&mut self) {
        self.session.set_cancel(None);
    }
}

fn watch_cancel(
    signal: Option<Object<'_>>,
    cancelled: &Arc<AtomicBool>,
) -> Result<Option<AbortSignal>> {
    let Some(signal) = signal else {
        return Ok(None);
    };
    // napi's AbortSignal conversion subscribes to future events only. An
    // already-aborted signal will never emit another event, so reject it in
    // the task before any session lock or catalog call is attempted.
    if signal.get::<bool>("aborted")? == Some(true) {
        cancelled.store(true, Ordering::Release);
        return Ok(None);
    }
    // Use napi's safe value conversion; no extra Env parameter or temporary
    // JS object is needed, and Rust callers without a signal need no Node env.
    let signal = AbortSignal::from_unknown(signal.to_unknown())?;
    let cancelled = Arc::clone(cancelled);
    signal.on_abort(move || cancelled.store(true, Ordering::Release));
    Ok(Some(signal))
}

/// Abortable mutex acquire. A blocking `lock()` would keep cancelled waiters
/// parked on a pooled session after Code Mode's soft wall fires.
/// Poison policy for the session mutex (applies to every lock site in this
/// module): a poisoned lock is recovered via `into_inner()`, never wedged. A
/// task that panics while holding the session must not permanently brick the
/// JS host, so lock sites read through the poison and keep serving.
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
            // Poison policy above: recover the session instead of wedging.
            Err(TryLockError::Poisoned(err)) => {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancelled_error());
                }
                return Ok(err.into_inner());
            }
        }
    }
}

/// Non-blocking acquire for [`Session::call_now`]. The JS thread must never
/// park on a worker-pool task's session hold, so contention reports
/// "session is busy" and the host falls back to `call`. Poison recovers per
/// the policy above (a poisoned session is never misreported as busy).
fn try_lock_session<'a>(
    inner: &'a Mutex<CodeModeSession>,
) -> Result<MutexGuard<'a, CodeModeSession>> {
    match inner.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::Poisoned(err)) => Ok(err.into_inner()),
        Err(TryLockError::WouldBlock) => Err(Error::from_reason("session is busy")),
    }
}

pub struct SessionCallTask {
    inner: Arc<Mutex<CodeModeSession>>,
    tool: String,
    args: Value,
    cancelled: Arc<AtomicBool>,
}

#[napi]
impl<'task> ScopedTask<'task> for SessionCallTask {
    type Output = Value;
    type JsValue = Unknown<'task>;

    fn compute(&mut self) -> Result<Self::Output> {
        let mut locked = lock_session(&self.inner, &self.cancelled)?;
        let session = SessionCancelGuard::arm(&mut locked, &self.cancelled);
        let result = session
            .session
            .call(&self.tool, std::mem::take(&mut self.args));
        result.map_err(map_err)
    }

    fn resolve(&mut self, env: &'task Env, output: Self::Output) -> Result<Self::JsValue> {
        env.to_js_value(&output)
    }
}

pub struct SessionBatchTask {
    inner: Arc<Mutex<CodeModeSession>>,
    calls: Vec<JsBatchCall>,
    cancelled: Arc<AtomicBool>,
}

#[napi]
impl Task for SessionBatchTask {
    type Output = JsBatchResponse;
    type JsValue = JsBatchResponse;

    fn compute(&mut self) -> Result<Self::Output> {
        let started = Instant::now();
        let mut locked = lock_session(&self.inner, &self.cancelled)?;
        let session = SessionCancelGuard::arm(&mut locked, &self.cancelled);
        let mut all_ok = true;
        let mut results = Vec::with_capacity(self.calls.len());
        let mut response_bytes = BATCH_ENVELOPE_RESERVE_BYTES;
        for call in std::mem::take(&mut self.calls) {
            if self.cancelled.load(Ordering::Acquire) {
                return Err(cancelled_error());
            }
            let result = match session.session.call(&call.tool, call.args.unwrap_or(default_args())) {
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
        Ok(JsBatchResponse {
            all_ok,
            results,
            call_count: session.session.call_count().min(u32::MAX as usize) as u32,
            wall_ms: started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32,
            mode: "serial-napi".to_string(),
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(output)
    }
}

/// Batch envelope reserve, derived exactly as codemode defines it (`batch.rs`:
/// `MAX_BATCH_VALUE_BYTES = MAX_BATCH_RESPONSE_BYTES -
/// BATCH_ENVELOPE_RESERVE_BYTES`). Retunes of either constant flow here with
/// no lockstep edit; never re-derive the subtraction inline.
const BATCH_ENVELOPE_RESERVE_BYTES: usize = MAX_BATCH_RESPONSE_BYTES - MAX_BATCH_VALUE_BYTES;

/// One per-result budget step of the napi batch wave. Public so the bilateral
/// budget suite can pin parity with codemode's `enforce_batch_response_budget`
/// without a live libuv task. See `tests/codemode/ffi_batch_budget.rs`.
pub fn enforce_result_budget(
    mut result: JsBatchCallResult,
    response_bytes: &mut usize,
) -> Result<JsBatchCallResult> {
    let bytes = encoded_len(&result).map_err(map_err)?;
    // Same acceptance rule as codemode `enforce_batch_response_budget`
    // (`batch.rs`): one spare byte per result for the envelope separator.
    if response_bytes
        .saturating_add(bytes)
        .saturating_add(1)
        <= MAX_BATCH_VALUE_BYTES
    {
        *response_bytes += bytes;
        return Ok(result);
    }
    result.ok = false;
    result.value = None;
    result.error = Some(format!(
        "codemode batch response exceeds {MAX_BATCH_RESPONSE_BYTES} bytes"
    ));
    *response_bytes = response_bytes
        .saturating_add(encoded_len(&result).map_err(map_err)?)
        .saturating_add(1);
    Ok(result)
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
            root,
        })
    }

    /// Dispatch one catalog tool on libuv's worker pool, never Node's event loop.
    #[napi]
    pub fn call(
        &self,
        tool: String,
        args: Option<Value>,
        signal: Option<Object<'_>>,
    ) -> Result<AsyncTask<SessionCallTask>> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = watch_cancel(signal, &cancelled)?;
        Ok(AsyncTask::with_optional_signal(
            SessionCallTask {
                inner: Arc::clone(&self.inner),
                tool,
                args: args.unwrap_or(default_args()),
                cancelled,
            },
            signal,
        ))
    }

    /// Bounded metadata or symbol lookup on the JS thread.
    ///
    /// Search cache hits may also run here so sticky repeats skip libuv.
    /// Unique search, index, semantic, and chain stay on [`Self::call`].
    /// Returns "session is busy" when a worker-pool task already holds the
    /// session so the JS host can fall back to `call`.
    #[napi]
    pub fn call_now(&self, tool: String, args: Option<Value>) -> Result<Value> {
        let args = args.unwrap_or(default_args());
        if tool == ToolName::Search.as_str() {
            let mut session = try_lock_session(&self.inner)?;
            let Some(value) = session.take_cached_search(&args).map_err(map_err)? else {
                // Unique search stays on `call()` / libuv. Return null so the
                // JS host can fall through without throwing (V8 exception tax
                // on every unique Pi search).
                return Ok(Value::Null);
            };
            return Ok(value);
        }
        if !is_fast_lookup(&tool) {
            return Err(Error::from_reason(
                "callNow is only for bounded metadata/symbol lookups; use call() for search/index/semantic/chain",
            ));
        }
        let mut session = try_lock_session(&self.inner)?;
        let result = session.call(&tool, args);
        result.map_err(map_err)
    }

    /// Dispatch one serial warm batch in a single worker-pool task.
    #[napi]
    pub fn batch(
        &self,
        calls: Vec<JsBatchCall>,
        signal: Option<Object<'_>>,
    ) -> Result<AsyncTask<SessionBatchTask>> {
        validate_batch(
            calls
                .iter()
                .map(|call| (call.id.as_str(), call.tool.as_str())),
        )
        .map_err(map_err)?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let signal = watch_cancel(signal, &cancelled)?;
        Ok(AsyncTask::with_optional_signal(
            SessionBatchTask {
                inner: Arc::clone(&self.inner),
                calls,
                cancelled,
            },
            signal,
        ))
    }

    /// How many tool calls this session has executed, served from the
    /// session itself (single source of truth; saturates at u32::MAX).
    #[napi(getter)]
    pub fn call_count(&self) -> u32 {
        // Poison policy stated on `lock_session`: this getter is infallible,
        // so it recovers the count through the poison instead of wedging
        // (previously it silently reported 0).
        let session = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        session.call_count().min(u32::MAX as usize) as u32
    }

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

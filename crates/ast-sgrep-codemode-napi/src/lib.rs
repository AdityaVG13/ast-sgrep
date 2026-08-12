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

#![deny(clippy::all)]

use ast_sgrep_codemode::{
    run_batch, BatchCall, BatchRequest, CodeModeSession, ParallelMode, SessionConfig, MAX_BATCH_CALLS,
};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;

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
pub struct JsBatchRequest {
    pub root: Option<String>,
    pub index_path: Option<String>,
    pub use_embed: Option<bool>,
    pub limit: Option<u32>,
    /// `serial` | `parallel` | `auto` (default auto).
    pub parallel_mode: Option<String>,
    pub calls: Vec<JsBatchCall>,
}

#[napi(object)]
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
    inner: Mutex<CodeModeSession>,
}

#[napi]
impl Session {
    #[napi(constructor)]
    pub fn new(config: Option<JsSessionConfig>) -> Result<Self> {
        let cfg = config.map(JsSessionConfig::into_rust).unwrap_or_default();
        let mut session = CodeModeSession::new(cfg);
        // Pi sessions run many tool calls; match sticky serve budget.
        session.max_calls = 10_000;
        Ok(Self {
            inner: Mutex::new(session),
        })
    }

    /// Dispatch one catalog tool by name with JSON args.
    #[napi]
    pub fn call(&self, tool: String, args: Option<Value>) -> Result<Value> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| Error::from_reason("session lock poisoned"))?;
        guard
            .call(&tool, args.unwrap_or(Value::Object(Default::default())))
            .map_err(map_err)
    }

    /// How many tool calls this session has executed.
    #[napi(getter)]
    pub fn call_count(&self) -> Result<u32> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| Error::from_reason("session lock poisoned"))?;
        Ok(guard.call_count() as u32)
    }

    /// Project root for this session.
    #[napi(getter)]
    pub fn root(&self) -> Result<String> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| Error::from_reason("session lock poisoned"))?;
        Ok(guard.config().root.display().to_string())
    }
}

/// One-shot batch (serial warm by default) — no process spawn.
#[napi]
pub fn batch(request: JsBatchRequest) -> Result<JsBatchResponse> {
    if request.calls.len() > MAX_BATCH_CALLS {
        return Err(Error::from_reason(format!(
            "batch.calls exceeds max {MAX_BATCH_CALLS}"
        )));
    }
    let parallel_mode = match request.parallel_mode.as_deref() {
        Some("serial") => Some(ParallelMode::Serial),
        Some("parallel") => Some(ParallelMode::Parallel),
        Some("auto") | None => Some(ParallelMode::Auto),
        Some(other) => {
            return Err(Error::from_reason(format!(
                "unknown parallel_mode: {other}"
            )));
        }
    };
    let rust_req = BatchRequest {
        root: request.root.map(PathBuf::from),
        index_path: request.index_path.map(PathBuf::from),
        use_embed: request.use_embed,
        limit: request.limit.map(|n| n as usize),
        parallel: None,
        parallel_mode,
        calls: request
            .calls
            .into_iter()
            .map(|c| BatchCall {
                id: c.id,
                tool: c.tool,
                args: c.args.unwrap_or(Value::Object(Default::default())),
            })
            .collect(),
    };
    let mut config = SessionConfig::default();
    if let Some(root) = &rust_req.root {
        config.root = root.clone();
    }
    if rust_req.index_path.is_some() {
        config.index_path = rust_req.index_path.clone();
    }
    if let Some(use_embed) = rust_req.use_embed {
        config.use_embed = use_embed;
    }
    if let Some(limit) = rust_req.limit {
        config.limit = limit.clamp(1, 500);
    }
    let response = run_batch(config, &rust_req).map_err(map_err)?;
    Ok(JsBatchResponse {
        all_ok: response.all_ok,
        call_count: response.call_count as u32,
        wall_ms: response.wall_ms.min(u128::from(u32::MAX)) as u32,
        mode: response.mode.to_string(),
        results: response
            .results
            .into_iter()
            .map(|r| JsBatchCallResult {
                id: r.id,
                ok: r.ok,
                value: r.value,
                error: r.error,
            })
            .collect(),
    })
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

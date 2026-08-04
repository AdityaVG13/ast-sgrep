//! Batch + sticky-serve execution for Code Mode.
//!
//! Default mode is **serial warm** (one Searcher reused): for typical capsule
//! queries, SQLite open dominates, so N parallel opens are slower than N
//! sequential searches on one connection (Amdahl).

use crate::catalog::tool_catalog;
use crate::session::{CodeModeSession, SessionConfig};
use crate::tools::CallError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, Write};
use std::time::Instant;

/// Maximum tool calls in one batch wave (keeps one process bounded).
pub const MAX_BATCH_CALLS: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchCall {
    pub id: String,
    pub tool: String,
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchCallResult {
    pub id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResponse {
    /// True when every per-call result succeeded.
    pub all_ok: bool,
    pub results: Vec<BatchCallResult>,
    pub call_count: usize,
    pub wall_ms: u128,
    /// `serial` = shared warm Searcher (default); `parallel` = one session per call.
    pub mode: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelMode {
    /// Always one Searcher, sequential calls (fastest for cheap queries).
    Serial,
    /// One Searcher per call on rayon (only when all tools are read-only).
    Parallel,
    /// Serial unless N>=4 read-only calls (heuristic).
    Auto,
}

impl Default for ParallelMode {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchRequest {
    #[serde(default)]
    pub root: Option<std::path::PathBuf>,
    #[serde(default)]
    pub index_path: Option<std::path::PathBuf>,
    #[serde(default)]
    pub use_embed: Option<bool>,
    #[serde(default)]
    pub limit: Option<usize>,
    /// Legacy bool: true → Parallel, false → Serial. Prefer `parallel_mode`.
    #[serde(default)]
    pub parallel: Option<bool>,
    #[serde(default)]
    pub parallel_mode: Option<ParallelMode>,
    pub calls: Vec<BatchCall>,
}

/// NDJSON sticky-worker request line.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServeRequest {
    Call {
        id: String,
        tool: String,
        #[serde(default)]
        args: Value,
    },
    Batch {
        id: String,
        calls: Vec<BatchCall>,
        #[serde(default)]
        parallel_mode: Option<ParallelMode>,
    },
    End,
}

/// NDJSON sticky-worker response line.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServeResponse {
    Result {
        id: String,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    BatchResult {
        id: String,
        all_ok: bool,
        results: Vec<BatchCallResult>,
        wall_ms: u128,
        mode: String,
    },
    Bye,
    Error {
        id: Option<String>,
        error: String,
    },
}

fn is_read_only(tool: &str) -> bool {
    tool_catalog()
        .into_iter()
        .find(|t| t.name == tool)
        .map(|t| t.read_only)
        .unwrap_or(false)
}

fn resolve_mode(request: &BatchRequest) -> ParallelMode {
    if let Some(mode) = request.parallel_mode {
        return mode;
    }
    match request.parallel {
        Some(true) => ParallelMode::Parallel,
        Some(false) => ParallelMode::Serial,
        None => ParallelMode::Auto,
    }
}

fn choose_parallel(mode: ParallelMode, calls: &[BatchCall]) -> bool {
    if calls.len() <= 1 {
        return false;
    }
    if calls.iter().any(|c| !is_read_only(&c.tool)) {
        // Never parallelize mutations with readers.
        return false;
    }
    match mode {
        ParallelMode::Serial => false,
        ParallelMode::Parallel => true,
        // Parallel opens are expensive; only pay them when enough work might overlap.
        ParallelMode::Auto => calls.len() >= 4,
    }
}

/// Run many tool calls in one process with a shared or parallel Searcher strategy.
pub fn run_batch(config: SessionConfig, request: &BatchRequest) -> Result<BatchResponse, CallError> {
    validate_calls(&request.calls)?;
    let config = apply_config(config, request);
    let mode = resolve_mode(request);
    let started = Instant::now();
    let parallel = choose_parallel(mode, &request.calls);
    let results = if parallel {
        run_parallel(&config, &request.calls)
    } else {
        run_serial(&config, &request.calls)
    };
    let all_ok = results.iter().all(|r| r.ok);
    Ok(BatchResponse {
        all_ok,
        call_count: results.len(),
        results,
        wall_ms: started.elapsed().as_millis(),
        mode: if parallel { "parallel" } else { "serial" },
    })
}

fn validate_calls(calls: &[BatchCall]) -> Result<(), CallError> {
    if calls.is_empty() {
        return Err(CallError::InvalidArgs("batch.calls must be non-empty".into()));
    }
    if calls.len() > MAX_BATCH_CALLS {
        return Err(CallError::InvalidArgs(format!(
            "batch.calls exceeds max {MAX_BATCH_CALLS}"
        )));
    }
    for call in calls {
        if call.id.is_empty() {
            return Err(CallError::InvalidArgs("batch call id must be non-empty".into()));
        }
        if call.tool.is_empty() {
            return Err(CallError::InvalidArgs("batch call tool must be non-empty".into()));
        }
    }
    Ok(())
}

fn apply_config(mut config: SessionConfig, request: &BatchRequest) -> SessionConfig {
    if let Some(root) = &request.root {
        config.root = root.clone();
    }
    if request.index_path.is_some() {
        config.index_path = request.index_path.clone();
    }
    if let Some(use_embed) = request.use_embed {
        config.use_embed = use_embed;
    }
    if let Some(limit) = request.limit {
        config.limit = limit.clamp(1, 500);
    }
    config
}

fn run_serial(config: &SessionConfig, calls: &[BatchCall]) -> Vec<BatchCallResult> {
    let mut session = CodeModeSession::new(config.clone());
    session.max_calls = MAX_BATCH_CALLS.saturating_mul(4);
    calls.iter().map(|call| invoke(&mut session, call)).collect()
}

fn run_parallel(config: &SessionConfig, calls: &[BatchCall]) -> Vec<BatchCallResult> {
    use rayon::prelude::*;
    calls
        .par_iter()
        .map(|call| {
            let mut session = CodeModeSession::new(config.clone());
            session.max_calls = 8;
            invoke(&mut session, call)
        })
        .collect()
}

fn invoke(session: &mut CodeModeSession, call: &BatchCall) -> BatchCallResult {
    match session.call(&call.tool, call.args.clone()) {
        Ok(value) => BatchCallResult {
            id: call.id.clone(),
            ok: true,
            value: Some(value),
            error: None,
        },
        Err(err) => BatchCallResult {
            id: call.id.clone(),
            ok: false,
            value: None,
            error: Some(err.to_string()),
        },
    }
}

/// Sticky worker: one warm session, NDJSON requests on stdin, NDJSON on stdout.
///
/// Eliminates per-wave process spawn for multi-step Code Mode programs.
pub fn run_serve(config: SessionConfig, stdin: impl BufRead, mut stdout: impl Write) -> Result<(), CallError> {
    let mut session = CodeModeSession::new(config);
    session.max_calls = 10_000;
    for line in stdin.lines() {
        let line = line.map_err(|e| CallError::InvalidArgs(format!("stdin read: {e}")))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let req: ServeRequest = serde_json::from_str(line)
            .map_err(|e| CallError::InvalidArgs(format!("serve request: {e}")))?;
        match req {
            ServeRequest::End => {
                write_line(&mut stdout, &ServeResponse::Bye)?;
                break;
            }
            ServeRequest::Call { id, tool, args } => {
                let result = match session.call(&tool, args) {
                    Ok(value) => ServeResponse::Result {
                        id,
                        ok: true,
                        value: Some(value),
                        error: None,
                    },
                    Err(err) => ServeResponse::Result {
                        id,
                        ok: false,
                        value: None,
                        error: Some(err.to_string()),
                    },
                };
                write_line(&mut stdout, &result)?;
            }
            ServeRequest::Batch {
                id,
                calls,
                parallel_mode,
            } => {
                // Sticky serve always uses the warm session (serial). Parallel would
                // defeat the sticky Searcher — callers wanting parallel use one-shot batch.
                let _ = parallel_mode;
                if let Err(err) = validate_calls(&calls) {
                    write_line(
                        &mut stdout,
                        &ServeResponse::Error {
                            id: Some(id),
                            error: err.to_string(),
                        },
                    )?;
                    continue;
                }
                let started = Instant::now();
                let results: Vec<_> = calls.iter().map(|c| invoke(&mut session, c)).collect();
                let all_ok = results.iter().all(|r| r.ok);
                write_line(
                    &mut stdout,
                    &ServeResponse::BatchResult {
                        id,
                        all_ok,
                        results,
                        wall_ms: started.elapsed().as_millis(),
                        mode: "serial".into(),
                    },
                )?;
            }
        }
    }
    Ok(())
}

fn write_line(stdout: &mut impl Write, value: &impl Serialize) -> Result<(), CallError> {
    serde_json::to_writer(&mut *stdout, value)
        .map_err(|e| CallError::Other(anyhow::anyhow!("serve write: {e}")))?;
    stdout
        .write_all(b"\n")
        .map_err(|e| CallError::Other(anyhow::anyhow!("serve write: {e}")))?;
    stdout
        .flush()
        .map_err(|e| CallError::Other(anyhow::anyhow!("serve flush: {e}")))?;
    Ok(())
}

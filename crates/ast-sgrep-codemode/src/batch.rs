//! Batch + sticky-serve execution for Code Mode.
//!
//! Default mode is **serial warm** (one Searcher reused): for typical capsule
//! queries, SQLite open dominates, so N parallel opens are slower than N
//! sequential searches on one connection (Amdahl).

use crate::catalog::tool_catalog;
use crate::session::{encoded_len, CodeModeSession, SessionConfig};
use crate::tools::CallError;
use ast_sgrep_core::io_bounds::{read_bounded_line, BoundedLine};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, Write};
use std::time::Instant;

/// Maximum tool calls in one batch wave (keeps one process bounded).
pub const MAX_BATCH_CALLS: usize = 32;
/// Maximum encoded batch response size.
pub const MAX_BATCH_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const BATCH_ENVELOPE_RESERVE_BYTES: usize = 64 * 1024;
pub const MAX_BATCH_VALUE_BYTES: usize = MAX_BATCH_RESPONSE_BYTES - BATCH_ENVELOPE_RESERVE_BYTES;
pub const MAX_BATCH_ID_BYTES: usize = 128;
pub const MAX_BATCH_TOOL_BYTES: usize = 128;
pub const MAX_BATCH_ERROR_BYTES: usize = 8 * 1024;

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParallelMode {
    /// Always one Searcher, sequential calls (fastest for cheap queries).
    Serial,
    /// One Searcher per call on rayon (only when all tools are read-only).
    Parallel,
    /// Serial unless N>=4 read-only calls (heuristic).
    #[default]
    Auto,
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
pub fn run_batch(
    config: SessionConfig,
    request: &BatchRequest,
) -> Result<BatchResponse, CallError> {
    validate_calls(&request.calls)?;
    let config = apply_config(config, request);
    let mode = resolve_mode(request);
    let started = Instant::now();
    let parallel = choose_parallel(mode, &request.calls);
    let mut results = if parallel {
        run_parallel(&config, &request.calls)
    } else {
        run_serial(&config, &request.calls)
    };
    enforce_batch_response_budget(&mut results);
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
        return Err(CallError::InvalidArgs(
            "batch.calls must be non-empty".into(),
        ));
    }
    if calls.len() > MAX_BATCH_CALLS {
        return Err(CallError::InvalidArgs(format!(
            "batch.calls exceeds max {MAX_BATCH_CALLS}"
        )));
    }
    for call in calls {
        validate_call_identity(&call.id, &call.tool)?;
    }
    Ok(())
}

fn validate_call_identity(id: &str, tool: &str) -> Result<(), CallError> {
    validate_id(id)?;
    if tool.is_empty() {
        return Err(CallError::InvalidArgs(
            "batch call tool must be non-empty".into(),
        ));
    }
    if tool.len() > MAX_BATCH_TOOL_BYTES {
        return Err(CallError::InvalidArgs(format!(
            "batch call tool exceeds {MAX_BATCH_TOOL_BYTES} bytes"
        )));
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<(), CallError> {
    if id.is_empty() {
        return Err(CallError::InvalidArgs(
            "batch call id must be non-empty".into(),
        ));
    }
    if id.len() > MAX_BATCH_ID_BYTES {
        return Err(CallError::InvalidArgs(format!(
            "batch call id exceeds {MAX_BATCH_ID_BYTES} bytes"
        )));
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
    calls
        .iter()
        .map(|call| invoke(&mut session, call))
        .collect()
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
            error: Some(bound_error(err.to_string())),
        },
    }
}

fn enforce_batch_response_budget(results: &mut [BatchCallResult]) {
    let mut used = BATCH_ENVELOPE_RESERVE_BYTES;
    for result in results {
        let bytes = encoded_len(result).unwrap_or(usize::MAX);
        if used.saturating_add(bytes).saturating_add(1) <= MAX_BATCH_VALUE_BYTES {
            used += bytes;
            continue;
        }
        result.ok = false;
        result.value = None;
        result.error = Some(format!(
            "codemode batch response exceeds {MAX_BATCH_RESPONSE_BYTES} bytes"
        ));
        used = used
            .saturating_add(encoded_len(result).unwrap_or(usize::MAX))
            .saturating_add(1);
    }
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

/// Sticky worker: one warm session, NDJSON requests on stdin, NDJSON on stdout.
///
/// Eliminates per-wave process spawn for multi-step Code Mode programs.
pub fn run_serve(
    config: SessionConfig,
    mut stdin: impl BufRead,
    mut stdout: impl Write,
) -> Result<(), CallError> {
    let mut session = CodeModeSession::new(config);
    session.max_calls = 10_000;
    loop {
        let Some(line) = read_bounded_line(&mut stdin, ast_sgrep_core::MAX_STDIN_LINE_BYTES)
            .map_err(|e| CallError::InvalidArgs(format!("stdin read: {e}")))?
        else {
            break;
        };
        let line = match line {
            BoundedLine::Line(line) => line,
            BoundedLine::TooLong => {
                write_line(
                    &mut stdout,
                    &ServeResponse::Error {
                        id: None,
                        error: format!(
                            "serve request line exceeds max {} bytes",
                            ast_sgrep_core::MAX_STDIN_LINE_BYTES
                        ),
                    },
                )?;
                continue;
            }
        };
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let raw: Value = match serde_json::from_slice(&line) {
            Ok(raw) => raw,
            Err(error) => {
                write_line(
                    &mut stdout,
                    &ServeResponse::Error {
                        id: None,
                        error: bound_error(format!("serve request: {error}")),
                    },
                )?;
                continue;
            }
        };
        let request_id = raw
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| validate_id(id).is_ok())
            .map(str::to_owned);
        let req: ServeRequest = match serde_json::from_value(raw) {
            Ok(req) => req,
            Err(error) => {
                write_line(
                    &mut stdout,
                    &ServeResponse::Error {
                        id: request_id,
                        error: bound_error(format!("serve request: {error}")),
                    },
                )?;
                continue;
            }
        };
        match req {
            ServeRequest::End => {
                write_line(&mut stdout, &ServeResponse::Bye)?;
                break;
            }
            ServeRequest::Call { id, tool, args } => {
                if let Err(err) = validate_call_identity(&id, &tool) {
                    write_line(
                        &mut stdout,
                        &ServeResponse::Error {
                            id: Some(id),
                            error: bound_error(err.to_string()),
                        },
                    )?;
                    continue;
                }
                // br-r49: a spent session answers the offending request once
                // and then dies — never a flood of identical budget errors.
                if session.exhausted() {
                    write_line(
                        &mut stdout,
                        &ServeResponse::Result {
                            id,
                            ok: false,
                            value: None,
                            error: Some(bound_error(
                                CallError::BudgetExhausted(session.max_calls).to_string(),
                            )),
                        },
                    )?;
                    return Err(CallError::BudgetExhausted(session.max_calls));
                }
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
                        error: Some(bound_error(err.to_string())),
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
                if let Err(err) = validate_id(&id) {
                    write_line(
                        &mut stdout,
                        &ServeResponse::Error {
                            id: Some(id),
                            error: bound_error(err.to_string()),
                        },
                    )?;
                    continue;
                }
                if let Err(err) = validate_calls(&calls) {
                    write_line(
                        &mut stdout,
                        &ServeResponse::Error {
                            id: Some(id),
                            error: bound_error(err.to_string()),
                        },
                    )?;
                    continue;
                }
                let started = Instant::now();
                // br-r49: same fail-once contract as single calls — a spent
                // session answers the batch once and stops.
                if session.exhausted() {
                    write_line(
                        &mut stdout,
                        &ServeResponse::Error {
                            id: Some(id),
                            error: bound_error(
                                CallError::BudgetExhausted(session.max_calls).to_string(),
                            ),
                        },
                    )?;
                    return Err(CallError::BudgetExhausted(session.max_calls));
                }
                let mut results: Vec<_> = calls.iter().map(|c| invoke(&mut session, c)).collect();
                enforce_batch_response_budget(&mut results);
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

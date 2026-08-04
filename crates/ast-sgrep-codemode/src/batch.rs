//! Same-tick batch execution for Code Mode (Amdahl: cut process-spawn serial cost).

use crate::session::{CodeModeSession, SessionConfig};
use crate::tools::CallError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    pub ok: bool,
    pub results: Vec<BatchCallResult>,
    pub call_count: usize,
    pub wall_ms: u128,
    /// `parallel` = one Searcher session per call on a thread pool; `serial` = warm reuse.
    pub mode: &'static str,
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
    /// Prefer parallel sessions when call count > 1 (default true).
    #[serde(default = "default_true")]
    pub parallel: bool,
    pub calls: Vec<BatchCall>,
}

fn default_true() -> bool {
    true
}

/// Run many tool calls in one process.
///
/// Amdahl: one process start + N searches beats N cold CLI spawns when search
/// work is short. When `parallel` is true and N>1, each call gets its own
/// session on a rayon thread (multiple SQLite readers) so wall time ≈ max(call)
/// rather than sum(call).
pub fn run_batch(config: SessionConfig, request: &BatchRequest) -> Result<BatchResponse, CallError> {
    if request.calls.is_empty() {
        return Err(CallError::InvalidArgs("batch.calls must be non-empty".into()));
    }
    if request.calls.len() > MAX_BATCH_CALLS {
        return Err(CallError::InvalidArgs(format!(
            "batch.calls exceeds max {MAX_BATCH_CALLS}"
        )));
    }
    for call in &request.calls {
        if call.id.is_empty() {
            return Err(CallError::InvalidArgs("batch call id must be non-empty".into()));
        }
        if call.tool.is_empty() {
            return Err(CallError::InvalidArgs("batch call tool must be non-empty".into()));
        }
    }

    let mut config = config;
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

    let started = Instant::now();
    let parallel = request.parallel && request.calls.len() > 1;
    let results = if parallel {
        run_parallel(&config, &request.calls)
    } else {
        run_serial(&config, &request.calls)
    };

    let ok = results.iter().all(|r| r.ok);
    Ok(BatchResponse {
        ok,
        call_count: results.len(),
        results,
        wall_ms: started.elapsed().as_millis(),
        mode: if parallel { "parallel" } else { "serial" },
    })
}

fn run_serial(config: &SessionConfig, calls: &[BatchCall]) -> Vec<BatchCallResult> {
    let mut session = CodeModeSession::new(config.clone());
    session.max_calls = MAX_BATCH_CALLS.saturating_mul(2);
    calls
        .iter()
        .map(|call| match session.call(&call.tool, call.args.clone()) {
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
        })
        .collect()
}

fn run_parallel(config: &SessionConfig, calls: &[BatchCall]) -> Vec<BatchCallResult> {
    use rayon::prelude::*;
    calls
        .par_iter()
        .map(|call| {
            let mut session = CodeModeSession::new(config.clone());
            session.max_calls = 8;
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
        })
        .collect()
}

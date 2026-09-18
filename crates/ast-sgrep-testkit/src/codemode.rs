//! CodeMode session fixtures (feature `codemode`).
//!
//! # Contract
//!
//! - One canonical copy of the session/batch/serve harness duplicated across
//!   the `tests/codemode` suites (error-api, oracle, numerical, invalidation,
//!   recovery, ffi). Hermetic by construction: explicit `root`, `index_path:
//!   None` (no ambient `ASGREP_INDEX_PATH`), `limit: 5`, `use_embed: false`,
//!   `AgentCapsule` format.
//! - Builders are pure constructors; only [`serve_lines`] runs requests, and
//!   it returns `(Result, lines)` so the caller — not this module — decides
//!   the verdict. `serve_lines` panics only when serve output is not UTF-8.
//! - Determinism: fixtures are fixed values; served outputs are deterministic
//!   up to session behavior (lexical only, no embeddings, no wall-clock).

use ast_sgrep_codemode::{
    run_serve, BatchCall, BatchRequest, CallError, CodeModeSession, ServeRequest, SessionConfig,
};
use ast_sgrep_plugins::OutputFormat;
use serde_json::{json, Value};
use std::io::Cursor;
use std::path::Path;

/// Hermetic session config: explicit `root`, no index path, limit 5, lexical
/// only, `AgentCapsule` format. Pure constructor.
pub fn config_at(root: &Path) -> SessionConfig {
    SessionConfig {
        root: root.to_path_buf(),
        index_path: None,
        limit: 5,
        use_embed: false,
        default_format: OutputFormat::AgentCapsule,
    }
}

/// Unindexed [`CodeModeSession`] over [`config_at`]: the pure-transform
/// fixture (catalog/filter/select/plan math without an index).
pub fn session_at(root: &Path) -> CodeModeSession {
    CodeModeSession::new(config_at(root))
}

/// All-`None` batch envelope around `calls` (no root/index/embed/limit/
/// parallel overrides). Pure constructor.
pub fn batch_request(calls: Vec<BatchCall>) -> BatchRequest {
    BatchRequest {
        root: None,
        index_path: None,
        use_embed: None,
        limit: None,
        parallel: None,
        parallel_mode: None,
        calls,
    }
}

/// One batch call (`id`, `tool`, JSON `args`). Pure constructor.
pub fn batch_call(id: &str, tool: &str, args: Value) -> BatchCall {
    BatchCall {
        id: id.to_string(),
        tool: tool.to_string(),
        args,
    }
}

/// `catalog_search` batch call with an explicit query (the 2-arg canonical
/// form; the 1-arg `query = "search"` copies are `catalog_call(id, "search")`).
/// Pure constructor.
pub fn catalog_call(id: &str, query: &str) -> BatchCall {
    batch_call(id, "catalog_search", json!({"query": query}))
}

/// Run `input` (newline-delimited serve requests) through a sticky `run_serve`
/// worker on [`config_at`]: returns the terminal result plus every output line
/// as UTF-8 strings. Panics only when serve output is not UTF-8.
pub fn serve_lines(input: String, root: &Path) -> (Result<(), CallError>, Vec<String>) {
    let mut out = Vec::new();
    let result = run_serve(config_at(root), Cursor::new(input), &mut out);
    let text = String::from_utf8(out).expect("serve output is utf8");
    let lines = text.lines().map(str::to_string).collect();
    (result, lines)
}

/// Serialize one [`ServeRequest`] as a newline-terminated serve input line.
/// Pure constructor.
pub fn serve_request_line(request: &ServeRequest) -> String {
    format!(
        "{}\n",
        serde_json::to_string(request).expect("request serializes")
    )
}

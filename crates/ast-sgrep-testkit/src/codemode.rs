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

/// Hermetic indexed session config: explicit `root` + `index_path`, limit 5,
/// lexical only, `AgentCapsule` format. Pure constructor — the indexed
/// counterpart of [`config_at`] for suites whose sessions must touch a store
/// (explicit temp db, never the ambient index home).
pub fn config_at_indexed(root: &Path, index_path: &Path) -> SessionConfig {
    SessionConfig {
        root: root.to_path_buf(),
        index_path: Some(index_path.to_path_buf()),
        limit: 5,
        use_embed: false,
        default_format: OutputFormat::AgentCapsule,
    }
}

/// Indexed [`CodeModeSession`] over [`config_at_indexed`].
pub fn session_at_indexed(root: &Path, index_path: &Path) -> CodeModeSession {
    CodeModeSession::new(config_at_indexed(root, index_path))
}

/// Exact `call_now` fast-gate reason. Owned by the napi wrapper: slow, aliased,
/// and unknown tools are rejected with this text before dispatch, uncounted.
pub const CALL_NOW_ONLY: &str = "callNow is only for bounded metadata/symbol lookups; use call() for search/index/semantic/chain";

/// Exact contention reason. The napi `try_lock` loser path returns this so the
/// JS host can fall back to `call()`.
pub const SESSION_BUSY: &str = "session is busy";

/// Exact JS-visible budget text: core `CallError::BudgetExhausted(10_000)`
/// surfaced through napi. Bilateral suites also assert dynamic equality with
/// the core `Display`, so core drift fails loudly instead of silently.
pub const BUDGET_EXCEEDED: &str = "codemode call budget exceeded (max_calls=10000)";

/// Exact missing-symbol reason: core `InvalidArgs` text surfaced through napi
/// (shared by `defs` and `callers`).
pub const DEFS_NEEDS_SYMBOL: &str =
    "symbol is required. Call asgrep.defs(\"Name\") or asgrep.defs({ symbol: \"Name\" })";

/// Core query-length limit, re-exported so suites pin oversize behavior
/// without a literal that can drift from `ast-sgrep-core`.
pub use ast_sgrep_core::MAX_QUERY_CHARS;

/// Byte-identity over canonical serialization, not just `Value` equality:
/// bilateral (napi-vs-core) and determinism (repeat/fresh-session) agreement.
/// Panics with `what` context on mismatch. Pure assertion.
pub fn assert_json_byte_identical(a: &Value, b: &Value, what: &str) {
    let left = serde_json::to_vec(a).expect("serialize left");
    let right = serde_json::to_vec(b).expect("serialize right");
    assert_eq!(left, right, "{what} mismatch");
}

/// Serialize one [`ServeRequest`] as a newline-terminated serve input line.
/// Pure constructor.
pub fn serve_request_line(request: &ServeRequest) -> String {
    format!(
        "{}\n",
        serde_json::to_string(request).expect("request serializes")
    )
}

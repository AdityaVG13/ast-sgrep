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
    run_serve, BatchCall, BatchRequest, CallError, CodeModeSession, ParallelMode, ServeRequest,
    SessionConfig,
};
use ast_sgrep_core::search::HitSignal;
use ast_sgrep_core::{HitKind, SearchHit};
use ast_sgrep_plugins::OutputFormat;
use serde_json::{json, Value};
use std::io::Cursor;
use std::path::Path;
use tempfile::TempDir;

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

/// [`batch_request`] with an explicit Serial/Parallel wave mode.
/// INTENT: recovery suites pin mode agreement (serial vs parallel answers
/// identical per call); the all-`None` envelope cannot express that. Pure
/// constructor; [`batch_request`] is untouched, so existing callers keep the
/// `Auto` default byte-identically.
pub fn batch_request_with_mode(calls: Vec<BatchCall>, mode: ParallelMode) -> BatchRequest {
    BatchRequest {
        root: None,
        index_path: None,
        use_embed: None,
        limit: None,
        parallel: None,
        parallel_mode: Some(mode),
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

/// INTENT: indexed session that creates its own out-of-root index dir,
/// runs `index_repo`, and asserts `ok:true` — the indexed-drill fixture.
/// Delta vs [`session_at_indexed`]: that is the pure constructor; this runs
/// the index beat. The caller keeps the [`TempDir`] alive. Panics on IO
/// failure or a non-ok index beat.
pub fn indexed_session_at(root: &Path) -> (TempDir, CodeModeSession) {
    let index_dir = tempfile::tempdir().expect("index dir");
    let mut session = session_at_indexed(root, &index_dir.path().join("index.db"));
    let indexed = session
        .call("index_repo", json!({"force": false}))
        .expect("initial index");
    assert_eq!(indexed["ok"], json!(true));
    (index_dir, session)
}

/// INTENT: 3-hit boundary straddle (2.0/1.999/2.001) — the filter
/// exactness/totality fixture. Pure constructor.
pub fn hits_fixture() -> Value {
    json!([
        {"kind": "def", "file": "src/a.rs", "score": 2.0},
        {"kind": "def", "file": "src/b.rs", "score": 1.999},
        {"kind": "def", "file": "src/c.rs", "score": 2.001},
    ])
}

/// INTENT: five hits with strictly descending scores 5..1 (input order is
/// score order) for threshold/limit relations. Pure constructor.
pub fn scored_hits5() -> Value {
    json!([
        {"kind": "def", "file": "src/s5.rs", "score": 5.0},
        {"kind": "def", "file": "src/s4.rs", "score": 4.0},
        {"kind": "def", "file": "src/s3.rs", "score": 3.0},
        {"kind": "def", "file": "src/s2.rs", "score": 2.0},
        {"kind": "def", "file": "src/s1.rs", "score": 1.0},
    ])
}

/// INTENT: six hits with strictly descending scores 6..1 (input order is
/// score order) for threshold sweeps and plan/chain drills. Pure
/// constructor.
pub fn scored_hits6() -> Value {
    json!([
        {"kind": "def", "file": "f6.rs", "score": 6.0},
        {"kind": "def", "file": "f5.rs", "score": 5.0},
        {"kind": "def", "file": "f4.rs", "score": 4.0},
        {"kind": "def", "file": "f3.rs", "score": 3.0},
        {"kind": "def", "file": "f2.rs", "score": 2.0},
        {"kind": "def", "file": "f1.rs", "score": 1.0},
    ])
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

/// INTENT: canonical budget-rendering hit — a Def hit over `lib.rs:1-3` with a
/// caller-chosen multi-line excerpt. [`crate::mk_hit`] fixes a synthetic
/// excerpt, but budget rendering is excerpt-only, so oracles need this ONE
/// canonical shape. Pure constructor.
pub fn sample_search_hit(excerpt: &str) -> SearchHit {
    SearchHit {
        kind: HitKind::Def,
        file: "lib.rs".to_string(),
        line_start: 1,
        line_end: 3,
        symbol: Some("alpha".to_string()),
        caller: None,
        callee: None,
        language: Some("rust".to_string()),
        score: 3.0,
        signal: HitSignal::Exact,
        contributors: vec![HitKind::Def],
        margin: 0.0,
        confidence: 0.0,
        resolution: None,
        embed_fields: None,
        critic: Vec::new(),
        excerpt: excerpt.to_string(),
        byte_span: None,
    }
}

/// INTENT: the 2-step catalog→select plan — the oracle determinism /
/// return-default / budget fixture. Pure constructor.
pub fn search_select_plan() -> Value {
    json!({"steps": [
        {"id": "a", "tool": "catalog_search", "args": {"query": "search"}},
        {"id": "b", "tool": "select", "args": {"value": "$a", "fields": ["tools"]}},
    ], "return": "$b"})
}

/// INTENT: total `CallError` discriminant projection (the enum variant
/// only, never message text): the cross-cutting error-comparison
/// primitive — equal discriminants mean the same failure layer reached
/// the caller. Pure projection.
pub fn call_error_discriminant(err: &CallError) -> &'static str {
    match err {
        CallError::UnknownTool(_) => "unknown_tool",
        CallError::InvalidArgs(_) => "invalid_args",
        CallError::BudgetExhausted(_) => "budget_exhausted",
        CallError::Json(_) => "json",
        CallError::Other(_) => "other",
    }
}

/// INTENT: canonical `CallError::Other` cause-chain assert — an anyhow
/// failure wrapped as `Other` must keep its cause: the std source is
/// present and the original typed cause `E` is still reachable by walking
/// the chain, never flattened to a bare string. Generic over the expected
/// cause type (e.g. `assert_other_preserves_cause::<std::io::Error>`).
/// Panics when the error is not `Other`, has no source, or the chain
/// lacks `E`.
pub fn assert_other_preserves_cause<E>(err: &CallError)
where
    E: std::error::Error + 'static,
{
    let inner = match err {
        CallError::Other(inner) => inner,
        other => panic!("expected Other, got {other:?}"),
    };
    assert!(
        std::error::Error::source(err).is_some(),
        "Other must keep a source"
    );
    assert!(
        inner
            .chain()
            .any(|cause| cause.downcast_ref::<E>().is_some()),
        "{} cause must survive the wrap",
        std::any::type_name::<E>()
    );
}

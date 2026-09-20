#![allow(unused_imports)]
//! Shared kit for the MCP error-API suites (`error_api_pass1`–`error_api_pass4`).
//!
//! The `allow(unused_imports)` is load-bearing (mirrors core's kit): each pass
//! file uses a subset of the re-exports, and an import unused in one target
//! must not fail another (this bit --fix once: do not "clean" it).
//!
//! Thin facade: builders, extractors, shape asserts, fixtures, and the
//! timeout-bounded one-shot drivers come from `ast_sgrep_testkit`
//! (re-exported below); this module holds only what is genuinely
//! area-local. Area-local items carry a plain keep-comment.
//!
//! Contracts (inherited from testkit where reused):
//!
//! - Discriminants are codes, envelope shapes, and raw bytes — never message
//!   text. `tool_error_discriminant` projects everything about a tool error
//!   EXCEPT the human message.
//! - Every live-session read and every process wait is timeout-bounded (15s),
//!   so a regressed server fails the test instead of hanging the suite.
//! - One-shot drivers are strictly sequential (response order matches request
//!   order), except `rpc_pipeline` which returns arrival order by design.
//! - Strictest-shape unification: `assert_tool_error_shape` is testkit's
//!   strict form (exactly one `text` block); `assert_tool_success_shape`
//!   keeps the `structuredContent` requirement, which is STRICTER than
//!   testkit's `assert_tool_success`.

// Each of the 4 pass targets compiles this module and uses a subset;
// per-target unused items are expected, not dead code.
#![allow(dead_code)]

use serde_json::Value;
use std::collections::HashMap;

// --- testkit re-exports: builders, extractors, shape asserts, fixtures,
// --- timeout-bounded one-shot drivers, canonical state path ---
//
// Unify on the STRICTEST shape where local copies drifted:
// - `assert_tool_error_shape`: testkit's strict form.
// - `assert_tool_success_shape`: the strict form (requires
//   `structuredContent`); strictly stronger than `assert_tool_success`.
// - `corrupt_index_db`: testkit's deterministic sentinel (the fault
//   contract is byte-identical every run).
pub use ast_sgrep_testkit::{
    assert_jsonrpc_error, assert_tool_error_shape, assert_tool_success_shape, corrupt_index_db,
    index_db_path, rpc_pipeline, rpc_session, rpc_session_env, rpc_session_raw, tool_body,
    tool_call, tool_error_discriminant, LiveSession,
};

/// The 5 search channels swept by the cross-channel relations.
// WHY area-local: the error-area channel sweep; other suites enumerate tools.
pub const SEARCH_CHANNELS: [&str; 5] = [
    "search",
    "keyword_search",
    "ast_search",
    "semantic_search",
    "code_search",
];

/// Canonical error-API fixture source: one findable symbol.
// WHY area-local: single-symbol tree keeps `file_count`/`nodes` pins exact.
pub const FIXTURE_SOURCE: &str = "fn target_symbol() {}\n";

/// Single-file tree with one findable symbol, not yet indexed.
// WHY area-local: zero-arg canonical fixture over testkit's parameterized one.
pub fn file_tree() -> tempfile::TempDir {
    ast_sgrep_testkit::file_tree(&[("src/lib.rs", FIXTURE_SOURCE)])
}

/// Single-file tree with one findable symbol, indexed.
// WHY area-local: zero-arg canonical fixture over testkit's parameterized one.
pub fn indexed_tree() -> tempfile::TempDir {
    let temp = file_tree();
    ast_sgrep_testkit::index_tree(temp.path());
    temp
}

/// Parse one raw JSON-RPC line.
// WHY area-local: one-line alias keeping relation comparators readable.
pub fn parse_line(raw: &str) -> Value {
    serde_json::from_str(raw).expect("JSON-RPC")
}

/// Canonical form of a response with the `id` normalized away, for comparing
/// calls that differ only in id. `serde_json::Value` sorts object keys, so
/// `to_string` is a canonical byte encoding.
// WHY area-local: within-session repetition comparator for the E3 relations.
pub fn canonical_modulo_id(raw: &str) -> String {
    let mut value = parse_line(raw);
    value["id"] = serde_json::json!(0);
    serde_json::to_string(&value).expect("canonical JSON")
}

/// Index raw response lines by numeric `id` (panics on duplicates).
// WHY area-local: permutation comparator for the E3 position relations.
pub fn by_id(raw_lines: &[String]) -> HashMap<i64, &str> {
    let mut map = HashMap::new();
    for line in raw_lines {
        let id = parse_line(line)["id"].as_i64().expect("numeric id echo");
        assert!(map.insert(id, line.as_str()).is_none(), "duplicate id {id}");
    }
    map
}

/// Search call over the canonical fixture symbol (registers hits by default).
// WHY area-local: drill-call sugar for the E4 live sessions.
pub fn search_call(id: u32, channel: &str, limit: u32) -> Value {
    tool_call(
        id,
        channel,
        serde_json::json!({"query": "target_symbol", "limit": limit, "resend_seen": true}),
    )
}

/// Read call over the canonical fixture's first line.
// WHY area-local: drill-call sugar for the E4 live sessions.
pub fn read_call(id: u32) -> Value {
    tool_call(
        id,
        "code_read",
        serde_json::json!({"ids": ["src/lib.rs#L1-L1"]}),
    )
}

/// Send-one/read-one with id-echo assertion plus clean-finish, over testkit's
/// timeout-bounded [`LiveSession`].
// WHY area-local: two-line drill verbs; transport stays in testkit.
pub trait LiveSessionExt {
    /// Send one call, read its response, assert the id echo.
    fn call(&mut self, payload: &Value, id: u32) -> Value;
    /// Close stdin, wait (bounded), and assert a clean exit.
    fn finish_clean(&mut self);
}

impl LiveSessionExt for LiveSession {
    fn call(&mut self, payload: &Value, id: u32) -> Value {
        self.send(payload);
        let response = self.recv();
        assert_eq!(response["id"], id, "{response:#}");
        response
    }

    fn finish_clean(&mut self) {
        self.close_stdin();
        let status = self.wait_clean();
        assert!(status.success(), "MCP exited {status}");
    }
}

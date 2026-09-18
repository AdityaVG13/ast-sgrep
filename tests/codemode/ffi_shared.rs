//! Shared hermetic setup for the four FFI suites (no `#[test]` here).
//!
//! Included via `#[path = "ffi_shared.rs"]`. Centralizes the napi-typed
//! helpers — `Session` construction, the napi+core bilateral pair, schema
//! materialization — which cannot move to `ast-sgrep-testkit` without adding
//! a testkit→napi dependency (testkit's `Cargo.toml` is out of scope for the
//! FFI consolidation). Everything testkit *can* offer (reason pins,
//! `MAX_QUERY_CHARS`, byte-identity assert, core batch builders, file writers)
//! is imported from testkit instead; nothing here duplicates it.

use ast_sgrep_codemode::CodeModeSession;
use ast_sgrep_codemode_napi::{JsBatchCall, JsSessionConfig, Session};
use serde_json::Value;
use std::path::Path;
use tempfile::TempDir;

/// Fresh empty workspace root; sessions stay lazy (no Searcher opens here).
pub fn empty_root() -> TempDir {
    TempDir::new().expect("tempdir")
}

/// Hermetic napi session: explicit temp `index_path` (`None` would materialize
/// a db under the real index home on first store touch), lexical only.
pub fn session_on(root: &Path, db: &str) -> Session {
    Session::new(Some(JsSessionConfig {
        root: Some(root.display().to_string()),
        index_path: Some(root.join(db).display().to_string()),
        limit: None,
        use_embed: Some(false),
    }))
    .expect("Session::new")
}

/// `index_status` opens the store writable, materializing an empty schema so
/// later readonly opens (find/search/read/defs) serve zero-hit results instead
/// of the fail-closed "index is empty" gate.
pub fn materialize(session: &Session) {
    session
        .call_now("index_status".to_string(), None)
        .expect("materialize empty schema");
}

/// Napi wrapper + direct core on the SAME root and index db, with mirrored
/// config (limit 5 both sides, no embed), so outputs must be byte-identical.
/// The core half reuses the testkit indexed fixture; the napi `limit` mirrors
/// it explicitly because `Session::new(None)` would read ambient env.
pub fn pair(root: &Path, db: &str) -> (Session, CodeModeSession) {
    let index_path = root.join(db);
    let napi = Session::new(Some(JsSessionConfig {
        root: Some(root.display().to_string()),
        index_path: Some(index_path.display().to_string()),
        limit: Some(5),
        use_embed: Some(false),
    }))
    .expect("Session::new");
    let core = ast_sgrep_testkit::session_at_indexed(root, &index_path);
    (napi, core)
}

/// `index_status` on both sides materializes the shared empty schema; each
/// side counts its own call.
pub fn materialize_both(napi: &Session, core: &mut CodeModeSession) {
    napi.call_now("index_status".to_string(), Some(serde_json::json!({})))
        .expect("napi materialize");
    core.call("index_status", serde_json::json!({}))
        .expect("core materialize");
}

/// One napi batch call (`id`, `tool`, optional JSON `args`). The core-side
/// counterpart is testkit's `batch_call`.
pub fn js_call(id: &str, tool: &str, args: Option<Value>) -> JsBatchCall {
    JsBatchCall {
        id: id.to_string(),
        tool: tool.to_string(),
        args,
    }
}

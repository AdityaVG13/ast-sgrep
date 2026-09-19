//! CodeMode crash-recovery fixtures (feature `codemode`).
//!
//! # Contract
//!
//! - One canonical copy of the file-local helpers triplicated across the
//!   `tests/codemode/recovery_*` suites (contracts/drills/relations).
//! - Record loaders layer errors as the product does: IO failure -> `Other`,
//!   syntax failure -> `Json`, well-formed values delegate to `parse_plan` /
//!   batch validation. The layer — not message text — is the discriminant.
//! - Fixtures are fixed-content tempdirs; the `token` parameter only selects
//!   the unique searchable symbol so suites keep distinct tokens.
//! - Helpers panic (never `Result`) on fixture IO/index failure, matching
//!   suite convention: a broken fixture is a test failure, not a fallible op.

use ast_sgrep_codemode::{
    parse_plan, run_serve, BatchRequest, CallError, CodeModeSession, Plan, SessionConfig,
};
use serde_json::{json, Value};
use std::io::Cursor;
use std::path::Path;
use tempfile::TempDir;

/// INTENT: indexed session over a fixed two-file repo whose first file
/// defines exactly `token`: the writable starting point every codemode
/// recovery test builds from. Shaped counts stay hand-computable because the
/// token lives in exactly one file.
pub fn indexed_codemode_repo(token: &str) -> (TempDir, SessionConfig) {
    let body = format!("pub fn {token}() {{}}\n");
    let temp = crate::fixture::file_tree(&[
        ("src/a.rs", body.as_str()),
        ("src/b.rs", "pub fn other_fn() {}\n"),
    ]);
    let config = crate::codemode::config_at_indexed(temp.path(), &temp.path().join("index.db"));
    let mut session = CodeModeSession::new(config.clone());
    session
        .call("index_repo", json!({"force": false}))
        .expect("index");
    (temp, config)
}

/// INTENT: byte-identical twin repos (the never-faulted oracle): twin A is
/// faulted and repaired, twin B is pristine, and recovery proves them equal.
/// Search/read/edit values are root-relative, so full-Value equality across
/// tempdirs is well-defined; `index_status` embeds absolute paths, so twins
/// compare [`status_counts`] only.
pub fn twin_repos(token: &str) -> (TempDir, SessionConfig, TempDir, SessionConfig) {
    let (a_temp, a_config) = indexed_codemode_repo(token);
    let (b_temp, b_config) = indexed_codemode_repo(token);
    (a_temp, a_config, b_temp, b_config)
}

/// INTENT: host-persisted plan-record loader. Layers: IO failure -> `Other`,
/// syntax failure -> `Json`, well-formed values delegate to `parse_plan`.
pub fn load_plan_file(path: &Path) -> Result<Plan, CallError> {
    let bytes = std::fs::read(path).map_err(|e| CallError::Other(e.into()))?;
    let value: Value = serde_json::from_slice(&bytes).map_err(CallError::from)?;
    parse_plan(&value)
}

/// INTENT: host-persisted batch-record loader. Layers: IO failure -> `Other`,
/// syntax failure -> `Json`, well-formed values delegate to batch validation.
pub fn load_batch_file(path: &Path) -> Result<BatchRequest, CallError> {
    let bytes = std::fs::read(path).map_err(|e| CallError::Other(e.into()))?;
    let request: BatchRequest = serde_json::from_slice(&bytes).map_err(CallError::from)?;
    Ok(request)
}

/// INTENT: read the SQLite `user_version` stamp (header bytes 60..64,
/// big-endian): schema-version surgery reads back the live stamp.
pub fn db_user_version(db: &Path) -> u32 {
    let bytes = std::fs::read(db).expect("read db");
    u32::from_be_bytes(bytes[60..64].try_into().expect("full header"))
}

/// INTENT: write the SQLite `user_version` stamp: plants future/stale schema
/// versions for the refuse/migrate contract tests.
pub fn set_db_user_version(db: &Path, version: u32) {
    let mut bytes = std::fs::read(db).expect("read db");
    assert!(bytes.len() > 4096, "fixture index must exceed one page");
    bytes[60..64].copy_from_slice(&version.to_be_bytes());
    std::fs::write(db, &bytes).expect("write db");
}

/// INTENT: operator removal — delete the db plus its SQLite sidecars: the
/// documented repair path for dead index bytes.
pub fn remove_db_with_sidecars(db: &Path) {
    std::fs::remove_file(db).expect("remove db");
    crate::fault::remove_sqlite_sidecars(db);
}

/// INTENT: run a serve `script` through a sticky `run_serve` worker on an
/// INDEXED session and return the non-empty raw output lines: the full-function
/// proof transcript (index read + source read + index-free call + Bye).
/// `serve_lines` runs unindexed sessions only, so drills need this indexed
/// counterpart. Panics unless every script line is answered.
pub fn serve_transcript_indexed(config: SessionConfig, script: &[Value]) -> Vec<String> {
    let mut input = String::new();
    for call in script {
        input.push_str(&serde_json::to_string(call).expect("serve line"));
        input.push('\n');
    }
    let mut out = Vec::new();
    run_serve(config, Cursor::new(input), &mut out).expect("serve runs");
    let text = String::from_utf8(out).expect("utf8");
    let lines: Vec<String> = text
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    assert_eq!(
        lines.len(),
        script.len(),
        "serve transcript must answer every script line: {text}"
    );
    lines
}

/// INTENT: root-independent `index_status` projection (count fields only):
/// the cross-tempdir comparator for twin repos, whose absolute paths differ
/// by construction.
pub fn status_counts(status: &Value) -> Value {
    json!({
        "file_count": status["file_count"],
        "line_count": status["line_count"],
        "symbol_count": status["symbol_count"],
        "caller_count": status["caller_count"],
        "import_count": status["import_count"],
        "semantic_chunk_count": status["semantic_chunk_count"],
    })
}

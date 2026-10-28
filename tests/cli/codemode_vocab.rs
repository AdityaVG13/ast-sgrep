//! F-SG-CODEMODE-VOCAB: the session is the front door — `asgrep serve` (was
//! `codemode-serve`, alias kept) — and its dispatcher carries the CLI
//! ergonomics contract: exact cross-surface vocabulary (MCP tool names)
//! recovers and RUNS, typos recover into read-only tools only, and a
//! near-miss naming a mutating tool teaches instead of running. Truly
//! unknown tools get the catalog pointer, never a bare wall.

use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tempfile::TempDir;

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

/// Run one codemode-batch session over stdin; returns the machine envelope.
fn batch(root: &str, calls: Value) -> (i32, Value, String) {
    let mut child = Command::new(asgrep_bin())
        .args(["codemode-batch", "--requests", "-"])
        .env_remove("ASGREP_NO_EMBED")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn codemode-batch");
    let payload = json!({
        "root": root,
        "use_embed": false,
        "parallel": false,
        "calls": calls,
    })
    .to_string();
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(payload.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let value = serde_json::from_str(&stdout).expect("batch envelope json");
    (out.status.code().unwrap_or(-1), value, stderr)
}

fn vocab_root() -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("corpus");
    fs::create_dir_all(root.join("src")).expect("mkdir");
    fs::write(
        root.join("src/a.rs"),
        "fn alpha_marker() {\n    alpha_marker();\n}\n",
    )
    .expect("write a.rs");
    (temp, root)
}

#[test]
fn serve_front_door_answers_and_alias_survives() {
    let (_temp, root) = vocab_root();
    for subcommand in ["serve", "codemode-serve"] {
        let mut child = Command::new(asgrep_bin())
            .args([
                subcommand,
                "--no-embed",
                "--root",
                root.to_str().expect("utf8 root"),
            ])
            .env_remove("ASGREP_NO_EMBED")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn serve");
        let payload = format!(
            "{}\n{}\n",
            json!({"type":"call","id":"a","tool":"find","args":{"query":"alpha_marker"}}),
            json!({"type":"end"})
        );
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(payload.as_bytes())
            .expect("write stdin");
        let out = child.wait_with_output().expect("wait");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert_eq!(
            out.status.code(),
            Some(0),
            "subcommand={subcommand} stderr={stderr} stdout={stdout}"
        );
        assert!(
            stdout.contains("alpha_marker"),
            "subcommand={subcommand} must answer the find call: {stdout}"
        );
    }
}

#[test]
fn session_vocab_recovers_readonly_and_teaches_mutating() {
    let (_temp, root) = vocab_root();
    let root_str = root.to_str().expect("utf8 root");
    // No index anywhere: ambient freshness builds it on the first find.
    let (code, value, stderr) = batch(
        root_str,
        json!([
            { "id": "kw", "tool": "keyword_search", "args": { "query": "alpha_marker" } },
            { "id": "sem", "tool": "semantic_search", "args": { "query": "alpha_marker" } },
            { "id": "typo", "tool": "fnd", "args": { "query": "alpha_marker" } },
            { "id": "mut", "tool": "editt", "args": { "refs": ["src/a.rs#L1-L1"], "edits": [] } },
            { "id": "unk", "tool": "flurbify", "args": {} },
        ]),
    );
    assert_eq!(code, 0, "stderr={stderr}");
    let results = value["results"].as_array().expect("results");
    let by_id = |id: &str| {
        results
            .iter()
            .find(|r| r["id"] == json!(id))
            .unwrap_or_else(|| panic!("missing result {id}: {value}"))
    };

    // Exact cross-surface vocabulary recovers and runs, stamped with the
    // canonical name.
    let kw = by_id("kw");
    assert_eq!(kw["ok"], json!(true), "keyword_search: {kw}");
    assert_eq!(kw["value"]["recovered"], json!("find"), "kw value: {kw}");
    let sem = by_id("sem");
    assert_eq!(sem["ok"], json!(true), "semantic_search: {sem}");
    assert_eq!(sem["value"]["recovered"], json!("semantic"), "sem: {sem}");

    // Typos recover into read-only tools.
    let typo = by_id("typo");
    assert_eq!(typo["ok"], json!(true), "fnd typo: {typo}");
    assert_eq!(typo["value"]["recovered"], json!("find"), "typo: {typo}");

    // A near-miss naming a mutating tool never runs: it teaches.
    let near_miss = by_id("mut");
    assert_eq!(
        near_miss["ok"],
        json!(false),
        "editt must not run: {near_miss}"
    );
    let error = near_miss["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("mutating") || error.contains("never guessed"),
        "editt error must name the refusal: {error}"
    );

    // Truly unknown tools get the catalog pointer, not a bare wall.
    let unk = by_id("unk");
    assert_eq!(unk["ok"], json!(false), "flurbify: {unk}");
    let unk_error = unk["error"].as_str().unwrap_or_default();
    assert!(
        unk_error.contains("unknown tool") && unk_error.contains("catalog_search"),
        "unknown-tool error must teach the catalog: {unk_error}"
    );
}

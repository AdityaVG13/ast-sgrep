//! F-SG-CODEMODE-AMBIENT: a session answers on CURRENT disk, never on the
//! index as of some earlier explicit refresh. Ambient freshness: search
//! probes the index delta and incrementally updates inline — the agent never
//! calls an index tool. The call that performed maintenance says so
//! (`refreshed.files`); untouched trees add no field (byte-stable responses).
use ast_sgrep_testkit::asgrep_bin;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tempfile::TempDir;

/// Run one codemode-batch session over stdin and return the machine envelope.
/// `calls_or_request`: a calls array, or a full request object (parallel, …).
fn batch(root: &str, calls_or_request: Value) -> (i32, Value, String) {
    let mut request = match calls_or_request {
        Value::Array(calls) => json!({ "calls": calls }),
        value @ Value::Object(_) => value,
        other => panic!("batch expects calls array or request object: {other}"),
    };
    let object = request.as_object_mut().expect("request object");
    object.insert("root".into(), json!(root));
    object.insert("use_embed".into(), json!(false));
    let payload = request.to_string();
    let mut child = Command::new(asgrep_bin())
        .args(["codemode-batch", "--requests", "-"])
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn asgrep");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(payload.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let code = output.status.code().unwrap_or(-1);
    let value = serde_json::from_str(&stdout).unwrap_or(Value::Null);
    (code, value, stderr)
}

fn ambient_root() -> (TempDir, PathBuf) {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("corpus");
    fs::create_dir_all(root.join("src")).expect("mkdir");
    (temp, root)
}

#[test]
fn search_reflects_edits_without_any_index_call() {
    let (_temp, root) = ambient_root();
    let root_str = root.to_str().expect("utf8 root").to_owned();
    fs::write(root.join("src/a.rs"), "fn alpha_marker() {}\n").expect("write");

    // No index exists anywhere in this test: the first search must build it
    // ambiently (notice with files >= 1) and answer.
    let (code, value, stderr) = batch(
        &root_str,
        json!([{ "id": "a", "tool": "find", "args": { "query": "alpha_marker" } }]),
    );
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    let call = &value["results"][0];
    assert_eq!(call["ok"], true, "{call}");
    let call_value = call["value"].to_string();
    assert!(call_value.contains("alpha_marker"), "{call_value}");
    assert!(call_value.contains("src/a.rs"), "{call_value}");
    let refreshed = &call["value"]["refreshed"];
    assert!(
        refreshed["files"].as_u64().unwrap_or(0) >= 1,
        "initial ambient build must report maintenance: {call_value}"
    );

    // Edit after the session closed: a NEW session's search must reflect the
    // edit without any index_repo/refresh call.
    fs::write(root.join("src/a.rs"), "fn beta_marker() {}\n").expect("rewrite");
    let (code, value, stderr) = batch(
        &root_str,
        json!([
            { "id": "b", "tool": "find", "args": { "query": "beta_marker" } },
            { "id": "gone", "tool": "find", "args": { "query": "alpha_marker" } },
        ]),
    );
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    let beta = value["results"][0].clone();
    assert_eq!(beta["ok"], true, "{beta}");
    assert!(
        beta["value"].to_string().contains("beta_marker"),
        "{}",
        beta["value"]
    );
    assert!(
        beta["value"].to_string().contains("src/a.rs"),
        "{}",
        beta["value"]
    );
    // The stale symbol must not survive: zero hits means no hit rows carry
    // the file path.
    let gone = value["results"][1].clone();
    assert_eq!(gone["ok"], true, "{gone}");
    assert!(
        !gone["value"].to_string().contains("src/a.rs"),
        "stale alpha hit survived an edit: {}",
        gone["value"]
    );
}

#[test]
fn unchanged_tree_adds_no_refresh_notice() {
    let (_temp, root) = ambient_root();
    let root_str = root.to_str().expect("utf8 root").to_owned();
    fs::write(root.join("src/a.rs"), "fn gamma_marker() {}\n").expect("write");

    // One session, two identical searches (serial: both calls share the
    // session, exercising the ambient cooldown): the first builds (notice),
    // the second finds an unchanged tree and must not carry a `refreshed`
    // field.
    let (code, value, stderr) = batch(
        &root_str,
        json!({
            "parallel": false,
            "calls": [
                { "id": "one", "tool": "find", "args": { "query": "gamma_marker" } },
                { "id": "two", "tool": "find", "args": { "query": "gamma_marker" } },
            ],
        }),
    );
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    let first = value["results"][0].clone();
    let second = value["results"][1].clone();
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(second["ok"], true, "{second}");
    assert!(
        first["value"]["refreshed"]["files"].as_u64().unwrap_or(0) >= 1,
        "first call performs the initial build: {}",
        first["value"]
    );
    assert!(
        second["value"].get("refreshed").is_none(),
        "unchanged tree must stay notice-free: {}",
        second["value"]
    );
}

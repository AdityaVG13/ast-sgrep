use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
/// Locate asgrep-mcp. `env!(CARGO_BIN_EXE_asgrep-mcp)` unavailable when workspace rustc-wrapper is a shell script.
fn mcp_bin() -> PathBuf {
    if let Some(p) = option_env!("CARGO_BIN_EXE_asgrep-mcp") {
        return PathBuf::from(p);
    }
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(profile)
        .join("asgrep-mcp")
}
fn rpc(payload: Value) -> Value {
    let mut child = Command::new(mcp_bin())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn MCP");
    writeln!(child.stdin.take().unwrap(), "{payload}").unwrap();
    let out = child.wait_with_output().expect("wait MCP");
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("JSON-RPC")
}
#[test]
fn initialize_returns_protocol_and_tools_capability() {
    let r = rpc(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}));
    assert_eq!(r["id"], 1);
    assert_eq!(r["result"]["protocolVersion"], "2024-11-05");
    assert!(r["result"]["capabilities"]["tools"].is_object());
    assert_eq!(r["result"]["serverInfo"]["name"], "ast-sgrep");
    assert!(r.get("error").is_none());
}
#[test]
fn tools_list_exposes_search_and_index_tools() {
    let r = rpc(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}));
    assert_eq!(r["id"], 2);
    let names: Vec<_> = r["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert!(
        names.contains(&"code_search".into())
            && names.contains(&"index_status".into())
            && names.contains(&"index_repo".into())
    );
}
#[test]
fn unknown_method_is_json_rpc_method_not_found() {
    let r = rpc(json!({"jsonrpc":"2.0","id":7,"method":"missing"}));
    assert_eq!(r["id"], 7);
    assert_eq!(r["error"]["code"], -32601);
    assert!(r.get("result").is_none());
}
#[test]
fn unknown_tool_remains_a_tool_error_result() {
    let r = rpc(
        json!({"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"missing","arguments":{}}}),
    );
    assert_eq!(r["id"], 8);
    assert_eq!(r["result"]["isError"], true);
    assert!(r.get("error").is_none());
}

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

fn request(payload: Value) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_asgrep-mcp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn MCP server");
    writeln!(child.stdin.take().unwrap(), "{payload}").unwrap();
    let output = child.wait_with_output().expect("wait for MCP server");
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).expect("valid JSON-RPC response")
}

#[test]
fn unknown_method_is_json_rpc_method_not_found() {
    let response = request(json!({"jsonrpc": "2.0", "id": 7, "method": "missing"}));

    assert_eq!(response["id"], 7);
    assert_eq!(response["error"]["code"], -32601);
    assert!(response.get("result").is_none());
}

#[test]
fn unknown_tool_remains_a_tool_error_result() {
    let response = request(json!({
        "jsonrpc": "2.0",
        "id": 8,
        "method": "tools/call",
        "params": {"name": "missing", "arguments": {}}
    }));

    assert_eq!(response["id"], 8);
    assert_eq!(response["result"]["isError"], true);
    assert!(response.get("error").is_none());
}

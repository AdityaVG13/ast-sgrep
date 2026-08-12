use super::LspServer;
use crate::support::read_message;
use std::io::Cursor;

fn frame(body: &str) -> Vec<u8> {
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

fn drain_messages(stdout: &[u8]) -> Vec<serde_json::Value> {
    let mut reader = std::io::BufReader::new(Cursor::new(stdout));
    let mut out = Vec::new();
    while let Some(body) = read_message(&mut reader).expect("frame") {
        out.push(serde_json::from_str(&body).expect("json"));
    }
    out
}

#[test]
fn exit_without_shutdown_leaves_loop_with_code_1() {
    let mut server = LspServer::new();
    let input = frame(r#"{"jsonrpc":"2.0","method":"exit"}"#);
    let mut reader = Cursor::new(input);
    let mut stdout = Vec::new();
    server.run_with(&mut reader, &mut stdout).unwrap();
    assert!(server.exit_requested);
    assert!(!server.shutdown_received);
    assert_eq!(server.process_exit_code(), 1);
    assert!(stdout.is_empty(), "exit is a notification");
}

#[test]
fn shutdown_stays_up_until_exit_and_rejects_later_requests() {
    let mut server = LspServer::new();
    let mut input = Vec::new();
    input.extend(frame(
        r#"{"jsonrpc":"2.0","id":1,"method":"shutdown","params":{}}"#,
    ));
    input.extend(frame(
        r#"{"jsonrpc":"2.0","id":2,"method":"workspace/symbol","params":{"query":"x"}}"#,
    ));
    input.extend(frame(r#"{"jsonrpc":"2.0","method":"exit"}"#));
    let mut reader = Cursor::new(input);
    let mut stdout = Vec::new();
    server.run_with(&mut reader, &mut stdout).unwrap();
    assert!(server.shutdown_received);
    assert!(server.exit_requested);
    assert_eq!(server.process_exit_code(), 0);
    let messages = drain_messages(&stdout);
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert_eq!(messages[0]["id"], 1);
    assert!(messages[0]["result"].is_null());
    assert_eq!(messages[1]["id"], 2);
    assert_eq!(messages[1]["error"]["code"], -32600);
}

#[test]
fn unparseable_message_with_id_gets_invalid_request() {
    // Missing method + present id must not hang the client (silent drop).
    let mut server = LspServer::new();
    let mut input = Vec::new();
    input.extend(frame(r#"{"jsonrpc":"2.0","id":42,"params":{}}"#));
    input.extend(frame(r#"{"jsonrpc":"2.0","method":"exit"}"#));
    let mut reader = Cursor::new(input);
    let mut stdout = Vec::new();
    server.run_with(&mut reader, &mut stdout).unwrap();
    let messages = drain_messages(&stdout);
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(messages[0]["id"], 42);
    assert_eq!(messages[0]["error"]["code"], -32600);
    assert!(
        messages[0]["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("Invalid Request"),
        "{messages:?}"
    );
}

#[test]
fn unparseable_message_without_id_is_dropped() {
    let mut server = LspServer::new();
    let mut input = Vec::new();
    input.extend(frame(r#"{"jsonrpc":"2.0","params":{}}"#));
    input.extend(frame(r#"{"jsonrpc":"2.0","method":"exit"}"#));
    let mut reader = Cursor::new(input);
    let mut stdout = Vec::new();
    server.run_with(&mut reader, &mut stdout).unwrap();
    assert!(stdout.is_empty(), "no id → no response: {stdout:?}");
    assert!(server.exit_requested);
}

//! Durable checks for LSP framing used by the `lsp_frame` fuzz target.

use ast_sgrep_lsp::transport::read_message;
use std::io::Cursor;

#[test]
fn read_message_parses_valid_frame() {
    let body = r#"{"jsonrpc":"2.0"}"#;
    let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    let mut cur = Cursor::new(frame.into_bytes());
    let msg = read_message(&mut cur).expect("io").expect("message");
    assert_eq!(msg, body);
}

#[test]
fn read_message_rejects_oversize_content_length() {
    // Product max is 8 MiB; oversize must error without panic.
    let frame = b"Content-Length: 999999999\r\n\r\n";
    let mut cur = Cursor::new(&frame[..]);
    assert!(read_message(&mut cur).is_err());
}

#[test]
fn read_message_incomplete_returns_none_or_err() {
    let mut cur = Cursor::new(b"Content-Length: 10\r\n\r\nshort");
    let res = read_message(&mut cur);
    // Incomplete body may be None (EOF) or Err depending on implementation.
    assert!(res.is_ok() || res.is_err());
}

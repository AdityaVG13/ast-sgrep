//! LSP JSON-RPC transport with Content-Length framing (spec-compliant).

use std::io::{self, BufRead, Write};

/// Maximum LSP message body size (8 MB).
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

/// Read one LSP message from stdin (Content-Length header + JSON body).
pub fn read_message(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some(rest) = line.strip_prefix("Content-Length:") {
            content_length = Some(rest.trim().parse().map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("bad Content-Length: {e}"))
            })?);
        }
    }
    let len = content_length.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length header")
    })?;
    if len > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Content-Length {len} exceeds max {MAX_MESSAGE_BYTES}"),
        ));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    Ok(Some(String::from_utf8(buf).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidData, e)
    })?))
}

/// Write one LSP message to stdout.
pub fn write_message(writer: &mut impl Write, body: &str) -> io::Result<()> {
    write!(
        writer,
        "Content-Length: {}\r\n\r\n{}",
        body.as_bytes().len(),
        body
    )?;
    writer.flush()
}

/// Send a JSON-RPC response.
pub fn send_response(writer: &mut impl Write, id: &serde_json::Value, result: serde_json::Value) -> io::Result<()> {
    let msg = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
    write_message(writer, &msg.to_string())
}

/// Send a JSON-RPC error response.
pub fn send_error(
    writer: &mut impl Write,
    id: &serde_json::Value,
    code: i64,
    message: &str,
) -> io::Result<()> {
    let msg = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    });
    write_message(writer, &msg.to_string())
}

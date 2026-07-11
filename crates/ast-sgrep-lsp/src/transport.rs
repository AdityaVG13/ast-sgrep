use std::io::{self, BufRead, Write};
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
pub fn read_message(reader: &mut impl BufRead) -> io::Result<Option<String>> {
    let Some(len) = read_content_length(reader)? else { return Ok(None); };
    if len > MAX_MESSAGE_BYTES { return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Content-Length {len} exceeds max {MAX_MESSAGE_BYTES}"),
        )); }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    String::from_utf8(buf)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
fn read_content_length(reader: &mut impl BufRead) -> io::Result<Option<usize>> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 { return Ok(None); }
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
    content_length
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length header")
        })
        .map(Some)
}
pub fn write_message(writer: &mut impl Write, body: &str) -> io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    writer.flush()
}
pub fn send_response(
    writer: &mut impl Write,
    id: &serde_json::Value,
    result: serde_json::Value,
) -> io::Result<()> {
    write_message(
        writer,
        &serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string(),
    )
}
pub fn send_error(
    writer: &mut impl Write,
    id: &serde_json::Value,
    code: i64,
    message: &str,
) -> io::Result<()> {
    write_message(
        writer,
        &serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "error": { "code": code, "message": message }
        })
        .to_string(),
    )
}

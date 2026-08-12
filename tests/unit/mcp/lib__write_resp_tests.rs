use super::*;
use std::io::{self, Write};

/// Captures writes and whether `flush` was called (pipe hosts require it).
struct FlushProbe {
    buf: Vec<u8>,
    flushed: bool,
}

impl Write for FlushProbe {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.flushed = true;
        Ok(())
    }
}

#[test]
fn write_resp_flushes_after_each_envelope() {
    let mut probe = FlushProbe {
        buf: Vec::new(),
        flushed: false,
    };
    write_resp(
        &mut probe,
        Some(Value::from(1)),
        Some(json!({"ok": true})),
        None,
    )
    .expect("write");
    assert!(
        probe.flushed,
        "MCP NDJSON over a pipe must flush or clients hang"
    );
    let line = std::str::from_utf8(&probe.buf).expect("utf8");
    assert!(line.ends_with('\n'), "NDJSON line terminator required");
    let value: Value = serde_json::from_str(line.trim_end()).expect("json");
    assert_eq!(value["jsonrpc"], "2.0");
    assert_eq!(value["id"], 1);
    assert_eq!(value["result"]["ok"], true);
}

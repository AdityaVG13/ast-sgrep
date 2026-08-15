//! Machine JSON envelopes and pre-parse failure helpers.

use std::io::{self, Read, Write};

pub(crate) const MACHINE_SCHEMA_VERSION: &str = "1.0.0";
const MAX_ERROR_MESSAGE_CHARS: usize = 4_096;

/// Write a line. Agents often pipe through `head`/`jq` and close early;
/// treat broken pipe as success so the process does not panic.
pub(crate) fn write_line(out: &mut impl Write, line: &str) -> io::Result<()> {
    match out
        .write_all(line.as_bytes())
        .and_then(|_| out.write_all(b"\n"))
    {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(e),
    }
}

/// Write a line to stdout (see [`write_line`]).
pub(crate) fn write_stdout_line(line: &str) -> io::Result<()> {
    let mut out = io::stdout().lock();
    write_line(&mut out, line)
}

fn bounded_error_message(message: &str) -> String {
    let mut chars = message.chars();
    let bounded: String = chars.by_ref().take(MAX_ERROR_MESSAGE_CHARS).collect();
    if chars.next().is_some() {
        format!("{bounded}…")
    } else {
        bounded
    }
}

fn machine_value(command: &str, value: impl serde::Serialize) -> anyhow::Result<serde_json::Value> {
    let mut value = serde_json::to_value(value)?;
    let object = match &mut value {
        serde_json::Value::Object(o) => o,
        _ => {
            return Ok(serde_json::json!({
                "schema_version": MACHINE_SCHEMA_VERSION, "tool": "asgrep",
                "command": command, "ok": true, "exit_code": 0, "data": value
            }));
        }
    };
    if command == "status" {
        object
            .entry("embed_backend")
            .or_insert(serde_json::Value::Null);
        object.entry("embed_dim").or_insert(serde_json::Value::Null);
    }
    object.insert("schema_version".into(), MACHINE_SCHEMA_VERSION.into());
    object.insert("tool".into(), "asgrep".into());
    object.insert("command".into(), command.into());
    object.insert("ok".into(), true.into());
    object.insert("exit_code".into(), 0.into());
    Ok(value)
}

pub(crate) fn print_machine_json(
    command: &str,
    value: impl serde::Serialize,
) -> anyhow::Result<()> {
    print_machine_json_with_style(command, value, false, true, 0)
}

/// Machine envelope with explicit ok/exit_code (doctor unhealthy path).
pub(crate) fn print_machine_json_status(
    command: &str,
    value: impl serde::Serialize,
    ok: bool,
    exit_code: i32,
) -> anyhow::Result<()> {
    print_machine_json_with_style(command, value, false, ok, exit_code)
}

pub(crate) fn print_machine_json_with_style(
    command: &str,
    value: impl serde::Serialize,
    compact: bool,
    ok: bool,
    exit_code: i32,
) -> anyhow::Result<()> {
    let mut value = machine_value(command, value)?;
    if let Some(object) = value.as_object_mut() {
        object.insert("ok".into(), ok.into());
        object.insert("exit_code".into(), exit_code.into());
    }
    let payload = if compact {
        serde_json::to_string(&value)?
    } else {
        serde_json::to_string_pretty(&value)?
    };
    write_stdout_line(&payload)?;
    Ok(())
}

pub(crate) fn print_machine_failure(command: &str, kind: &str, exit_code: i32, message: &str) {
    let value = serde_json::json!({
        "schema_version": MACHINE_SCHEMA_VERSION, "tool": "asgrep", "command": command,
        "ok": false, "exit_code": exit_code,
        "error": {"kind": kind, "message": bounded_error_message(message)}
    });
    let payload = serde_json::to_string_pretty(&value).expect("failure envelope serializes");
    // Ignore broken pipe: agents piping JSON may close early.
    let _ = write_stdout_line(&payload);
}

pub(crate) fn raw_machine_output_requested(args: &[std::ffi::OsString]) -> bool {
    args.iter().any(|a| {
        a == "--json"
            || a == "--robot-triage"
            || a == "--format"
            || a.to_str().is_some_and(|raw| raw.starts_with("--format="))
    }) || (args.iter().any(|a| a == "codemod") && args.iter().any(|a| a == "--dry-run"))
        || args.iter().any(|a| {
            // Always-machine commands (success path is JSON even without --json).
            a == "capabilities" || a == "doctor" || a == "codemode-batch"
        })
}

pub(crate) fn raw_command_name(args: &[std::ffi::OsString]) -> &'static str {
    const C: &[&str] = &[
        "index",
        "status",
        "reindex",
        "codemod",
        "search",
        "bench",
        "watch",
        "keyword",
        "semantic",
        "chain",
        "call-path",
        "capabilities",
        "version",
        "robot-docs",
        "doctor",
        "eval",
        "codemode-batch",
        "codemode-serve",
    ];
    args.iter()
        .filter_map(|a| a.to_str())
        .find_map(|a| C.iter().copied().find(|c| a == *c))
        .unwrap_or("search")
}

/// Max bytes for `codemode-batch` request payloads (file or stdin).
/// 4× `MAX_STDIN_LINE_BYTES` keeps batch JSON roomy without unbounded alloc.
pub(crate) const MAX_BATCH_REQUEST_BYTES: u64 = (ast_sgrep_core::MAX_STDIN_LINE_BYTES as u64) * 4;

/// Read UTF-8 from `reader`, never allocating more than `max_bytes + 1`.
/// Rejects payloads larger than `max_bytes` (d2a1.9: stdin must not OOM).
pub(crate) fn read_utf8_capped(mut reader: impl io::Read, max_bytes: u64) -> io::Result<String> {
    let mut buf = String::new();
    reader
        .by_ref()
        .take(max_bytes.saturating_add(1))
        .read_to_string(&mut buf)?;
    if (buf.len() as u64) > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("payload exceeds max {max_bytes} bytes"),
        ));
    }
    Ok(buf)
}

#[cfg(test)]
#[path = "../../../tests/unit/cli/machine.rs"]
mod tests;

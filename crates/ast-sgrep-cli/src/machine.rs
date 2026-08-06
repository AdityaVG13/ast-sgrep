//! Machine JSON envelopes and pre-parse failure helpers.

pub(crate) const MACHINE_SCHEMA_VERSION: &str = "1.0.0";
const MAX_ERROR_MESSAGE_CHARS: usize = 4_096;

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
    if compact {
        println!("{}", serde_json::to_string(&value)?);
    } else {
        println!("{}", serde_json::to_string_pretty(&value)?);
    }
    Ok(())
}

pub(crate) fn print_machine_failure(command: &str, kind: &str, exit_code: i32, message: &str) {
    let value = serde_json::json!({
        "schema_version": MACHINE_SCHEMA_VERSION, "tool": "asgrep", "command": command,
        "ok": false, "exit_code": exit_code,
        "error": {"kind": kind, "message": bounded_error_message(message)}
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).expect("failure envelope serializes")
    );
}

pub(crate) fn raw_machine_output_requested(args: &[std::ffi::OsString]) -> bool {
    args.iter().any(|a| {
        a == "--json"
            || a == "--robot-triage"
            || a == "--format"
            || a.to_str().is_some_and(|raw| raw.starts_with("--format="))
    }) || args.iter().any(|a| a == "capabilities" || a == "doctor")
}

pub(crate) fn raw_command_name(args: &[std::ffi::OsString]) -> &'static str {
    const C: &[&str] = &[
        "index",
        "status",
        "reindex",
        "search",
        "bench",
        "watch",
        "keyword",
        "semantic",
        "chain",
        "capabilities",
        "version",
        "robot-docs",
        "doctor",
        "eval",
    ];
    args.iter()
        .filter_map(|a| a.to_str())
        .find_map(|a| C.iter().copied().find(|c| a == *c))
        .unwrap_or("search")
}

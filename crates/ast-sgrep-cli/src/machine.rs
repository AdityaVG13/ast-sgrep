//! Machine JSON envelopes and pre-parse failure helpers.

use clap::CommandFactory;
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

/// Write a line to stderr (see [`write_line`]). Human diagnostics ride here
/// so script-parsed stdout stays pure result lines.
pub(crate) fn write_stderr_line(line: &str) -> io::Result<()> {
    let mut err = io::stderr().lock();
    write_line(&mut err, line)
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

thread_local! {
    /// Argv-level teaching notes (typo recovery) staged by the process
    /// boundary for machine-mode runs: they ride the JSON envelope instead
    /// of stderr, so `2>/dev/null` pipelines lose nothing (usage failures
    /// stay stderr-silent per the machine contract). Human mode prints
    /// them to stderr at the boundary instead (see `run_process_with`).
    static PENDING_NOTES: std::cell::RefCell<Vec<String>> =
        std::cell::RefCell::new(Vec::new());
}

/// Stage notes for the next machine envelope printed on this thread.
pub(crate) fn set_pending_notes(notes: Vec<String>) {
    PENDING_NOTES.with(|cell| *cell.borrow_mut() = notes);
}

/// Append one runtime note (decided after argv parse, e.g. a root remap) to
/// whatever argv-level notes are already staged.
pub(crate) fn push_pending_note(note: String) {
    PENDING_NOTES.with(|cell| cell.borrow_mut().push(note));
}

fn attach_pending_notes(object: &mut serde_json::Map<String, serde_json::Value>) {
    let notes = PENDING_NOTES.with(|cell| std::mem::take(&mut *cell.borrow_mut()));
    if !notes.is_empty() {
        if let Ok(value) = serde_json::to_value(notes) {
            object.insert("notes".into(), value);
        }
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
        attach_pending_notes(object);
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
    let mut value = serde_json::json!({
        "schema_version": MACHINE_SCHEMA_VERSION, "tool": "asgrep", "command": command,
        "ok": false, "exit_code": exit_code,
        "error": {"kind": kind, "message": bounded_error_message(message)}
    });
    if let Some(object) = value.as_object_mut() {
        attach_pending_notes(object);
    }
    let payload = serde_json::to_string_pretty(&value).expect("failure envelope serializes");
    // Ignore broken pipe: agents piping JSON may close early.
    let _ = write_stdout_line(&payload);
}

pub(crate) fn raw_machine_output_requested(args: &[std::ffi::OsString]) -> bool {
    let command = raw_command_name(args);
    args.iter().any(|a| {
        a == "--json"
            || a == "-j"
            || a == "--robot-triage"
            || a == "--format"
            || a.to_str().is_some_and(|raw| raw.starts_with("--format="))
    }) || (command == "codemod" && args.iter().any(|a| a == "--dry-run"))
        || crate::cli_args::Commands::is_always_machine_command_name(command.as_str())
}

pub(crate) fn raw_command_name(args: &[std::ffi::OsString]) -> String {
    let parsed = crate::Cli::command()
        // Recover only command provenance; the strict parse still owns refusal.
        .ignore_errors(true)
        .try_get_matches_from(args.iter().cloned())
        .ok()
        .and_then(|matches| matches.subcommand_name().map(str::to_owned));
    if let Some(name) = parsed {
        return name;
    }
    // No recognizable subcommand: a dev-only verb (`bench`/`eval`) in a
    // default build must fail closed under its own name, not dissolve into
    // the bare-search fallback (which would blame search grammar for
    // bench/eval flags). The strict parse still owns refusal; this only
    // fixes provenance so the envelope and the teaching note name the verb
    // the caller actually typed.
    if !cfg!(feature = "dev-tools") {
        if let Some(verb) = verb_position_token(args) {
            if verb == "bench" || verb == "eval" {
                return verb;
            }
        }
    }
    "search".to_owned()
}

/// First positional (the verb slot), skipping global flags and their values.
/// Mirrors the verb scan in `rewrite_search_path_alias`: option values and
/// later positionals must not promote a query/root into a command.
fn verb_position_token(args: &[std::ffi::OsString]) -> Option<String> {
    let parser = crate::Cli::command();
    let mut value_pending = false;
    for arg in args.iter().skip(1) {
        if value_pending {
            value_pending = false;
            continue;
        }
        if arg == "--" {
            break;
        }
        let Some(text) = arg.to_str() else {
            break;
        };
        if let Some(option) = text.strip_prefix("--") {
            let (name, attached) = option
                .split_once('=')
                .map_or((option, false), |(name, _)| (name, true));
            value_pending = !attached
                && parser.get_arguments().any(|spec| {
                    spec.get_long() == Some(name) && spec.get_action().takes_values()
                });
            continue;
        }
        if text.starts_with('-') && text != "-" {
            continue;
        }
        return Some(text.to_owned());
    }
    None
}

/// Max bytes for `codemode-batch` request payloads (file or stdin).
/// 4× `MAX_STDIN_LINE_BYTES` keeps batch JSON roomy without unbounded alloc.
pub(crate) const MAX_BATCH_REQUEST_BYTES: u64 = (ast_sgrep_core::MAX_STDIN_LINE_BYTES as u64) * 4;

/// Read UTF-8 from `reader`, retaining at most `max_bytes + 1` input bytes.
/// Rejects payloads larger than `max_bytes`; this is not a peak-memory bound.
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

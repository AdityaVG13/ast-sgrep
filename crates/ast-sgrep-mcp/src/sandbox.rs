//! Sandboxed project-root file reads for MCP `code_read`.
//! Extracted from `lib.rs` (EXP-004). Leaf path/IO helpers only;
//! JSON-RPC dispatch, `McpServer`, and `SearcherCache` stay in `lib`.

use anyhow::Context;
use serde_json::{json, Value};
use std::fs::File;
use std::io::{BufRead, Read};
use std::path::{Component, Path};

const MAX_SCAN_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) fn parse_node_id(id: &str) -> anyhow::Result<(&str, usize, usize)> {
    let (file, range) = id
        .rsplit_once("#L")
        .context("node ID must end in #Lstart-Lend")?;
    let (start_raw, end_raw) = range
        .split_once("-L")
        .context("node ID must end in #Lstart-Lend")?;
    let start = start_raw
        .parse::<u32>()
        .context("invalid node start line")?;
    let end = end_raw.parse::<u32>().context("invalid node end line")?;
    anyhow::ensure!(
        start > 0 && end >= start && start_raw == start.to_string() && end_raw == end.to_string(),
        "invalid or noncanonical node line range"
    );
    let start = start as usize;
    let end = end as usize;
    anyhow::ensure!(!file.is_empty(), "node ID file is empty");
    anyhow::ensure!(
        Path::new(file)
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir)),
        "node ID must be a relative project path"
    );
    Ok((file, start, end))
}

pub(crate) fn same_opened_file(expected: &std::fs::Metadata, actual: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        expected.dev() == actual.dev() && expected.ino() == actual.ino()
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        expected.volume_serial_number().is_some()
            && expected.volume_serial_number() == actual.volume_serial_number()
            && expected.file_index().is_some()
            && expected.file_index() == actual.file_index()
    }
    #[cfg(not(any(unix, windows)))]
    {
        expected.len() == actual.len() && expected.modified().ok() == actual.modified().ok()
    }
}

pub(crate) fn truncate_chars(value: &str, max_chars: usize) -> (&str, bool) {
    match value.char_indices().nth(max_chars) {
        Some((byte, _)) => (&value[..byte], true),
        None => (value, false),
    }
}

pub(crate) fn read_node(
    root: &Path,
    id: &str,
    context_lines: usize,
    max_chars: usize,
) -> anyhow::Result<Value> {
    let (file, requested_start, requested_end) = parse_node_id(id)?;
    let unresolved = root.join(file);
    anyhow::ensure!(unresolved.starts_with(root), "node ID escapes project root");
    let canonical = unresolved
        .canonicalize()
        .context("canonicalize node file")?;
    anyhow::ensure!(canonical.starts_with(root), "node ID escapes project root");
    let expected = canonical.metadata().context("stat node file")?;
    anyhow::ensure!(
        expected.is_file(),
        "node ID does not reference a regular file"
    );
    let handle = File::open(&canonical).context("open node file")?;
    let actual = handle.metadata().context("stat opened node file")?;
    anyhow::ensure!(
        same_opened_file(&expected, &actual),
        "node file changed while opening"
    );
    let reopened = unresolved.canonicalize().context("recheck node file")?;
    anyhow::ensure!(
        reopened == canonical && reopened.starts_with(root),
        "node file changed while opening"
    );
    let start = requested_start.saturating_sub(context_lines).max(1);
    let wanted_end = requested_end.saturating_add(context_lines);
    let (selected, total_lines) = scan_line_window(handle, start, wanted_end)?;
    anyhow::ensure!(
        requested_start <= total_lines && requested_end <= total_lines,
        "node line range is beyond end of file"
    );
    let end = wanted_end.min(total_lines);
    let selected = selected.join("\n");
    let (content, truncated) = truncate_chars(&selected, max_chars);
    Ok(json!({
        "id": id,
        "file": file,
        "lines": {"start": start, "end": end},
        "content": content,
        "truncated": truncated
    }))
}

/// Scan a file handle for lines in `[start, wanted_end]`. TOCTOU checks stay in `read_node`.
pub(crate) fn scan_line_window(
    handle: File,
    start: usize,
    wanted_end: usize,
) -> anyhow::Result<(Vec<String>, usize)> {
    let mut reader = std::io::BufReader::new(handle.take(MAX_SCAN_BYTES + 1));
    let mut line_number = 1usize;
    let mut total_lines = 0usize;
    let mut scanned_bytes = 0u64;
    let mut selected = Vec::new();
    loop {
        let mut bytes = Vec::new();
        let count = reader
            .read_until(b'\n', &mut bytes)
            .context("read node file")?;
        if count == 0 {
            if line_number == 1 {
                total_lines = 1;
                if start <= 1 && wanted_end >= 1 {
                    selected.push(String::new());
                }
            }
            break;
        }
        scanned_bytes = scanned_bytes.saturating_add(count as u64);
        anyhow::ensure!(
            scanned_bytes <= MAX_SCAN_BYTES,
            "node file exceeds scan limit"
        );
        if bytes.last() == Some(&b'\n') {
            bytes.pop();
        }
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        let line = String::from_utf8(bytes).context("node file is not valid UTF-8")?;
        total_lines = line_number;
        if line_number >= start && line_number <= wanted_end {
            selected.push(line);
        }
        if line_number >= wanted_end {
            break;
        }
        line_number += 1;
    }
    Ok((selected, total_lines))
}

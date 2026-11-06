use anyhow::anyhow;
use ast_sgrep_plugins::OutputFormat;
use serde_json::Value;
use std::io::Write;

use crate::tools::CallError;

use super::MAX_CALL_RESPONSE_BYTES;

#[derive(Default)]
struct CountingWriter(usize);

impl Write for CountingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(buf.len());
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Count JSON wire bytes without allocating a serialized buffer.
/// Adapters keep their own envelope accounting and serialization-error policy.
pub fn encoded_len(value: &impl serde::Serialize) -> Result<usize, serde_json::Error> {
    let mut writer = CountingWriter::default();
    serde_json::to_writer(&mut writer, value)?;
    Ok(writer.0)
}

/// Admit the final wire value before publishing it or committing an edit.
/// Conservative `json_size_bound` runs first; exact `encoded_len` only when
/// that bound exceeds the session cap.
pub(crate) fn admit_response(value: &Value) -> Result<(), CallError> {
    if json_size_bound(value) > MAX_CALL_RESPONSE_BYTES
        && encoded_len(value)? > MAX_CALL_RESPONSE_BYTES
    {
        return Err(CallError::Other(anyhow!(
            "codemode response exceeds {MAX_CALL_RESPONSE_BYTES} bytes"
        )));
    }
    Ok(())
}

/// Conservative JSON wire-size upper bound (every string byte as `\u00XX`).
/// Exact `encoded_len` runs only when this bound exceeds the session cap.
fn json_size_bound(value: &Value) -> usize {
    match value {
        Value::Null => 4,
        Value::Bool(true) => 4,
        Value::Bool(false) => 5,
        Value::Number(n) => n.to_string().len(),
        Value::String(s) => 2usize.saturating_add(s.len().saturating_mul(6)),
        Value::Array(items) => {
            let inner: usize = items.iter().map(json_size_bound).sum();
            2usize
                .saturating_add(inner)
                .saturating_add(items.len().saturating_sub(1))
        }
        Value::Object(map) => {
            let inner: usize = map
                .iter()
                .map(|(k, v)| {
                    2usize
                        .saturating_add(k.len().saturating_mul(6))
                        .saturating_add(1)
                        .saturating_add(json_size_bound(v))
                })
                .sum();
            2usize
                .saturating_add(inner)
                .saturating_add(map.len().saturating_sub(1))
        }
    }
}

pub(super) fn search_render_key(
    query: &str,
    limit: usize,
    format: OutputFormat,
    excerpt_lines: usize,
    semantic_only: bool,
    lang: Option<&str>,
) -> String {
    let fmt = match format {
        OutputFormat::Native => "n",
        OutputFormat::GitHub => "gh",
        OutputFormat::GitLab => "gl",
        OutputFormat::Agent => "a",
        OutputFormat::AgentCapsule => "c",
        OutputFormat::Compact => "k",
    };
    format!(
        "{query}\0{limit}\0{fmt}\0{excerpt_lines}\0{}\0{}",
        u8::from(semantic_only),
        lang.unwrap_or("")
    )
}

pub(super) fn ensure_render_input_bounded(
    response: &ast_sgrep_core::SearchResponse,
    format: OutputFormat,
    excerpt_lines: usize,
) -> anyhow::Result<()> {
    let mut bytes = response.query.len().saturating_mul(4);
    for hit in &response.hits {
        // Metadata is repeated in refs, follow-up hints, and reason strings.
        bytes = bytes.saturating_add(hit.file.len().saturating_mul(2));
        for value in [
            hit.symbol.as_deref(),
            hit.caller.as_deref(),
            hit.callee.as_deref(),
            hit.language.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            bytes = bytes.saturating_add(value.len().saturating_mul(4));
        }
        bytes = bytes.saturating_add(match format {
            OutputFormat::AgentCapsule if excerpt_lines == 0 => 4 * 121,
            OutputFormat::AgentCapsule => excerpt_prefix_bytes(&hit.excerpt, excerpt_lines),
            _ => hit.excerpt.len(),
        });
        if bytes > MAX_CALL_RESPONSE_BYTES {
            return Err(anyhow!(
                "codemode response source exceeds {MAX_CALL_RESPONSE_BYTES} bytes"
            ));
        }
    }
    Ok(())
}

pub(super) fn excerpt_prefix_bytes(excerpt: &str, lines: usize) -> usize {
    excerpt
        .lines()
        .take(lines)
        .enumerate()
        .fold(0usize, |total, (index, line)| {
            total
                .saturating_add(usize::from(index > 0))
                .saturating_add(line.len())
        })
}

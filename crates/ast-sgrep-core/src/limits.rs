//! Shared output-limit clamps used by CLI, MCP, and LSP (cross-surface-0f7r).

/// Hard ceiling for `--limit` / agent search result counts.
pub const MAX_OUTPUT_RESULTS: usize = 1000;
/// Hard ceiling for excerpt line budgets.
pub const MAX_EXCERPT_LINES: usize = 100;
/// Hard ceiling for source text retained by one search hit.
pub const MAX_SEARCH_HIT_EXCERPT_BYTES: usize = 64 * 1024;
/// Default agent/MCP soft ceiling (stricter than the hard max).
pub const DEFAULT_AGENT_LIMIT: usize = 100;
/// Hard ceiling for search query length (characters). Shared with MCP schema.
pub const MAX_QUERY_CHARS: usize = 4_096;
/// Hard ceiling for `regex:` pattern length (characters) before compile.
pub const MAX_REGEX_PATTERN_CHARS: usize = 4_096;
/// Hard ceiling for `file_filter` glob patterns.
pub const MAX_FILE_FILTER_CHARS: usize = 1_024;
/// Hard ceiling for a single NDJSON / JSON-RPC line on stdio agent surfaces.
pub const MAX_STDIN_LINE_BYTES: usize = 1_048_576;

/// Clamp a requested limit into `1..=MAX_OUTPUT_RESULTS`, falling back to `default`
/// when `requested` is `None` or zero.
pub fn clamp_output_limit(requested: Option<usize>, default: usize) -> usize {
    let base = requested.filter(|n| *n > 0).unwrap_or(default.max(1));
    base.clamp(1, MAX_OUTPUT_RESULTS)
}

/// Stricter agent-surface clamp (`1..=DEFAULT_AGENT_LIMIT`).
pub fn clamp_agent_limit(requested: Option<usize>, default: usize) -> usize {
    let base = requested.filter(|n| *n > 0).unwrap_or(default.max(1));
    base.clamp(1, DEFAULT_AGENT_LIMIT)
}

/// Reject oversize search queries. Empty is allowed (mode parsers treat it as no hits).
pub fn validate_query_len(query: &str) -> Result<(), String> {
    let n = query.chars().count();
    if n > MAX_QUERY_CHARS {
        return Err(format!(
            "query exceeds maximum of {MAX_QUERY_CHARS} characters ({n})"
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/unit/core/limits.rs"]
mod tests;

//! Shared output-limit clamps used by CLI, MCP, and LSP (cross-surface-0f7r).

/// Hard ceiling for `--limit` / agent search result counts.
pub const MAX_OUTPUT_RESULTS: usize = 1000;
/// Hard ceiling for excerpt line budgets.
pub const MAX_EXCERPT_LINES: usize = 100;
/// Default agent/MCP soft ceiling (stricter than the hard max).
pub const DEFAULT_AGENT_LIMIT: usize = 100;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_to_hard_ceiling() {
        assert_eq!(clamp_output_limit(Some(0), 16), 16);
        assert_eq!(clamp_output_limit(None, 16), 16);
        assert_eq!(clamp_output_limit(Some(50), 16), 50);
        assert_eq!(clamp_output_limit(Some(10_000), 16), MAX_OUTPUT_RESULTS);
        assert_eq!(clamp_agent_limit(Some(500), 16), DEFAULT_AGENT_LIMIT);
    }
}

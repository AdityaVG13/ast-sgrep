//! Clap value parsers for CLI args (split from `cli_args.rs`; bodies verbatim).

use std::path::PathBuf;

use crate::cli_args::PreviewMode;

pub(crate) const MAX_OUTPUT_RESULTS: usize = ast_sgrep_core::MAX_OUTPUT_RESULTS;
pub(crate) const MAX_EXCERPT_LINES: usize = ast_sgrep_core::MAX_EXCERPT_LINES;
pub(crate) const DEFAULT_SNIPPET_TOKENS: usize = 96;
pub(crate) const DEFAULT_RESPONSE_SNIPPET_TOKENS: usize = 768;
pub(crate) const MAX_SNIPPET_TOKENS: usize = 4_096;
pub(crate) const MAX_RESPONSE_SNIPPET_TOKENS: usize = 65_536;

pub(crate) fn parse_bounded_usize(raw: &str, maximum: usize, name: &str) -> Result<usize, String> {
    let value = raw
        .parse::<usize>()
        .map_err(|_| format!("{name} must be a non-negative integer"))?;
    if value > maximum {
        return Err(format!("{name} must not exceed {maximum}"));
    }
    Ok(value)
}

/// An unrecognized durability value is a hard error, never a silent
/// downgrade to a weaker profile.
pub(crate) fn parse_durability(raw: &str) -> Result<ast_sgrep_core::Durability, String> {
    ast_sgrep_core::Durability::parse(raw).ok_or_else(|| {
        format!("unknown durability '{raw}' (expected strict, balanced, or fast-unsafe)")
    })
}

/// A user-typed `--index-path` (or `ASGREP_INDEX_PATH`) resolves against the
/// shell CWD at the arg boundary, so every subcommand agrees on one absolute
/// path. Core's `try_index_db_path` root-joins RELATIVE paths, which is the
/// library contract — the CLI must never hand it a relative one.
pub(crate) fn parse_cwd_absolute_index_path(raw: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        return Ok(path);
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|error| format!("cannot resolve relative --index-path: {error}"))
}

/// m38g: bounded like the other token knobs so a hostile value cannot make the
/// renderer allocate without limit.
pub(crate) fn parse_budget_tokens(raw: &str) -> Result<usize, String> {
    parse_bounded_usize(raw, MAX_RESPONSE_SNIPPET_TOKENS, "--budget-tokens")
}

pub(crate) fn parse_preview_mode(raw: &str) -> Result<PreviewMode, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" => Ok(PreviewMode::None),
        "short" => Ok(PreviewMode::Short),
        "full" => Ok(PreviewMode::Full),
        other => Err(format!(
            "invalid --preview '{other}' (expected none|short|full)"
        )),
    }
}

pub(crate) fn parse_output_limit(raw: &str) -> Result<usize, String> {
    parse_bounded_usize(raw, MAX_OUTPUT_RESULTS, "--limit")
}

pub(crate) fn parse_excerpt_lines(raw: &str) -> Result<usize, String> {
    parse_bounded_usize(raw, MAX_EXCERPT_LINES, "--excerpt-lines")
}

pub(crate) fn parse_output_format(raw: &str) -> Result<String, String> {
    const FORMATS: &[&str] = &[
        "native",
        "agent",
        "agent-capsule",
        "compact",
        "github",
        "gitlab",
    ];
    let lower = raw.to_ascii_lowercase();
    if ast_sgrep_plugins::OutputFormat::parse(&lower).is_some() {
        return Ok(lower);
    }
    // Common agent mistakes: think format is "json" / typo "jason" — prefer compact for LLM use.
    let suggestion = match lower.as_str() {
        "json" | "jsno" | "josn" | "jason" | "ndjson" => Some("compact"),
        "gh" | "github-actions" => Some("github"),
        "gl" => Some("gitlab"),
        "capsule" | "agent_capsule" | "agentcapsule" => Some("agent-capsule"),
        _ => FORMATS
            .iter()
            .copied()
            .filter(|cand| crate::agent::edit_distance(&lower, cand) <= 2)
            .min_by_key(|cand| crate::agent::edit_distance(&lower, cand)),
    };
    let list = FORMATS.join(", ");
    Err(match suggestion {
        Some(s) => format!(
            "invalid --format '{raw}' (did you mean '{s}'?). Try: asgrep --json --format {s} \"query\" .\nAllowed: {list}"
        ),
        None => format!(
            "invalid --format '{raw}'. Try: asgrep --json --format compact \"query\" .\nAllowed: {list}"
        ),
    })
}

pub(crate) fn parse_snippet_tokens(raw: &str) -> Result<usize, String> {
    parse_bounded_usize(raw, MAX_SNIPPET_TOKENS, "--snippet-tokens")
}

pub(crate) fn parse_response_snippet_tokens(raw: &str) -> Result<usize, String> {
    parse_bounded_usize(
        raw,
        MAX_RESPONSE_SNIPPET_TOKENS,
        "--response-snippet-tokens",
    )
}

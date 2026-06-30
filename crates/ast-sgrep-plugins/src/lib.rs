//! Output format adapters for CI, GitHub, and GitLab integrations.

pub mod github;
pub mod gitlab;

use ast_sgrep_core::SearchResponse;

/// Supported external JSON output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Native ast-sgrep JSON (`query`, `limit`, `hits`).
    Native,
    /// GitHub code search API shape.
    GitHub,
    /// GitLab code search API shape.
    GitLab,
}

impl OutputFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "native" | "asgrep" => Some(Self::Native),
            "github" | "gh" => Some(Self::GitHub),
            "gitlab" | "gl" => Some(Self::GitLab),
            _ => None,
        }
    }
}

/// Format a search response for the chosen integration.
pub fn format_response(response: &SearchResponse, format: OutputFormat) -> serde_json::Value {
    match format {
        OutputFormat::Native => serde_json::to_value(response).unwrap_or_default(),
        OutputFormat::GitHub => github::to_github_json(response),
        OutputFormat::GitLab => gitlab::to_gitlab_json(response),
    }
}

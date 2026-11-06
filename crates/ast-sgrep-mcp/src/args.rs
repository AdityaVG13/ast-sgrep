//! MCP tool-args boundary parses (`impl McpServer`).
//! Extracted from `lib.rs`. Wire validation only; dispatch (`dispatch_tool`,
//! `tools_catalog`) and tool handlers stay in `lib`.

use super::{
    AgentSearchArgs, AgentSearchWire, CodeReadArgs, CodeReadWire, IndexRepoArgs, IndexRepoWire,
    IndexStatusArgs, IndexStatusWire, McpServer, PreviewMode, DEFAULT_READ_CHARS, MAX_AGENT_LIMIT,
    MAX_BUDGET_TOKENS, MAX_CONTEXT_LINES, MAX_READ_CHARS, MAX_READ_REFS,
};
use anyhow::Context;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::path::PathBuf;

impl McpServer {
    pub(crate) fn map_wire_error(err: serde_json::Error) -> anyhow::Error {
        let msg = err.to_string();
        // Preserve the prior unknown-key phrasing agents already see.
        if let Some(rest) = msg.strip_prefix("unknown field `") {
            if let Some(field) = rest.split('`').next() {
                return anyhow::anyhow!("unknown argument: {field}");
            }
        }
        anyhow::anyhow!(msg)
    }

    pub(crate) fn bounded_usize(
        name: &str,
        value: u64,
        minimum: usize,
        maximum: usize,
    ) -> anyhow::Result<usize> {
        let value = usize::try_from(value).with_context(|| format!("{name} must be an integer"))?;
        anyhow::ensure!(
            (minimum..=maximum).contains(&value),
            "{name} must be between {minimum} and {maximum}"
        );
        Ok(value)
    }

    pub(crate) fn resolve_root(&self, root: Option<String>) -> anyhow::Result<PathBuf> {
        let candidate = match root {
            None => self.root.clone(),
            Some(value) => PathBuf::from(value),
        };
        self.sandbox_root(candidate)
    }

    /// Single wire-args boundary parse (F19): every tool args object enters
    /// here, so the `is_object` check, the `Value` clone for `from_value`,
    /// and the unknown-field mapping live in exactly one place.
    pub(crate) fn parse_wire<T>(&self, args: &Value) -> anyhow::Result<T>
    where
        T: DeserializeOwned,
    {
        anyhow::ensure!(args.is_object(), "arguments must be an object");
        serde_json::from_value(args.clone()).map_err(Self::map_wire_error)
    }

    pub(crate) fn parse_agent_search(&self, args: &Value) -> anyhow::Result<AgentSearchArgs> {
        let wire: AgentSearchWire = self.parse_wire(args)?;
        let query = wire.query.trim();
        anyhow::ensure!(
            !query.is_empty() && query.chars().count() <= ast_sgrep_core::MAX_QUERY_CHARS,
            "query must contain 1 to {} characters",
            ast_sgrep_core::MAX_QUERY_CHARS
        );
        let limit = match wire.limit {
            None => self.limit,
            Some(value) => Self::bounded_usize("limit", value, 1, MAX_AGENT_LIMIT)?,
        };
        // m38g: present budget must be in 1..=MAX; absent means no budget.
        let budget_tokens = match wire.budget_tokens {
            None => None,
            Some(value) => Some(Self::bounded_usize(
                "budget_tokens",
                value,
                1,
                MAX_BUDGET_TOKENS,
            )?),
        };
        let file_filter = match wire.file_filter {
            None => None,
            Some(raw) => {
                let trimmed = raw.trim();
                anyhow::ensure!(!trimmed.is_empty(), "file_filter must be non-empty");
                anyhow::ensure!(
                    trimmed.chars().count() <= ast_sgrep_core::MAX_QUERY_CHARS,
                    "file_filter is too long"
                );
                Some(trimmed.to_owned())
            }
        };
        let lang = match wire.lang {
            None => None,
            Some(raw) => {
                let trimmed = raw.trim();
                anyhow::ensure!(!trimmed.is_empty(), "lang must be non-empty");
                Some(trimmed.to_owned())
            }
        };
        let preview = match wire.preview {
            None => PreviewMode::Short,
            Some(raw) => PreviewMode::parse(&raw)?,
        };
        Ok(AgentSearchArgs {
            query: query.to_owned(),
            root: self.resolve_root(wire.root)?,
            limit,
            resend_seen: wire.resend_seen.unwrap_or(false),
            budget_tokens,
            file_filter,
            lang,
            preview,
        })
    }

    pub(crate) fn parse_code_read(&self, args: &Value) -> anyhow::Result<CodeReadArgs> {
        let wire: CodeReadWire = self.parse_wire(args)?;
        anyhow::ensure!(
            !wire.ids.is_empty() && wire.ids.len() <= MAX_READ_REFS,
            "ids must contain 1 to 20 node IDs"
        );
        let context_lines = match wire.context_lines {
            None => 0,
            Some(value) => Self::bounded_usize("context_lines", value, 0, MAX_CONTEXT_LINES)?,
        };
        let max_chars = match wire.max_chars {
            None => DEFAULT_READ_CHARS,
            Some(value) => Self::bounded_usize("max_chars", value, 1, MAX_READ_CHARS)?,
        };
        Ok(CodeReadArgs {
            ids: wire.ids,
            root: self.resolve_root(wire.root)?,
            context_lines,
            max_chars,
        })
    }

    pub(crate) fn parse_index_status(&self, args: &Value) -> anyhow::Result<IndexStatusArgs> {
        let wire: IndexStatusWire = self.parse_wire(args)?;
        Ok(IndexStatusArgs {
            root: self.resolve_root(wire.root)?,
        })
    }

    pub(crate) fn parse_index_repo(&self, args: &Value) -> anyhow::Result<IndexRepoArgs> {
        let wire: IndexRepoWire = self.parse_wire(args)?;
        Ok(IndexRepoArgs {
            root: self.resolve_root(wire.root)?,
            force: wire.force.unwrap_or(false),
        })
    }

    /// Keep MCP tool roots under the configured workspace (v0mg).
    pub(crate) fn sandbox_root(&self, candidate: PathBuf) -> anyhow::Result<PathBuf> {
        let canonical = if candidate.exists() {
            candidate
                .canonicalize()
                .with_context(|| format!("canonicalize root {}", candidate.display()))?
        } else {
            anyhow::bail!(
                "project root does not exist or is not a directory: {}",
                candidate.display()
            );
        };
        anyhow::ensure!(
            canonical.starts_with(&self.root),
            "root {} escapes configured workspace {}",
            canonical.display(),
            self.root.display()
        );
        anyhow::ensure!(
            canonical.is_dir(),
            "project root is not a directory: {}",
            canonical.display()
        );
        Ok(canonical)
    }
}

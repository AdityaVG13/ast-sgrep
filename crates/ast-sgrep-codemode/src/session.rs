//! Warm Code Mode session over `ast-sgrep-core`.

use anyhow::{anyhow, Context};
use ast_sgrep_core::chain::{expand_chain, ChainConfig};
use ast_sgrep_core::{EmbedBackend, IndexOptions, Indexer, SearchOptions, Searcher};
use ast_sgrep_plugins::{format_response_with, OutputFormat};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::tools::{call_tool, CallError};

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub root: PathBuf,
    pub index_path: Option<PathBuf>,
    pub limit: usize,
    pub use_embed: bool,
    /// Default search output: capsule keeps PTC intermediates cheap.
    pub default_format: OutputFormat,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            root: std::env::var("ASGREP_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))),
            index_path: std::env::var("ASGREP_INDEX_PATH").ok().map(PathBuf::from),
            limit: ast_sgrep_core::clamp_output_limit(
                std::env::var("ASGREP_LIMIT")
                    .ok()
                    .and_then(|v| v.parse().ok()),
                SearchOptions::default_limit(),
            ),
            use_embed: std::env::var("ASGREP_NO_EMBED").ok().as_deref() != Some("1"),
            default_format: OutputFormat::AgentCapsule,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct SearcherKey {
    root: PathBuf,
    index_path: Option<PathBuf>,
    /// Opened Searcher limit; reused for any call whose limit is ≤ this.
    open_limit: usize,
    use_embed: bool,
}

/// Stateful façade: warm `Searcher`, budgets, and tool dispatch.
pub struct CodeModeSession {
    config: SessionConfig,
    searcher_cache: Mutex<Option<(SearcherKey, Searcher)>>,
    /// Soft budget: number of index-touching tool calls this session.
    calls: usize,
    pub max_calls: usize,
}

impl CodeModeSession {
    pub fn new(config: SessionConfig) -> Self {
        Self {
            config,
            searcher_cache: Mutex::new(None),
            calls: 0,
            max_calls: 64,
        }
    }

    pub fn from_env() -> Self {
        Self::new(SessionConfig::default())
    }

    pub fn config(&self) -> &SessionConfig {
        &self.config
    }

    pub fn call_count(&self) -> usize {
        self.calls
    }

    /// Dispatch any catalog tool by name.
    pub fn call(&mut self, name: &str, args: Value) -> Result<Value, CallError> {
        self.bump_call().map_err(CallError::from)?;
        call_tool(self, name, args)
    }

    pub(crate) fn bump_call(&mut self) -> anyhow::Result<()> {
        if self.calls >= self.max_calls {
            return Err(anyhow!(
                "codemode call budget exceeded (max_calls={})",
                self.max_calls
            ));
        }
        self.calls += 1;
        Ok(())
    }

    pub(crate) fn invalidate_searcher_cache(&self) {
        if let Ok(mut guard) = self.searcher_cache.lock() {
            *guard = None;
        }
    }

    fn root_arg(&self, args: &Value) -> PathBuf {
        args.get("root")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.config.root.clone())
    }

    fn resolve_format(&self, args: &Value) -> OutputFormat {
        match args.get("format").and_then(|v| v.as_str()) {
            Some("agent") | Some("llm") | Some("ai") => OutputFormat::Agent,
            Some("capsule") | Some("agent-capsule") => OutputFormat::AgentCapsule,
            _ => self.config.default_format,
        }
    }

    fn searcher_for(
        &self,
        root: PathBuf,
        needed_limit: usize,
    ) -> anyhow::Result<std::sync::MutexGuard<'_, Option<(SearcherKey, Searcher)>>> {
        let needed = needed_limit.clamp(1, 500);
        let mut guard = self
            .searcher_cache
            .lock()
            .map_err(|_| anyhow!("searcher cache lock poisoned"))?;
        let reuse = match guard.as_ref() {
            Some((k, _))
                if k.root == root
                    && k.index_path == self.config.index_path
                    && k.use_embed == self.config.use_embed
                    && k.open_limit >= needed =>
            {
                true
            }
            _ => false,
        };
        if !reuse {
            // Open at least as wide as config + this call so later smaller calls reuse.
            let open_limit = needed.max(self.config.limit).clamp(1, 500);
            let searcher = Searcher::new(SearchOptions {
                root: root.clone(),
                index_path: self.config.index_path.clone(),
                limit: open_limit,
                use_embed: self.config.use_embed,
                ..SearchOptions::default()
            })?;
            *guard = Some((
                SearcherKey {
                    root,
                    index_path: self.config.index_path.clone(),
                    open_limit,
                    use_embed: self.config.use_embed,
                },
                searcher,
            ));
        }
        Ok(guard)
    }

    pub(crate) fn search(&mut self, args: &Value) -> anyhow::Result<Value> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .context("query is required")?;
        ast_sgrep_core::validate_query_len(query).map_err(|e| anyhow::anyhow!(e))?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(self.config.limit)
            .clamp(1, 500);
        let semantic_only = args
            .get("semantic_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let excerpt_lines = args
            .get("excerpt_lines")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(0);
        let format = self.resolve_format(args);
        let root = self.root_arg(args);
        let guard = self.searcher_for(root, limit)?;
        let searcher = &guard.as_ref().expect("searcher_for populates cache").1;
        let mut response = if semantic_only {
            searcher.search_semantic(query)?
        } else {
            searcher.search(query)?
        };
        // Searcher may be wider than this call's limit (warm-cache reuse).
        if response.hits.len() > limit {
            response.hits.truncate(limit);
            response.limit = limit;
        }
        Ok(format_response_with(&response, format, excerpt_lines))
    }

    pub(crate) fn chain(&mut self, args: &Value) -> anyhow::Result<Value> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .context("query is required")?;
        ast_sgrep_core::validate_query_len(query).map_err(|e| anyhow::anyhow!(e))?;
        let max_depth = args
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
            .unwrap_or(2)
            .clamp(1, 8);
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(100)
            .clamp(1, 500);
        let top_n = args
            .get("top_n")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(20)
            .clamp(1, 50);
        let root = self.root_arg(args);
        let guard = self.searcher_for(root, self.config.limit)?;
        let searcher = &guard.as_ref().expect("searcher_for populates cache").1;
        let config = ChainConfig {
            max_depth,
            decay_factor: 0.5,
            limit,
            top_n,
        };
        let response = expand_chain(searcher.store(), query, &config)?;
        Ok(serde_json::to_value(response)?)
    }

    pub(crate) fn index_status(&mut self, args: &Value) -> anyhow::Result<Value> {
        let indexer = Indexer::new(IndexOptions {
            root: self.root_arg(args),
            index_path: self.config.index_path.clone(),
            ..IndexOptions::default()
        })?;
        Ok(serde_json::to_value(indexer.store().status()?)?)
    }

    pub(crate) fn index_repo(&mut self, args: &Value) -> anyhow::Result<Value> {
        let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
        let mut indexer = Indexer::new(IndexOptions {
            root: self.root_arg(args),
            index_path: self.config.index_path.clone(),
            embed_backend: EmbedBackend::Auto,
            ..IndexOptions::default()
        })?;
        // Bulk SQLite may commit before sidecar rebuild; invalidate on Ok and Err.
        let result = if force {
            indexer.reindex_all()
        } else {
            indexer.index_all()
        };
        self.invalidate_searcher_cache();
        let stats = result?;
        Ok(json!({
            "ok": true,
            "force": force,
            "stats": stats,
        }))
    }

    #[cfg(test)]
    fn searcher_cache_occupied(&self) -> bool {
        self.searcher_cache
            .lock()
            .map(|g| g.is_some())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod index_err_cache_tests {
    use super::*;
    use ast_sgrep_core::force_sidecar_rebuild_err;
    use tempfile::TempDir;

    #[test]
    fn index_repo_invalidates_searcher_on_index_err() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), "fn hello() {}\n").unwrap();
        let mut session = CodeModeSession::new(SessionConfig {
            root: root.clone(),
            index_path: None,
            limit: 8,
            use_embed: false,
            ..SessionConfig::default()
        });
        // Warm the Searcher cache (drop guard so invalidate can clear the slot).
        drop(
            session
                .searcher_for(root.clone(), 8)
                .expect("warm searcher"),
        );
        assert!(
            session.searcher_cache_occupied(),
            "precondition: searcher cache warm"
        );

        let _fail = force_sidecar_rebuild_err();
        let err = session
            .index_repo(&json!({}))
            .expect_err("forced sidecar rebuild must surface as index_repo Err");
        assert!(
            err.to_string().contains("forced sidecar rebuild failure"),
            "unexpected error: {err}"
        );
        assert!(
            !session.searcher_cache_occupied(),
            "searcher cache must clear on index_repo Err after possible disk mutation"
        );
    }
}

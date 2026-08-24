//! Warm Code Mode session over `ast-sgrep-core`.

use anyhow::{anyhow, Context};
use ast_sgrep_core::chain::{expand_chain, ChainConfig};
use ast_sgrep_core::{
    canonicalize_affected_path, EmbedBackend, IndexOptions, Indexer, SearchOptions, Searcher,
    MAX_EXCERPT_LINES, MAX_INCREMENTAL_PATHS,
};
use ast_sgrep_plugins::{format_response_with, OutputFormat};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::tools::{call_tool, CallError};

/// Maximum encoded value returned by one Code Mode tool call.
pub const MAX_CALL_RESPONSE_BYTES: usize = ast_sgrep_core::MAX_STDIN_LINE_BYTES;

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
    /// On-disk writer stamp observed when this Searcher was opened.
    writer_generation: u64,
}

/// Stateful façade: warm `Searcher`, budgets, and tool dispatch.
pub struct CodeModeSession {
    config: SessionConfig,
    searcher_cache: Mutex<Option<(SearcherKey, Searcher)>>,
    /// Soft budget: number of index-touching tool calls this session.
    calls: usize,
    pub max_calls: usize,
    /// Cooperative cancel for the in-flight `index_repo` walk/prepare.
    cancel: Option<Arc<AtomicBool>>,
}

fn interactive_index_threads() -> usize {
    std::env::var("ASGREP_INDEX_THREADS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or_else(|| {
            // Cancel polling exists, so interactive index can use the host width.
            std::thread::available_parallelism()
                .map(std::num::NonZeroUsize::get)
                .unwrap_or(4)
        })
}

impl CodeModeSession {
    pub fn new(config: SessionConfig) -> Self {
        Self {
            config,
            searcher_cache: Mutex::new(None),
            calls: 0,
            max_calls: 64,
            cancel: None,
        }
    }

    pub fn set_cancel(&mut self, cancel: Option<Arc<AtomicBool>>) {
        self.cancel = cancel;
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
        self.bump_call()?;
        let value = call_tool(self, name, args)?;
        let bytes = encoded_json_len(&value)?;
        if bytes > MAX_CALL_RESPONSE_BYTES {
            return Err(CallError::Other(anyhow!(
                "codemode response exceeds {MAX_CALL_RESPONSE_BYTES} bytes"
            )));
        }
        Ok(value)
    }

    /// True once the sticky call budget is exhausted (br-r49): serve callers
    /// must answer the offending request once and then stop, not flood.
    pub fn exhausted(&self) -> bool {
        self.calls >= self.max_calls
    }

    pub(crate) fn bump_call(&mut self) -> Result<(), CallError> {
        if self.calls >= self.max_calls {
            return Err(CallError::BudgetExhausted(self.max_calls));
        }
        self.calls += 1;
        Ok(())
    }

    pub(crate) fn invalidate_searcher_cache(&self) {
        if let Ok(mut guard) = self.searcher_cache.lock() {
            *guard = None;
        }
    }

    /// Drop warm Searcher when an external writer bumped the on-disk stamp
    /// for the cached Searcher's root (not the session workspace).
    fn sync_writer_generation(&self) -> anyhow::Result<()> {
        let mut guard = self
            .searcher_cache
            .lock()
            .map_err(|_| anyhow!("searcher cache lock poisoned"))?;
        if let Some((key, _)) = guard.as_ref() {
            let current =
                ast_sgrep_core::read_writer_generation(&key.root, key.index_path.as_deref());
            if key.writer_generation != current {
                *guard = None;
            }
        }
        Ok(())
    }

    fn root_arg(&self, args: &Value) -> anyhow::Result<PathBuf> {
        let configured = self.config.root.canonicalize().with_context(|| {
            format!(
                "cannot resolve session root: {}",
                self.config.root.display()
            )
        })?;
        let Some(raw) = args.get("root").and_then(|v| v.as_str()) else {
            return Ok(configured);
        };
        let requested = Path::new(raw);
        let candidate = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            configured.join(requested)
        };
        let candidate = candidate
            .canonicalize()
            .with_context(|| format!("cannot resolve requested root: {}", candidate.display()))?;
        if !candidate.starts_with(&configured) {
            return Err(anyhow!(
                "requested root is outside the configured session root: {}",
                candidate.display()
            ));
        }
        Ok(candidate)
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
        self.sync_writer_generation()?;
        let needed = needed_limit.clamp(1, 500);
        let mut guard = self
            .searcher_cache
            .lock()
            .map_err(|_| anyhow!("searcher cache lock poisoned"))?;
        let reuse = matches!(
            guard.as_ref(),
            Some((key, _))
                if key.root == root
                    && key.index_path == self.config.index_path
                    && key.use_embed == self.config.use_embed
                    && key.open_limit >= needed
                    && key.writer_generation
                        == ast_sgrep_core::read_writer_generation(
                            &key.root,
                            key.index_path.as_deref(),
                        )
        );
        if !reuse {
            // Open at least as wide as config + this call so later smaller calls reuse.
            let open_limit = needed.max(self.config.limit).clamp(1, 500);
            let writer_generation =
                ast_sgrep_core::read_writer_generation(&root, self.config.index_path.as_deref());
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
                    writer_generation,
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
            .unwrap_or(0)
            .min(MAX_EXCERPT_LINES);
        let format = self.resolve_format(args);
        let root = self.root_arg(args)?;
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
        ensure_render_input_bounded(&response, format, excerpt_lines)?;
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
        let root = self.root_arg(args)?;
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
            root: self.root_arg(args)?,
            index_path: self.config.index_path.clone(),
            ..IndexOptions::default()
        })?;
        Ok(serde_json::to_value(indexer.store().status()?)?)
    }

    pub(crate) fn index_repo(&mut self, args: &Value) -> anyhow::Result<Value> {
        let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
        let root = self.root_arg(args)?;
        let paths = incremental_paths(args, &root)?;
        if force && paths.is_some() {
            return Err(anyhow!("index_repo force and paths are mutually exclusive"));
        }
        let mut indexer = Indexer::new(IndexOptions {
            root,
            index_path: self.config.index_path.clone(),
            embed_semantic: self.config.use_embed,
            embed_backend: EmbedBackend::Auto,
            ..IndexOptions::default()
        })?;
        if let Some(cancel) = &self.cancel {
            indexer.set_cancel(Arc::clone(cancel));
        }
        indexer.set_thread_limit(interactive_index_threads());
        // Bulk SQLite may commit before sidecar rebuild; invalidate on Ok and Err.
        let result: anyhow::Result<Value> = (|| {
            if let Some(paths) = paths {
                let stats = indexer.update_paths(&paths)?;
                indexer.flush_deferred_rebuilds()?;
                Ok(json!({
                    "ok": true,
                    "force": false,
                    "targeted": true,
                    "path_count": paths.len(),
                    "stats": {
                        "files_indexed": stats.files_indexed,
                        "files_skipped": stats.files_skipped,
                        "files_removed": stats.files_removed,
                        "files_failed": stats.files_failed,
                    },
                }))
            } else {
                let stats = if force {
                    indexer.reindex_all()?
                } else {
                    indexer.index_all()?
                };
                Ok(json!({
                    "ok": true,
                    "force": force,
                    "targeted": false,
                    "stats": stats,
                }))
            }
        })();
        self.invalidate_searcher_cache();
        result
    }
}

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

pub(crate) fn encoded_len(value: &impl serde::Serialize) -> Result<usize, serde_json::Error> {
    let mut writer = CountingWriter::default();
    serde_json::to_writer(&mut writer, value)?;
    Ok(writer.0)
}

pub(crate) fn encoded_json_len(value: &Value) -> Result<usize, serde_json::Error> {
    encoded_len(value)
}

fn ensure_render_input_bounded(
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

fn excerpt_prefix_bytes(excerpt: &str, lines: usize) -> usize {
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

fn incremental_paths(args: &Value, root: &Path) -> anyhow::Result<Option<Vec<PathBuf>>> {
    let Some(raw_paths) = args.get("paths") else {
        return Ok(None);
    };
    let raw_paths = raw_paths
        .as_array()
        .context("index_repo paths must be an array")?;
    if raw_paths.is_empty() {
        return Err(anyhow!("index_repo paths must be non-empty"));
    }
    if raw_paths.len() > MAX_INCREMENTAL_PATHS {
        return Err(anyhow!(
            "index_repo paths exceeds max {MAX_INCREMENTAL_PATHS}"
        ));
    }

    let root = root
        .canonicalize()
        .with_context(|| format!("cannot resolve index root: {}", root.display()))?;
    let mut seen = HashSet::with_capacity(raw_paths.len());
    let mut paths = Vec::with_capacity(raw_paths.len());
    for raw in raw_paths {
        let raw = raw
            .as_str()
            .context("index_repo paths entries must be strings")?;
        if raw.is_empty() {
            return Err(anyhow!("index_repo paths entries must be non-empty"));
        }
        let path = Path::new(raw);
        if path
            .components()
            .any(|component| component == Component::ParentDir)
        {
            return Err(anyhow!("index_repo path traversal rejected: {raw}"));
        }
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        let canonical = canonicalize_affected_path(&candidate)
            .with_context(|| format!("cannot resolve index path: {}", candidate.display()))?;
        if !canonical.starts_with(&root) {
            return Err(anyhow!(
                "index_repo path is outside project root: {}",
                candidate.display()
            ));
        }
        if seen.insert(canonical.clone()) {
            paths.push(canonical);
        }
    }
    Ok(Some(paths))
}

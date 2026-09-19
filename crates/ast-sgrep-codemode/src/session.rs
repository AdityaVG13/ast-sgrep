//! Warm Code Mode session over `ast-sgrep-core`.

use anyhow::{anyhow, Context};
use ast_sgrep_core::chain::{expand_chain, ChainConfig};
use ast_sgrep_core::{
    canonicalize_affected_path, expand_incremental_path_list, EmbedBackend, IndexOptions, Indexer,
    Language, SearchOptions, Searcher, MAX_EXCERPT_LINES, MAX_INCREMENTAL_PATHS,
};
use ast_sgrep_plugins::{format_response_with, OutputFormat};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
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
const RENDER_CACHE_CAP: usize = 64;

pub struct CodeModeSession {
    config: SessionConfig,
    searcher_cache: Mutex<Option<(SearcherKey, Searcher)>>,
    /// Formatted capsule/agent JSON for identical search args on a warm Searcher.
    /// Cleared whenever the Searcher is dropped (index write or writer stamp).
    render_cache: Mutex<HashMap<String, Value>>,
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

/// Pin a relative Code Mode `index_path` under the session workspace root.
/// Absolute paths stay as given so tests and Pi can keep the index in a temp dir.
fn resolve_session_index_path(root: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        return path;
    }
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    root.join(path)
}

impl CodeModeSession {
    pub fn new(config: SessionConfig) -> Self {
        let mut config = config;
        if let Some(path) = config.index_path.take() {
            // CLI relative indexes resolve against process cwd. A Code Mode
            // session must pin them under the workspace root so Pi cannot
            // write `custom-index/` into whatever directory the agent started in.
            config.index_path = Some(resolve_session_index_path(&config.root, path));
        }
        Self {
            config,
            searcher_cache: Mutex::new(None),
            render_cache: Mutex::new(HashMap::new()),
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
        if json_size_bound(&value) > MAX_CALL_RESPONSE_BYTES {
            let bytes = encoded_json_len(&value)?;
            if bytes > MAX_CALL_RESPONSE_BYTES {
                return Err(CallError::Other(anyhow!(
                    "codemode response exceeds {MAX_CALL_RESPONSE_BYTES} bytes"
                )));
            }
        }
        Ok(value)
    }

    /// True once the sticky call budget is exhausted: serve callers must
    /// answer the offending request once and then stop, not flood.
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
        self.clear_render_cache();
    }

    fn clear_render_cache(&self) {
        if let Ok(mut guard) = self.render_cache.lock() {
            guard.clear();
        }
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

    pub(crate) fn jail_root(&self, args: &Value) -> anyhow::Result<PathBuf> {
        self.root_arg(args)
    }

    pub(crate) fn with_searcher<F, T>(
        &self,
        root: PathBuf,
        needed_limit: usize,
        f: F,
    ) -> anyhow::Result<T>
    where
        F: FnOnce(&Searcher) -> anyhow::Result<T>,
    {
        let guard = self.searcher_for(root, needed_limit, None)?;
        let searcher = &guard.as_ref().expect("searcher_for populates cache").1;
        f(searcher)
    }

    fn searcher_for(
        &self,
        root: PathBuf,
        needed_limit: usize,
        writer_generation: Option<u64>,
    ) -> anyhow::Result<std::sync::MutexGuard<'_, Option<(SearcherKey, Searcher)>>> {
        let needed = needed_limit.clamp(1, 500);
        let writer_generation = writer_generation.unwrap_or_else(|| {
            ast_sgrep_core::read_writer_generation(&root, self.config.index_path.as_deref())
        });
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
                    && key.writer_generation == writer_generation
        );
        if !reuse {
            if let Some((_, old)) = guard.take() {
                old.release_read_snapshot();
            }
            self.clear_render_cache();
            // Open at least as wide as config + this call so later smaller calls reuse.
            let open_limit = needed.max(self.config.limit).clamp(1, 500);
            // Stamps stay off (capsules discard them). Response LRU stays on:
            // sticky Pi sessions repeat needles.
            let searcher = Searcher::new(SearchOptions {
                root: root.clone(),
                index_path: self.config.index_path.clone(),
                limit: open_limit,
                use_embed: self.config.use_embed,
                ..SearchOptions::default()
            })?
            .with_response_stamp(false);
            let _ = searcher.hold_read_snapshot();
            let _ = searcher.warm_search_path();
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
        let raw_query = args
            .get("query")
            .and_then(|v| v.as_str())
            .context(
                "query is required. Call asgrep.search(\"text\") or asgrep.search({ query: \"text\" })",
            )?;
        let query = scoped_search_query(args, raw_query);
        ast_sgrep_core::validate_query_len(&query).map_err(|e| anyhow::anyhow!(e))?;
        let lang_filter = optional_lang(args)?;
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
        let writer_generation =
            ast_sgrep_core::read_writer_generation(&root, self.config.index_path.as_deref());
        self.drop_searcher_if_writer_changed(writer_generation);
        let render_key = search_render_key(
            &query,
            limit,
            format,
            excerpt_lines,
            semantic_only,
            lang_filter.as_deref(),
        );
        if let Some(cached) = self.cached_render(&render_key) {
            return Ok(cached);
        }
        let guard = self.searcher_for(root, limit, Some(writer_generation))?;
        let searcher = &guard.as_ref().expect("searcher_for populates cache").1;
        let mut response = if semantic_only {
            searcher.search_semantic(&query)?
        } else {
            searcher.search(&query)?
        };
        if let Some(lang) = lang_filter.as_deref() {
            response.hits.retain(|hit| hit.language.as_deref() == Some(lang));
        }
        // Searcher may be wider than this call's limit (warm-cache reuse).
        if response.hits.len() > limit {
            response.hits.truncate(limit);
            response.limit = limit;
        }
        ensure_render_input_bounded(&response, format, excerpt_lines)?;
        let value = format_response_with(&response, format, excerpt_lines);
        drop(guard);
        self.store_render(render_key, value.clone());
        Ok(value)
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
        let guard = self.searcher_for(root, self.config.limit, None)?;
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

    fn cached_render(&self, key: &str) -> Option<Value> {
        self.render_cache
            .lock()
            .ok()
            .and_then(|guard| guard.get(key).cloned())
    }

    fn drop_searcher_if_writer_changed(&self, current: u64) {
        let Ok(mut guard) = self.searcher_cache.lock() else {
            return;
        };
        let stale = matches!(
            guard.as_ref(),
            Some((key, _)) if key.writer_generation != current
        );
        if stale {
            if let Some((_, old)) = guard.take() {
                old.release_read_snapshot();
            }
            drop(guard);
            self.clear_render_cache();
        }
    }

    /// Return a previously rendered search capsule without running retrieval.
    ///
    /// Used by NAPI `callNow` so sticky repeats skip the libuv hop. Unique
    /// queries return `None` and must use `call()`.
    pub fn peek_cached_search(&self, args: &Value) -> Option<Value> {
        let raw_query = args.get("query").and_then(|v| v.as_str())?;
        let query = scoped_search_query(args, raw_query);
        ast_sgrep_core::validate_query_len(&query).ok()?;
        let lang_filter = optional_lang(args).ok()?;
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
        let root = self.root_arg(args).ok()?;
        let current =
            ast_sgrep_core::read_writer_generation(&root, self.config.index_path.as_deref());
        {
            let guard = self.searcher_cache.lock().ok()?;
            match guard.as_ref() {
                Some((key, _)) if key.writer_generation == current => {}
                _ => return None,
            }
        }
        let render_key = search_render_key(
            &query,
            limit,
            format,
            excerpt_lines,
            semantic_only,
            lang_filter.as_deref(),
        );
        self.cached_render(&render_key)
    }

    /// Cache hit path for NAPI `callNow`: bump the session budget like `call`.
    pub fn take_cached_search(&mut self, args: &Value) -> Result<Option<Value>, crate::tools::CallError> {
        let Some(value) = self.peek_cached_search(args) else {
            return Ok(None);
        };
        self.bump_call()?;
        Ok(Some(value))
    }

    fn store_render(&self, key: String, value: Value) {
        let Ok(mut guard) = self.render_cache.lock() else {
            return;
        };
        if guard.len() >= RENDER_CACHE_CAP && !guard.contains_key(&key) {
            guard.clear();
        }
        guard.insert(key, value);
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
        // Per-call override: hosts drive freshness refreshes with embeddings off
        // so the first search returns lexical/AST hits in seconds, and build
        // vectors on the explicit index/reindex call. Absent = session default.
        let use_embed = args
            .get("use_embed")
            .and_then(|v| v.as_bool())
            .unwrap_or(self.config.use_embed);
        let root = self.root_arg(args)?;
        let paths = incremental_paths(args, &root)?;
        if force && paths.is_some() {
            return Err(anyhow!("index_repo force and paths are mutually exclusive"));
        }
        let mut indexer = Indexer::new(IndexOptions {
            root,
            index_path: self.config.index_path.clone(),
            embed_semantic: use_embed,
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

/// Conservative JSON wire-size upper bound (every string byte as `\u00XX`).
/// Exact `encoded_json_len` runs only when this bound exceeds the session cap.
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

fn search_render_key(
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
    Ok(Some(expand_incremental_path_list(paths, MAX_INCREMENTAL_PATHS)))
}

fn scoped_search_query(args: &Value, query: &str) -> String {
    let scope = ["in", "file_filter", "fileFilter"]
        .into_iter()
        .find_map(|key| args.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|path| {
            !path.is_empty() && !path.split(['/', '\\']).any(|segment| segment == "..")
        });
    match scope {
        Some(path) if !query.split_whitespace().any(|token| token.starts_with("in:")) => {
            format!("in:{path} {query}")
        }
        _ => query.to_string(),
    }
}

fn optional_lang(args: &Value) -> anyhow::Result<Option<String>> {
    let Some(raw) = args
        .get("lang")
        .or_else(|| args.get("lang_filter"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let Some(language) = Language::parse(raw) else {
        return Err(anyhow!(
            "unknown lang '{raw}' (expected a stored id or extension such as rs, ts, py)"
        ));
    };
    Ok(Some(language.as_str().to_string()))
}

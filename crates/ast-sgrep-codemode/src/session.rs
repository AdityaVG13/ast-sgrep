//! Warm Code Mode session over `ast-sgrep-core`.

use anyhow::{anyhow, Context};
use ast_sgrep_core::chain::{expand_chain, ChainConfig};
use ast_sgrep_core::{
    expand_incremental_path_list, IndexOptions, IndexStore, Indexer,
    Language, SearchOptions, Searcher, MAX_EXCERPT_LINES, MAX_INCREMENTAL_PATHS,
};
use ast_sgrep_plugins::{
    format_response_with, is_bodyless_fn_args_pattern, MissContext, OutputFormat,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::jail::{JailKind, JailResolve, jail_join};
use crate::tools::{call_tool, CallError};

/// Maximum encoded value returned by one Code Mode tool call.
pub const MAX_CALL_RESPONSE_BYTES: usize = ast_sgrep_core::MAX_STDIN_LINE_BYTES;

/// Maximum hits a Code Mode tool call may request. Pinned by the clamp edges
/// in `tests/codemode/ffi_surface.rs` (0/1/500/600) — behavior unchanged, do
/// not raise without updating that pin. (Core `MAX_OUTPUT_RESULTS` is 1000;
/// the session ceiling stays 500 by contract.)
pub(crate) const MAX_SESSION_LIMIT: usize = 500;

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub root: PathBuf,
    pub index_path: Option<PathBuf>,
    pub limit: usize,
    pub use_embed: bool,
    /// Default search output: capsule keeps PTC intermediates cheap.
    pub default_format: OutputFormat,
    /// Ambient freshness: search probes the index delta and incrementally
    /// updates inline, so sessions answer on current disk without any
    /// explicit index call. Hosts that drive freshness themselves opt out.
    pub ambient_freshness: bool,
    /// Minimum interval between ambient freshness probes. Tests set zero to
    /// exercise the probe path without wall-clock sleeps.
    pub ambient_cooldown: std::time::Duration,
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
            ambient_freshness: std::env::var("ASGREP_NO_AMBIENT_FRESHNESS").ok().as_deref()
                != Some("1"),
            ambient_cooldown: std::time::Duration::from_secs(1),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct SearcherKey {
    root: PathBuf,
    index_path: Option<PathBuf>,
    lang_filter: Option<String>,
    /// Opened Searcher limit; reused for any call whose limit is ≤ this.
    open_limit: usize,
    use_embed: bool,
    /// On-disk writer stamp observed when this Searcher was opened.
    writer_generation: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SearcherPreparation {
    Fresh,
    Read,
    Search,
}

struct CachedSearcher {
    key: SearcherKey,
    searcher: Searcher,
    preparation: SearcherPreparation,
}

impl CachedSearcher {
    fn for_read(&mut self) -> &Searcher {
        if self.preparation == SearcherPreparation::Fresh {
            // Anchor the deferred snapshot at open, as warming did before,
            // without materializing search-only caches for a read window.
            let _ = self.searcher.store().index_data_version();
            self.preparation = SearcherPreparation::Read;
        }
        &self.searcher
    }

    fn for_search(&mut self) -> &Searcher {
        if self.preparation != SearcherPreparation::Search {
            // Keep the existing best-effort, once-per-open policy, including
            // failed warm attempts. Retrieval-first opens take the old path.
            let _ = self.searcher.warm_search_path();
            self.preparation = SearcherPreparation::Search;
        }
        &self.searcher
    }
}

/// Stateful façade: warm `Searcher`, budgets, and tool dispatch.
const RENDER_CACHE_CAP: usize = 64;

pub struct CodeModeSession {
    config: SessionConfig,
    searcher_cache: Mutex<Option<CachedSearcher>>,
    /// Formatted capsule/agent JSON for identical search args on a warm Searcher.
    /// Cleared whenever the Searcher is dropped (index write or writer stamp).
    render_cache: Mutex<HashMap<String, Value>>,
    /// Soft budget: number of index-touching tool calls this session.
    calls: usize,
    pub max_calls: usize,
    /// Cooperative cancel for the in-flight `index_repo` walk/prepare.
    cancel: Option<Arc<AtomicBool>>,
    /// Last ambient freshness probe (cooldown-bounded; None = never probed).
    last_ambient: Option<std::time::Instant>,
    /// Writer-opening ambient refreshes (surfaced through `index_status`).
    ambient_refreshes: u64,
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

/// Order-independent stat fingerprint over the indexer's candidate set.
/// Must stay in lockstep with `Indexer::collect_index_candidates` (same dir
/// skips, same ignore gating, same file filter) and with
/// `IndexStore::file_stat_fingerprint` (same path + mtime fold) so a missing
/// file cannot hide from the probe.
/// Returns `None` when any stat is unreadable — the caller then runs the
/// writer instead of treating the tree as unchanged.
///
/// Contract: the ambient writer always indexes with
/// `IndexOptions::default()`, so the probe derives `respect_gitignore` from
/// that same default instead of hardcoding ignore behavior. The candidate
/// predicate (`index_candidate_admitted`) and the fold
/// (`fold_file_stat_identity`) are owned by core and called by both the
/// indexer and this probe. Directory pruning stays walk-local: core's
/// predicate covers file-level admission only (`filter_entry` pruning and
/// the non-UTF8 `rel` rejection need walk state the predicate does not
/// take).
fn ambient_tree_fingerprint(root: &Path) -> Option<u64> {
    use ast_sgrep_core::gitignore::{should_skip_dir, IgnoreMatcher};
    // Same flag the ambient writer uses; see `ambient_refresh`.
    let respect_gitignore = IndexOptions::default().respect_gitignore;
    let ignore = IgnoreMatcher::new(root);
    let mut fp: u64 = 0;
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if should_skip_dir(e.path()) {
                return false;
            }
            if respect_gitignore && e.file_type().is_dir() {
                if let Ok(rel) = e.path().strip_prefix(root) {
                    if !rel.as_os_str().is_empty() && ignore.is_dir_ignored(rel) {
                        return false;
                    }
                }
            }
            true
        })
    {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        if !ast_sgrep_core::index_candidate_admitted(respect_gitignore, &ignore, rel, entry.path()) {
            continue;
        }
        let Ok(rel_str) = ast_sgrep_core::indexed_rel_path(rel) else {
            continue;
        };
        let metadata = entry.metadata().ok()?;
        let modified = metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        ast_sgrep_core::fold_file_stat_identity(
            &mut fp,
            &rel_str,
            modified.as_secs() as i64,
            modified.subsec_nanos(),
        );
    }
    Some(fp)
}

fn indexed_tree_fingerprint(root: &Path, index_path: Option<&Path>) -> Option<u64> {
    let store = IndexStore::open_readonly(root, index_path).ok()?;
    store.file_stat_fingerprint().ok()
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
            last_ambient: None,
            ambient_refreshes: 0,
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

    pub fn config_mut(&mut self) -> &mut SessionConfig {
        &mut self.config
    }

    pub fn call_count(&self) -> usize {
        self.calls
    }

    /// Dispatch any catalog tool by name.
    pub fn call(&mut self, name: &str, args: Value) -> Result<Value, CallError> {
        self.bump_call()?;
        let value = call_tool(self, name, args)?;
        admit_response(&value)?;
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

    pub(crate) fn root_arg(&self, args: &Value) -> anyhow::Result<PathBuf> {
        let Some(raw) = args.get("root").and_then(|v| v.as_str()) else {
            return self.config.root.canonicalize().with_context(|| {
                format!(
                    "cannot resolve session root: {}",
                    self.config.root.display()
                )
            });
        };
        // Per-call root overrides resolve through the jail: `..` rejects even
        // when it would resolve back inside, the workspace root canonicalizes
        // so a symlinked root cannot widen containment, and every refusal
        // keeps this function's legacy message.
        match jail_join(&self.config.root, raw, JailResolve::Strict) {
            Ok(root) => Ok(root),
            Err(err) => Err(match err.kind {
                JailKind::Traversal | JailKind::Outside => anyhow!(
                    "requested root is outside the configured session root: {}",
                    err.candidate.display()
                ),
                JailKind::UnresolvableRoot => anyhow!(
                    "cannot resolve session root: {}",
                    self.config.root.display()
                ),
                JailKind::Unresolvable => {
                    let candidate = err.candidate.display().to_string();
                    err.into_anyhow(format!("cannot resolve requested root: {candidate}"))
                }
                // No legacy arm: the old code never checked UTF-8 (fail closed).
                JailKind::NonUtf8 => anyhow!(err.to_string()),
            }),
        }
    }

    fn resolve_format(&self, args: &Value) -> OutputFormat {
        match args.get("format").and_then(|v| v.as_str()) {
            Some("agent") | Some("llm") | Some("ai") => OutputFormat::Agent,
            Some("capsule") | Some("agent-capsule") => OutputFormat::AgentCapsule,
            _ => self.config.default_format,
        }
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
        let mut guard = self.searcher_for(root, needed_limit, None, None)?;
        let searcher = guard
            .as_mut()
            .expect("searcher_for populates cache")
            .for_read();
        f(searcher)
    }

    fn searcher_for(
        &self,
        root: PathBuf,
        needed_limit: usize,
        lang_filter: Option<&str>,
        writer_generation: Option<u64>,
    ) -> anyhow::Result<std::sync::MutexGuard<'_, Option<CachedSearcher>>> {
        let needed = needed_limit.clamp(1, MAX_SESSION_LIMIT);
        let writer_generation = writer_generation.unwrap_or_else(|| {
            ast_sgrep_core::read_writer_generation(&root, self.config.index_path.as_deref())
        });
        let mut guard = self
            .searcher_cache
            .lock()
            .map_err(|_| anyhow!("searcher cache lock poisoned"))?;
        let reuse = !ast_sgrep_core::env_flag::memo_off("codemode")
            && matches!(
                guard.as_ref(),
                Some(CachedSearcher { key, .. })
                    if key.root == root
                        && key.index_path == self.config.index_path
                        && key.lang_filter.as_deref() == lang_filter
                        && key.use_embed == self.config.use_embed
                        && key.open_limit >= needed
                        && key.writer_generation == writer_generation
            );
        if !reuse {
            if let Some(old) = guard.take() {
                old.searcher.release_read_snapshot();
            }
            self.clear_render_cache();
            // Open at least as wide as config + this call so later smaller calls reuse.
            let open_limit = needed.max(self.config.limit).clamp(1, MAX_SESSION_LIMIT);
            // Stamps stay off (capsules discard them). Response LRU stays on:
            // sticky Pi sessions repeat needles.
            let searcher = Searcher::new(SearchOptions {
                root: root.clone(),
                index_path: self.config.index_path.clone(),
                lang_filter: lang_filter.map(str::to_owned),
                limit: open_limit,
                use_embed: self.config.use_embed,
                ..SearchOptions::default()
            })?
            .with_response_stamp(false);
            let _ = searcher.hold_read_snapshot();
            *guard = Some(CachedSearcher {
                key: SearcherKey {
                    root,
                    index_path: self.config.index_path.clone(),
                    lang_filter: lang_filter.map(str::to_owned),
                    open_limit,
                    use_embed: self.config.use_embed,
                    writer_generation,
                },
                searcher,
                preparation: SearcherPreparation::Fresh,
            });
        }
        Ok(guard)
    }
}

/// Normalized search-render inputs shared by `search` and `peek_cached_search`.
struct SearchRenderParts {
    query: String,
    lang_filter: Option<String>,
    limit: usize,
    semantic_only: bool,
    excerpt_lines: usize,
    format: OutputFormat,
}

impl CodeModeSession {
    /// Six-step render normalization: query, scope, length, lang, limit, mode,
    /// excerpt budget, format. Strict (`?`); `peek_cached_search` degrades to
    /// `None` via `.ok()`.
    fn search_render_parts(&self, args: &Value) -> anyhow::Result<SearchRenderParts> {
        let raw_query = args.get("query").and_then(|v| v.as_str()).context(
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
            .clamp(1, MAX_SESSION_LIMIT);
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
        Ok(SearchRenderParts {
            query,
            lang_filter,
            limit,
            semantic_only,
            excerpt_lines,
            format,
        })
    }

    pub(crate) fn search(&mut self, args: &Value) -> anyhow::Result<Value> {
        let parts = self.search_render_parts(args)?;
        let SearchRenderParts {
            query,
            lang_filter,
            limit,
            semantic_only,
            excerpt_lines,
            format,
        } = parts;
        let root = self.root_arg(args)?;
        // Ambient freshness BEFORE any cache read: an update invalidates the
        // render cache too, so a maintenance notice never serves stale hits.
        let ambient_notice = self.ambient_refresh(&root);
        let writer_generation =
            ast_sgrep_core::read_writer_generation(&root, self.config.index_path.as_deref());
        self.drop_searcher_if_stale(&root, lang_filter.as_deref(), writer_generation);
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
        let mut guard =
            self.searcher_for(root, limit, lang_filter.as_deref(), Some(writer_generation))?;
        let searcher = guard
            .as_mut()
            .expect("searcher_for populates cache")
            .for_search();
        let mut response = if semantic_only {
            searcher.search_semantic(&query)?
        } else {
            searcher.search(&query)?
        };
        // Searcher may be wider than this call's limit (warm-cache reuse).
        // A cut here is as real as the finish layer's (br-tvv) — the
        // envelope must own it too (br-tku). The limit echo is this call's
        // contract in EVERY branch: an untruncated warm result otherwise
        // echoes the searcher's gather limit (the session default), which
        // is not what the caller asked for.
        response.limit = limit;
        if response.hits.len() > limit {
            response.truncated = true;
            response.hits.truncate(limit);
        }
        let indexed_empty = response.hits.is_empty()
            && searcher.store().has_indexed_files().ok() == Some(false);
        ensure_render_input_bounded(&response, format, excerpt_lines)?;
        let mut value = format_response_with(&response, format, excerpt_lines);
        drop(guard);
        // Zero-hit answers classify the miss (CLI parity, 6a3i + br-yjw): a
        // bodyless-args pattern miss teaches its contract instead of a bare
        // empty list. Runs before the render-cache clone so cached takes
        // carry it; why/next derive from the query alone, so the cached
        // render stays deterministic per render key.
        if response.hits.is_empty() {
            let reason = MissContext {
                pattern_miss: is_bodyless_fn_args_pattern(&query),
                indexed_files: if indexed_empty { Some(0) } else { None },
                ..Default::default()
            }
            .reason();
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "why".to_string(),
                    serde_json::Value::String(reason.as_str().to_owned()),
                );
                object.insert(
                    "next".to_string(),
                    serde_json::Value::String(reason.next_step(None)),
                );
            }
        }
        // Cache the maintenance-free render: the notice is per-call state and
        // must never leak into a later identical query via the render cache.
        let cacheable = value.clone();
        if let Some(notice) = ambient_notice {
            if let Some(object) = value.as_object_mut() {
                object.insert("refreshed".to_string(), notice);
            }
        }
        // Cached takes bypass call(), so only admitted wire values may be stored.
        admit_response(&value)?;
        self.store_render(render_key, cacheable);
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
            .unwrap_or(2)
            .clamp(1, 8) as u32;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(100)
            .clamp(1, MAX_SESSION_LIMIT);
        let top_n = args
            .get("top_n")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(20)
            .clamp(1, 50);
        let max_work = match args.get("max_work").filter(|value| !value.is_null()) {
            None => ast_sgrep_core::chain::DEFAULT_CHAIN_WORK,
            Some(value) => {
                let limit = ast_sgrep_core::chain::MAX_CHAIN_WORK as u64;
                let n = value
                    .as_u64()
                    .filter(|n| (1..=limit).contains(n))
                    .with_context(|| format!("max_work must be an integer in 1..={limit}"))?;
                usize::try_from(n).expect("bounded chain work fits usize")
            }
        };
        let root = self.root_arg(args)?;
        let ambient_notice = self.ambient_refresh(&root);
        let mut guard = self.searcher_for(root, self.config.limit, None, None)?;
        let searcher = guard
            .as_mut()
            .expect("searcher_for populates cache")
            .for_search();
        let config = ChainConfig {
            max_depth,
            decay_factor: 0.5,
            limit,
            top_n,
            max_work,
        };
        let response = expand_chain(searcher.store(), query, &config)?;
        let mut value = serde_json::to_value(response)?;
        if let Some(notice) = ambient_notice {
            if let Some(object) = value.as_object_mut() {
                object.insert("refreshed".to_string(), notice);
            }
        }
        Ok(value)
    }

    fn cached_render(&self, key: &str) -> Option<Value> {
        if ast_sgrep_core::env_flag::memo_off("render") {
            return None;
        }
        self.render_cache
            .lock()
            .ok()
            .and_then(|guard| guard.get(key).cloned())
    }

    fn drop_searcher_if_stale(&self, root: &Path, lang_filter: Option<&str>, current: u64) {
        let Ok(mut guard) = self.searcher_cache.lock() else {
            return;
        };
        let stale = matches!(
            guard.as_ref(),
            Some(CachedSearcher { key, .. }) if key.root.as_path() != root
                || key.lang_filter.as_deref() != lang_filter
                || key.writer_generation != current
        );
        if stale {
            if let Some(old) = guard.take() {
                old.searcher.release_read_snapshot();
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
        // Lenient twin of `search`'s preamble: any normalization failure
        // degrades to `None` via `.ok()`.
        let parts = self.search_render_parts(args).ok()?;
        let SearchRenderParts {
            query,
            lang_filter,
            limit,
            semantic_only,
            excerpt_lines,
            format,
        } = parts;
        let root = self.root_arg(args).ok()?;
        let current =
            ast_sgrep_core::read_writer_generation(&root, self.config.index_path.as_deref());
        {
            let guard = self.searcher_cache.lock().ok()?;
            match guard.as_ref() {
                Some(CachedSearcher { key, .. })
                    if key.root == root
                        && key.lang_filter == lang_filter
                        && key.writer_generation == current => {}
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
    pub fn take_cached_search(
        &mut self,
        args: &Value,
    ) -> Result<Option<Value>, crate::tools::CallError> {
        let Some(value) = self.peek_cached_search(args) else {
            return Ok(None);
        };
        self.bump_call()?;
        Ok(Some(value))
    }

    fn store_render(&self, key: String, value: Value) {
        if ast_sgrep_core::env_flag::memo_off("render") {
            return;
        }
        let Ok(mut guard) = self.render_cache.lock() else {
            return;
        };
        if guard.len() >= RENDER_CACHE_CAP && !guard.contains_key(&key) {
            guard.clear();
        }
        guard.insert(key, value);
    }

    /// Ambient freshness: before a search answers, probe the index delta and
    /// incrementally update inline so hits reflect current disk without any
    /// explicit index call. Cooldown-bounded per session. Maintenance failure
    /// falls back to serving the current index (the search itself stays
    /// fail-closed). Returns `{"files", "ms"}` when maintenance ran, else
    /// `None` (unchanged tree adds no field — responses stay byte-stable).
    fn ambient_refresh(&mut self, root: &Path) -> Option<Value> {
        if !self.config.ambient_freshness {
            return None;
        }
        if let Some(last) = self.last_ambient {
            if last.elapsed() < self.config.ambient_cooldown {
                return None;
            }
        }
        self.last_ambient = Some(std::time::Instant::now());
        // Stat-only delta: compare the live tree to the indexed identities.
        // An unchanged tree never opens the writer. No session fingerprint
        // memo — the probe itself is the stat walk plus one readonly open.
        let disk = ambient_tree_fingerprint(root);
        let indexed = indexed_tree_fingerprint(root, self.config.index_path.as_deref());
        if disk.is_some() && disk == indexed {
            return None;
        }
        let started = std::time::Instant::now();
        let mut indexer = Indexer::new(IndexOptions {
            root: root.to_path_buf(),
            index_path: self.config.index_path.clone(),
            embed_semantic: self.config.use_embed,
            ..IndexOptions::default()
        })
        .ok()?;
        indexer.set_thread_limit(interactive_index_threads());
        if let Some(cancel) = &self.cancel {
            indexer.set_cancel(Arc::clone(cancel));
        }
        let stats = indexer.index_all().ok()?;
        self.ambient_refreshes += 1;
        let changed = stats.files_indexed + stats.files_removed;
        if changed == 0 {
            return None;
        }
        self.invalidate_searcher_cache();
        Some(json!({
            "files": changed,
            "ms": started.elapsed().as_millis() as u64,
        }))
    }

    pub(crate) fn index_status(&mut self, args: &Value) -> anyhow::Result<Value> {
        // Writable open on purpose: an empty workspace has no db yet, and
        // hosts (FFI `materialize`, Pi probe) use status to create the schema
        // so later readonly searches return zero hits instead of "index is empty".
        let indexer = Indexer::new(IndexOptions {
            root: self.root_arg(args)?,
            index_path: self.config.index_path.clone(),
            ..IndexOptions::default()
        })?;
        let mut value = serde_json::to_value(indexer.store().status()?)?;
        value["ambient_refreshes"] = serde_json::json!(self.ambient_refreshes);
        Ok(value)
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
        // Targeted reindex tolerates a missing leaf (new/deleted files keep
        // working); `..` and containment still reject with this tool's
        // legacy messages.
        let canonical = match jail_join(&root, raw, JailResolve::LenientLeaf) {
            Ok(path) => path,
            Err(err) => {
                return Err(match err.kind {
                    JailKind::Traversal => {
                        anyhow!("index_repo path traversal rejected: {raw}")
                    }
                    JailKind::UnresolvableRoot => {
                        anyhow!("cannot resolve index root: {}", root.display())
                    }
                    JailKind::Unresolvable => {
                        let candidate = err.candidate.display().to_string();
                        err.into_anyhow(format!("cannot resolve index path: {candidate}"))
                    }
                    JailKind::Outside => anyhow!(
                        "index_repo path is outside project root: {}",
                        err.candidate.display()
                    ),
                    // No legacy arm: the old code never checked UTF-8 (fail closed).
                    JailKind::NonUtf8 => {
                        anyhow!("index_repo path is not valid UTF-8: {raw}")
                    }
                });
            }
        };
        if seen.insert(canonical.clone()) {
            paths.push(canonical);
        }
    }
    Ok(Some(expand_incremental_path_list(
        paths,
        MAX_INCREMENTAL_PATHS,
    )))
}

fn scoped_search_query(args: &Value, query: &str) -> String {
    let scope = ["in", "file_filter", "fileFilter"]
        .into_iter()
        .find_map(|key| args.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|path| !path.is_empty() && !path.split(['/', '\\']).any(|segment| segment == ".."));
    match scope {
        Some(path)
            if !query
                .split_whitespace()
                .any(|token| token.starts_with("in:")) =>
        {
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

#[cfg(test)]
mod ambient_probe_tests {
    use super::*;

    /// The ambient probe and the store must agree on the candidate set when
    /// ignore rules exclude files: gitignored entries are absent from both
    /// fingerprints, so an unchanged tree never triggers a rewrite.
    #[test]
    fn probe_matches_store_on_gitignored_fixture() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        std::fs::write(dir.path().join("kept.rs"), "pub fn kept() {}\n").expect("kept");
        std::fs::write(dir.path().join("ignored.rs"), "pub fn ignored() {}\n")
            .expect("ignored");
        std::fs::create_dir(dir.path().join("ignored_dir")).expect("ignored dir");
        std::fs::write(
            dir.path().join("ignored_dir").join("deep.rs"),
            "pub fn deep() {}\n",
        )
        .expect("deep");
        std::fs::write(
            dir.path().join(".gitignore"),
            "ignored.rs\nignored_dir/\n",
        )
        .expect("gitignore");

        let index_path = dir.path().join("probe.db");
        let mut indexer = Indexer::new(IndexOptions {
            root: dir.path().to_path_buf(),
            index_path: Some(index_path.clone()),
            embed_semantic: false,
            ..IndexOptions::default()
        })
        .expect("indexer");
        indexer.index_all().expect("index");

        let store = IndexStore::open_readonly(dir.path(), Some(index_path.as_path()))
            .expect("store");
        let indexed = store.file_stat_fingerprint().expect("store fingerprint");
        let disk = ambient_tree_fingerprint(dir.path()).expect("probe fingerprint");
        assert_eq!(
            disk, indexed,
            "probe and store disagree on gitignored fixture"
        );
    }
}

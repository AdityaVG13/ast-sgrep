//! MCP stdio server for snippet-first hierarchical ast-sgrep retrieval.
//!
//! Wire protocol is fastmcp-rust over stdio. Tool dispatch stays synchronous
//! on `spawn_blocking` so the reader can accept `ping` and `notifications/cancelled`
//! while search or index holds SQLite.
//!
//! Warm path: a single process reuses one `Searcher` across search-channel calls
//! (invalidated on `index_repo`) so AI agents avoid per-request SQLite open cost.
//!
//! # Byte-stability contract (9q0l)
//!
//! Tool definitions enter the model prompt on every request and are the largest
//! genuinely cacheable region this server controls. `tools/list` must therefore
//! be byte-identical across calls and across processes for a given build:
//!
//! * emit tools in a fixed literal order -- never from a `HashMap` or any other
//!   unordered collection;
//! * keep descriptions and schemas free of per-call data (no paths, counts,
//!   timestamps, or generation numbers);
//! * treat any change here as a cache invalidation for every connected client.
//!
//! Search envelopes are deterministic for the same query and index generation.
//! `serde_json` sorts object keys alphabetically, so key names decide wire
//! order; per-call accounting is named `z*` to keep it in a trailing block.
//!
//! `tools_list_is_byte_identical_across_calls` enforces the first rule.

#![forbid(unsafe_code)]

mod args;
mod compact;
mod envelope;
mod handler;
mod sandbox;

use anyhow::Context;
use ast_sgrep_core::{IndexOptions, IndexStore, Indexer, SearchOptions, Searcher, StoreError};
use ast_sgrep_plugins::{MissContext, to_compact_miss_json};
use envelope::{fnv1a64, render_compact_envelope};
use sandbox::read_node;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Serve MCP over stdio via fastmcp-rust (asupersync runtime).
pub fn serve_stdio_blocking(server: McpServer) -> anyhow::Result<()> {
    handler::serve_stdio(server)
}

const MAX_AGENT_LIMIT: usize = 100;
const MAX_READ_REFS: usize = 20;
const MAX_CONTEXT_LINES: usize = 100;
const DEFAULT_READ_CHARS: usize = 100_000;
const MAX_READ_CHARS: usize = 1_000_000;
const INDEX_REPO_DEADLINE: Duration = Duration::from_secs(600);
/// Upper bound on a client-requested response token budget (m38g).
const MAX_BUDGET_TOKENS: usize = 65_536;
/// Bound on remembered emitted snippets (v972).
const MAX_EMITTED_SNIPPETS: usize = 4_096;
/// Marker replacing a snippet this session already sent for the same id and
/// unchanged content (v972). The agent already has the body in its transcript;
/// `code_read` returns it again if not.
const ELIDED_SNIPPET: &str = "~";

#[derive(Clone, Copy)]
enum AgentSearchMode {
    /// Fused hybrid (CLI `Searcher::search`) — default agent path.
    Hybrid,
    Keyword,
    Ast,
    Semantic,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PreviewMode {
    None,
    #[default]
    Short,
    Full,
}

impl PreviewMode {
    fn parse(raw: &str) -> anyhow::Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(Self::None),
            "short" => Ok(Self::Short),
            "full" => Ok(Self::Full),
            other => anyhow::bail!("preview must be none|short|full (got {other})"),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentSearchWire {
    query: String,
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    limit: Option<u64>,
    #[serde(default)]
    resend_seen: Option<bool>,
    #[serde(default)]
    budget_tokens: Option<u64>,
    #[serde(default)]
    file_filter: Option<String>,
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    preview: Option<String>,
}

/// Trusted agent-search args after MCP tools/call boundary parse.
#[derive(Debug)]
struct AgentSearchArgs {
    query: String,
    root: PathBuf,
    limit: usize,
    resend_seen: bool,
    budget_tokens: Option<usize>,
    file_filter: Option<String>,
    lang: Option<String>,
    preview: PreviewMode,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeReadWire {
    ids: Vec<String>,
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    context_lines: Option<u64>,
    #[serde(default)]
    max_chars: Option<u64>,
}

/// Trusted `code_read` args after boundary parse.
#[derive(Debug)]
struct CodeReadArgs {
    ids: Vec<String>,
    root: PathBuf,
    context_lines: usize,
    max_chars: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexStatusWire {
    #[serde(default)]
    root: Option<String>,
}

/// Trusted `index_status` args after boundary parse.
struct IndexStatusArgs {
    root: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexRepoWire {
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    force: Option<bool>,
}

/// Trusted `index_repo` args after boundary parse.
struct IndexRepoArgs {
    root: PathBuf,
    force: bool,
}

#[derive(Default)]
struct WriterState {
    /// Last observed on-disk writer stamp; mismatch clears the session maps.
    writer_generation: u64,
}

/// Warm Searcher for this MCP process. Invalidated on writer stamp change.
/// `file_filter` is not part of the key: filtered calls open a one-shot Searcher.
struct CachedSearcher {
    root: PathBuf,
    lang_filter: Option<String>,
    open_limit: usize,
    writer_generation: u64,
    searcher: Searcher,
}

/// Compact path-id intern for this writer generation. Built once from indexed
/// paths (sorted, first-id wins) so `code_read` does not reopen the store per id.
type CompactPathIntern = (u64, PathBuf, HashMap<String, String>);

/// Relative compact IDs are meaningful only within their canonical call root.
type RootScopedId = (PathBuf, String);

pub struct McpServer {
    /// Configured workspace root; all tool roots must stay under this path.
    root: PathBuf,
    index_path: Option<PathBuf>,
    limit: usize,
    use_embed: bool,
    use_semantic_only: bool,
    /// Writer stamp observed by this session; cleared after index mutations.
    writer_state: Mutex<WriterState>,
    /// Single-flight lock for index_repo (es7u).
    index_lock: Mutex<()>,
    /// Root-scoped hit id to hash of the snippet already sent this session (v972).
    /// Bounded; cleared when the index changes, so an elision can never point
    /// at content from a previous index generation.
    emitted_snippets: Mutex<HashMap<RootScopedId, u64>>,
    searcher_cache: Mutex<Option<CachedSearcher>>,
    compact_paths: Mutex<Option<CompactPathIntern>>,
}

impl McpServer {
    pub fn from_env() -> anyhow::Result<Self> {
        let root = std::env::var("ASGREP_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let root = root
            .canonicalize()
            .with_context(|| format!("canonicalize ASGREP_ROOT {}", root.display()))?;
        Ok(Self {
            root,
            index_path: std::env::var("ASGREP_INDEX_PATH").ok().map(PathBuf::from),
            limit: std::env::var("ASGREP_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .map(|n| ast_sgrep_core::clamp_agent_limit(Some(n), SearchOptions::default_limit()))
                .unwrap_or_else(|| {
                    ast_sgrep_core::clamp_agent_limit(None, SearchOptions::default_limit())
                }),
            use_embed: !ast_sgrep_core::env_flag::env_flag("ASGREP_NO_EMBED"),
            use_semantic_only: ast_sgrep_core::env_flag::env_flag("ASGREP_SEMANTIC_ONLY"),
            writer_state: Mutex::new(WriterState::default()),
            index_lock: Mutex::new(()),
            emitted_snippets: Mutex::new(HashMap::new()),
            searcher_cache: Mutex::new(None),
            compact_paths: Mutex::new(None),
        })
    }

    pub(crate) fn tools_catalog(&self) -> Value {
        let search_properties = json!({
            "query": {"type": "string", "minLength": 1, "maxLength": ast_sgrep_core::MAX_QUERY_CHARS,
                "description": "Query grammar: plain words, symbols, snake_case fragments ('auth refresh', 'parse_pattern'); channel prefixes defs:SYMBOL, callers:SYMBOL, imports:MODULE, word:TERM, literal:TEXT, regex:PATTERN, pattern:'fn name($$$A)' with metavariables (bare $VAR patterns auto-route to pattern); scope inside the query with in:PATH."},
            "root": {"type": "string", "description": "Project root (defaults to ASGREP_ROOT or cwd)"},
            "limit": {"type": "integer", "minimum": 1, "maximum": MAX_AGENT_LIMIT},
            "resend_seen": {"type": "boolean", "description": "Send snippets already returned this session instead of the ~ marker. Set true only if you do not keep earlier results."},
            "budget_tokens": {"type": "integer", "minimum": 1, "maximum": MAX_BUDGET_TOKENS, "description": "Whole-response token budget. Each hit gains a trailing detail level (metadata|signature|block|full) and omitted source is marked with a gap marker."},
            "file_filter": {"type": "string", "description": "Repository-relative glob restricting hits (e.g. src/**/*.rs)"},
            "lang": {"type": "string", "description": "Language filter: stored id or extension (rs, ts, py, …)"},
            "preview": {"type": "string", "enum": ["none", "short", "full"], "description": "Snippet size: none (ids only), short (default compact), full (larger excerpts)"}
        });
        // r2lu: a declared outputSchema lets a client parse results without
        // reverse-engineering the compact envelope from prose.
        let search_output_schema = json!({
            "type": "object",
            "properties": {
                "v": {"type": "integer", "description": "Envelope schema version"},
                "q": {"type": "string", "description": "Echoed query"},
                "r": {"type": "array", "items": {"type": "string"},
                       "description": "Shared path roots; present only when folding is smaller"},
                "p": {"type": "object",
                       "description": "Path id to project path, or [root_index, suffix] when folded"},
                "h": {"type": "array", "description": "Hits as [id, kind, signal, symbol, snippet] (plus detail level under budget_tokens)",
                       "items": {"type": "array"}},
                "why": {"type": "string", "description": "Miss classification; present only on zero-hit responses"},
                "tried": {"type": "array", "items": {"type": "string"}},
                "next": {"type": "string"},
                "zn": {"type": "integer", "description": "Hit count"},
                "zb": {"type": "array", "items": {"type": "integer"}},
                "zt": {"type": "integer"},
                "ze": {"type": "integer", "description": "Snippets elided as already sent this session"},
                "zd": {"type": "array", "items": {"type": "integer"}, "description": "[token budget, spent]"}
            },
            "required": ["v", "q"]
        });
        let search_tool = |name: &str, description: &str, props: Value| {
            json!({
                "name": name,
                "description": description,
                "inputSchema": {
                    "type": "object",
                    "properties": props,
                    "required": ["query"],
                    "additionalProperties": false
                },
                "outputSchema": search_output_schema.clone(),
                // Spec-current metadata (2025-06-18+): retrieval never mutates
                // index or workspace state, so clients can gate writes without
                // parsing schemas.
                "annotations": {
                    "title": name.replace('_', " "),
                    "readOnlyHint": true,
                    "destructiveHint": false,
                    "idempotentHint": true,
                    "openWorldHint": false
                }
            })
        };
        // kxmc: the compact envelope contract lives in the tool descriptions,
        // which clients send once and cache, instead of being re-explained in
        // every search response.
        const COMPACT_CONTRACT: &str = concat!(
            " Returns a compact envelope: `p` maps path ids to project paths, and each entry of `h` is",
            " [id, kind, signal, symbol, snippet] where id is `<path_id>:<start_line>-<end_line>`.",
            " kind: x=asgrep d=def c=caller g=graph a=anchor i=import p=pattern e=embed.",
            " signal: x=exact t=structural m=semantic. Pass any id straight to code_read for the full body.",
            " A snippet of `~` means this session already sent that exact body for that id:",
            " reuse the earlier result, or call code_read. Pass resend_seen=true to disable."
        );
        let describe = |summary: &str| format!("{summary}{COMPACT_CONTRACT}");
        json!({"tools": [
            search_tool("search", &describe("Prefer over grep when you know a name — even a fragment — or want definitions, call sites, or importers; hits rank definitions-first and their compact IDs expand through code_read, so a search chains into reading. Query grammar: plain words/snake_case fragments (fused), defs: callers: imports: word: literal: regex: pattern:'fn name($$$A)', scope with in:PATH. Raw strings, logs, config values: plain text search remains right. Same engine as the CLI."), search_properties.clone()),
            search_tool("keyword_search", &describe("Lexical-only search (FTS/trigram). Does not fuse AST or semantic channels."), search_properties.clone()),
            search_tool("ast_search", &describe("Native AST/pattern search (pattern: semantics). No external ast-grep process."), search_properties.clone()),
            search_tool("semantic_search", &describe("Embedding-only search. Requires a non-empty index with semantic chunks."), search_properties.clone()),
            // Kept for clients still calling the pre-split name; dispatches as Keyword (see dispatch_tool).
            search_tool("code_search", &describe("Deprecated compatibility alias for keyword_search; no automatic fusion across channels."), search_properties),
            {"name": "code_read", "description": "Read full code for result node IDs with optional adjacent-line context. Accepts compact search ids (`<path_id>:<start>-<end>`), explicit `path#Lstart-Lend` refs, single-line `path#Lstart`, and LSP-style `path:start`. Paths are sandboxed under ASGREP_ROOT.",
             "inputSchema": {"type": "object", "properties": {
                "ids": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": MAX_READ_REFS},
                "root": {"type": "string", "description": "Project root under the configured workspace"},
                "context_lines": {"type": "integer", "minimum": 0, "maximum": MAX_CONTEXT_LINES},
                "max_chars": {"type": "integer", "minimum": 1, "maximum": MAX_READ_CHARS}
             }, "required": ["ids"], "additionalProperties": false},
             "annotations": {"title": "Code read", "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}},
            {"name": "index_status", "description": "Show ast-sgrep index statistics for a project root under the configured workspace.",
             "annotations": {"title": "Index status", "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false},
             "inputSchema": {"type": "object", "properties": {"root": {"type": "string"}}, "additionalProperties": false}},
            {"name": "index_repo", "description": "Build or incrementally update the index. Single-flight with a wall-clock deadline; concurrent calls serialize.",
             "annotations": {"title": "Index repo", "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false},
             "inputSchema": {"type": "object", "properties": {
                "root": {"type": "string"}, "force": {"type": "boolean"}}, "additionalProperties": false}}
        ]})
    }

    /// Tool name → handler. `code_search` remains a keyword alias (compat; protocol tests pin it).
    /// Argument shapes are parsed once here into trusted structs; handlers never re-validate wire keys.
    pub(crate) fn dispatch_tool(
        &self,
        name: &str,
        args: &Value,
        cancel: Arc<AtomicBool>,
    ) -> anyhow::Result<String> {
        if cancel.load(Ordering::Acquire) {
            anyhow::bail!(ast_sgrep_core::INDEX_CANCELLED);
        }
        match name {
            "search" => {
                let parsed = self.parse_agent_search(args)?;
                self.tool_agent_search(parsed, AgentSearchMode::Hybrid)
            }
            "keyword_search" | "code_search" => {
                let parsed = self.parse_agent_search(args)?;
                self.tool_agent_search(parsed, AgentSearchMode::Keyword)
            }
            "ast_search" => {
                let parsed = self.parse_agent_search(args)?;
                self.tool_agent_search(parsed, AgentSearchMode::Ast)
            }
            "semantic_search" => {
                let parsed = self.parse_agent_search(args)?;
                self.tool_agent_search(parsed, AgentSearchMode::Semantic)
            }
            "code_read" => {
                let parsed = self.parse_code_read(args)?;
                self.tool_code_read(parsed)
            }
            "index_status" => {
                let parsed = self.parse_index_status(args)?;
                self.tool_index_status(parsed)
            }
            "index_repo" => {
                let parsed = self.parse_index_repo(args)?;
                self.tool_index_repo(parsed, cancel)
            }
            other => Err(anyhow::anyhow!("unknown tool: {other}")),
        }
    }

    fn search_options(
        &self,
        root: PathBuf,
        limit: usize,
        file_filter: Option<String>,
        lang_filter: Option<String>,
    ) -> SearchOptions {
        SearchOptions {
            root,
            index_path: self.index_path.clone(),
            limit,
            use_embed: self.use_embed,
            use_semantic_only: self.use_semantic_only,
            file_filter,
            lang_filter,
            ..SearchOptions::default()
        }
    }

    fn lock_or_recover<T>(
        mutex: &Mutex<T>,
        clear: impl FnOnce(&mut T),
    ) -> std::sync::MutexGuard<'_, T> {
        match mutex.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                mutex.clear_poison();
                let mut guard = PoisonError::into_inner(poisoned);
                clear(&mut guard);
                guard
            }
        }
    }

    fn base_index_options(&self, root: PathBuf) -> IndexOptions {
        IndexOptions {
            root,
            index_path: self.index_path.clone(),
            ..IndexOptions::default()
        }
    }

    /// Clear the session maps when an external writer bumped the stamp.
    fn sync_writer_generation(&self) {
        let current =
            ast_sgrep_core::read_writer_generation(&self.root, self.index_path.as_deref());
        let stale = {
            let guard = Self::lock_or_recover(&self.writer_state, |_| {});
            guard.writer_generation != current
        };
        if stale {
            // H1/F20: single invalidation path — reuse the post-index clear
            // so a cached map added there cannot be missed here. It parks the
            // stamp at 0, so record the stamp just observed afterwards.
            self.invalidate_after_index_attempt();
            *Self::lock_or_recover(&self.writer_state, |_| {}) =
                WriterState { writer_generation: current };
        }
    }

    fn drop_cached_searcher(&self) {
        if let Some(old) = Self::lock_or_recover(&self.searcher_cache, |_| {}).take() {
            old.searcher.release_read_snapshot();
        }
    }

    fn dispatch_search(
        searcher: &Searcher,
        mode: AgentSearchMode,
        query: &str,
    ) -> anyhow::Result<ast_sgrep_core::SearchResponse> {
        Ok(match mode {
            AgentSearchMode::Hybrid => searcher.search(query)?,
            AgentSearchMode::Keyword => searcher.search_lexical(query)?,
            AgentSearchMode::Ast => searcher.search(&format!("pattern: {query}"))?,
            AgentSearchMode::Semantic => searcher.search_semantic(query)?,
        })
    }

    /// Typed empty-index gate (P2): the search path fails closed with
    /// [`StoreError::EmptyIndex`]; match the variant through the whole
    /// anyhow chain so a future `.context()` cannot reintroduce string
    /// matching. Message text is never read here.
    fn is_empty_index_error(error: &anyhow::Error) -> bool {
        error.chain().any(|cause| {
            matches!(cause.downcast_ref::<StoreError>(), Some(store) if store.is_empty_index())
        })
    }

    fn clamp_search_response(
        mut response: ast_sgrep_core::SearchResponse,
        limit: usize,
    ) -> ast_sgrep_core::SearchResponse {
        response.limit = limit;
        if response.hits.len() > limit {
            response.truncated = true;
            response.hits.truncate(limit);
        }
        response
    }

    fn searcher_for(
        &self,
        root: PathBuf,
        limit: usize,
        file_filter: Option<String>,
        lang_filter: Option<String>,
    ) -> anyhow::Result<Searcher> {
        self.sync_writer_generation();
        Ok(Searcher::new(self.search_options(
            root,
            limit,
            file_filter,
            lang_filter,
        ))?)
    }

    fn cached_searcher(
        &self,
        root: PathBuf,
        needed_limit: usize,
        lang_filter: Option<&str>,
    ) -> anyhow::Result<std::sync::MutexGuard<'_, Option<CachedSearcher>>> {
        self.sync_writer_generation();
        let needed = needed_limit.clamp(1, MAX_AGENT_LIMIT);
        let writer_generation =
            ast_sgrep_core::read_writer_generation(&root, self.index_path.as_deref());
        let mut guard = Self::lock_or_recover(&self.searcher_cache, |slot| *slot = None);
        let reuse = !ast_sgrep_core::env_flag::memo_off("warm")
            && matches!(
                guard.as_ref(),
                Some(CachedSearcher { root: cached_root, lang_filter: cached_lang, open_limit, writer_generation: cached_gen, .. })
                    if cached_root == &root
                        && cached_lang.as_deref() == lang_filter
                        && *open_limit >= needed
                        && *cached_gen == writer_generation
            );
        if !reuse {
            if let Some(old) = guard.take() {
                old.searcher.release_read_snapshot();
            }
            let open_limit = needed.max(self.limit).clamp(1, MAX_AGENT_LIMIT);
            let searcher = Searcher::new(self.search_options(
                root.clone(),
                open_limit,
                None,
                lang_filter.map(str::to_owned),
            ))?;
            // P3: snapshot/warm failures propagate instead of being swallowed.
            // A half-warmed Searcher never enters the cache; the caller maps
            // an empty index to the miss envelope and surfaces the rest as a
            // tool error, exactly as if the search itself had failed.
            searcher.hold_read_snapshot()?;
            searcher.warm_search_path()?;
            *guard = Some(CachedSearcher {
                root,
                lang_filter: lang_filter.map(str::to_owned),
                open_limit,
                writer_generation,
                searcher,
            });
        }
        Ok(guard)
    }

    fn finish_search(
        response: ast_sgrep_core::SearchResponse,
        limit: usize,
        store: &IndexStore,
    ) -> (ast_sgrep_core::SearchResponse, Option<usize>) {
        let indexed = if response.hits.is_empty() {
            store.has_indexed_files().ok().map(usize::from)
        } else {
            None
        };
        (Self::clamp_search_response(response, limit), indexed)
    }

    fn run_search(
        &self,
        root: PathBuf,
        limit: usize,
        file_filter: Option<String>,
        lang: Option<String>,
        mode: AgentSearchMode,
        query: &str,
    ) -> anyhow::Result<(ast_sgrep_core::SearchResponse, Option<usize>)> {
        if file_filter.is_some() || ast_sgrep_core::env_flag::memo_off("warm") {
            let searcher = self.searcher_for(root, limit, file_filter, lang)?;
            let response = Self::dispatch_search(&searcher, mode, query)?;
            return Ok(Self::finish_search(response, limit, searcher.store()));
        }
        let guard = self.cached_searcher(root, limit, lang.as_deref())?;
        let searcher = &guard.as_ref().expect("cached_searcher populates").searcher;
        let response = Self::dispatch_search(searcher, mode, query)?;
        Ok(Self::finish_search(response, limit, searcher.store()))
    }

    fn tool_agent_search(
        &self,
        args: AgentSearchArgs,
        mode: AgentSearchMode,
    ) -> anyhow::Result<String> {
        let AgentSearchArgs {
            query,
            root,
            limit,
            resend_seen,
            budget_tokens,
            file_filter,
            lang,
            preview,
        } = args;
        let (response, indexed_files) =
            match self.run_search(root.clone(), limit, file_filter, lang, mode, &query) {
                Ok(pair) => pair,
                Err(error) if Self::is_empty_index_error(&error) => {
                    let miss = to_compact_miss_json(
                        &query,
                        &self.diagnose_miss(&root, &query, mode, Some(0)),
                    );
                    return Ok(serde_json::to_string(&miss)?);
                }
                Err(error) => return Err(error),
            };
        // 6a3i: a miss is the cheapest response we can send, and the one where
        // a vague answer costs the most in speculative agent retries.
        if response.hits.is_empty() {
            let miss = to_compact_miss_json(
                &query,
                &self.diagnose_miss(&root, &query, mode, indexed_files),
            );
            return Ok(serde_json::to_string(&miss)?);
        }
        // kxmc: compact key-free envelope, minified.
        let mut envelope = render_compact_envelope(&response, preview, budget_tokens);
        if !resend_seen && preview != PreviewMode::None {
            self.elide_seen_snippets(&root, &mut envelope);
        }
        Ok(serde_json::to_string(&envelope)?)
    }

    /// Replace snippets this session already sent for the same id and unchanged
    /// content (v972).
    ///
    /// Iterative agent search re-runs overlapping queries constantly, and the
    /// model pays again for bytes it already has. Keying on a content hash (not
    /// just the id) means an edited file re-sends in full, and the map is
    /// cleared on `index_repo` so an elision never spans index generations.
    fn elide_seen_snippets(&self, root: &Path, envelope: &mut Value) {
        let Some(hits) = envelope.get_mut("h").and_then(Value::as_array_mut) else {
            return;
        };
        let mut seen = Self::lock_or_recover(&self.emitted_snippets, |seen| seen.clear());
        let mut elided = 0_usize;
        for hit in hits {
            let Some(row) = hit.as_array_mut() else {
                continue;
            };
            let (Some(id), Some(snippet)) = (
                row.first().and_then(Value::as_str).map(str::to_owned),
                row.get(4).and_then(Value::as_str).map(str::to_owned),
            ) else {
                continue;
            };
            if snippet.is_empty() || snippet == ELIDED_SNIPPET {
                continue;
            }
            let digest = fnv1a64(snippet.as_bytes());
            let key = (root.to_path_buf(), id);
            if seen.get(&key) == Some(&digest) {
                row[4] = Value::String(ELIDED_SNIPPET.to_owned());
                elided += 1;
                continue;
            }
            if seen.len() >= MAX_EMITTED_SNIPPETS && !seen.contains_key(&key) {
                continue;
            }
            seen.insert(key, digest);
        }
        if elided > 0 {
            // Volatile accounting, so it sorts with the `z*` tail (9q0l).
            envelope["ze"] = Value::from(elided);
        }
    }

    /// Classify a zero-hit search. `indexed_files` is the probe already taken
    /// on the search path (`None` = not known; `Some(0)` = empty index).
    fn diagnose_miss(
        &self,
        root: &Path,
        query: &str,
        mode: AgentSearchMode,
        indexed_files: Option<usize>,
    ) -> MissContext {
        let indexed_files = indexed_files.or_else(|| {
            IndexStore::open_readonly(root, self.index_path.as_deref())
                .ok()
                .and_then(|store| store.has_indexed_files().ok())
                .map(usize::from)
        });
        let channel = match mode {
            AgentSearchMode::Hybrid => "hybrid",
            AgentSearchMode::Keyword => "lexical",
            AgentSearchMode::Ast => "structural",
            AgentSearchMode::Semantic => "semantic",
        };
        // Semantic search over an index with no semantic chunks is a channel
        // that could not run, not an honest absence of matches.
        let unavailable = if matches!(mode, AgentSearchMode::Semantic) && !self.use_embed {
            vec!["semantic".to_owned()]
        } else {
            Vec::new()
        };
        let mut scope = Vec::new();
        if root != self.root {
            if let Some(relative) = root.strip_prefix(&self.root).ok().map(Path::to_path_buf) {
                if !relative.as_os_str().is_empty() {
                    scope.push(("root".to_owned(), relative.display().to_string()));
                }
            }
        }
        MissContext {
            tried: vec![channel.to_owned()],
            unavailable,
            scope,
            indexed_files,
            pattern_miss: ast_sgrep_plugins::is_bodyless_fn_args_pattern(query),
        }
    }

    fn tool_code_read(&self, args: CodeReadArgs) -> anyhow::Result<String> {
        let CodeReadArgs {
            ids,
            root,
            context_lines,
            max_chars,
        } = args;
        let per_ref_chars = max_chars / ids.len();
        let remainder = max_chars % ids.len();
        let root = root.canonicalize().context("canonicalize project root")?;
        let mut nodes = Vec::with_capacity(ids.len());
        for (index, id) in ids.iter().enumerate() {
            let budget = per_ref_chars + usize::from(index < remainder);
            // kxmc: accept compact search ids as well as `path#Lstart-Lend`.
            let resolved = self.resolve_compact_id(&root, id);
            nodes.push(read_node(&root, &resolved, context_lines, budget)?);
        }
        Ok(serde_json::to_string(&json!({"nodes": nodes}))?)
    }

    fn tool_index_status(&self, args: IndexStatusArgs) -> anyhow::Result<String> {
        let indexer = Indexer::new(self.base_index_options(args.root))?;
        Ok(serde_json::to_string(&indexer.store().status()?)?)
    }

    fn tool_index_repo(
        &self,
        args: IndexRepoArgs,
        cancel: Arc<AtomicBool>,
    ) -> anyhow::Result<String> {
        let IndexRepoArgs { root, force } = args;
        // Single-flight wait counts toward the soft deadline (es7u).
        let started = Instant::now();
        let _flight = Self::lock_or_recover(&self.index_lock, |_| {});
        anyhow::ensure!(
            started.elapsed() < INDEX_REPO_DEADLINE,
            "index_repo exceeded {}s single-flight deadline before start",
            INDEX_REPO_DEADLINE.as_secs()
        );
        let mut indexer = Indexer::new(IndexOptions {
            embed_semantic: self.use_embed,
            ..self.base_index_options(root)
        })?;
        indexer.set_cancel(cancel);
        // index_all commits SQLite before sidecar rebuild; Err may still mean
        // durable mutation. Capture result then always sync session caches.
        let result = if force {
            indexer.reindex_all()
        } else {
            indexer.index_all()
        };
        // Always drop cached Searcher / path ids / elisions after a mutative
        // attempt (Ok or Err). Mid-sidecar Err must not leave a warm Searcher
        // serving pre-mutation hits.
        self.invalidate_after_index_attempt();
        let stats = result?;
        anyhow::ensure!(
            started.elapsed() <= INDEX_REPO_DEADLINE,
            "index_repo exceeded {}s deadline after mutation (index may have committed; caches were invalidated)",
            INDEX_REPO_DEADLINE.as_secs()
        );
        Ok(serde_json::to_string(&stats)?)
    }

    /// Invalidate session path/snippet maps after any index attempt.
    ///
    /// Call on both Ok and Err once `Indexer::new` succeeded and `index_all` /
    /// `reindex_all` returned: bulk commit can land before sidecar rebuild fails.
    fn invalidate_after_index_attempt(&self) {
        drop(Self::lock_or_recover(&self.writer_state, |state| {
            state.writer_generation = 0;
        }));
        Self::lock_or_recover(&self.emitted_snippets, |seen| seen.clear()).clear();
        self.drop_cached_searcher();
        *Self::lock_or_recover(&self.compact_paths, |intern| *intern = None) = None;
    }
}

#[cfg(test)]
mod wire_invalidation_regression {
    //! F19/F20 regression pins (census_mcp.md): one generic wire parse (S1)
    //! and one session-invalidation path (H1).
    use super::*;
    use serde_json::json;

    fn test_server() -> McpServer {
        McpServer {
            root: PathBuf::from("/tmp"),
            index_path: None,
            limit: 10,
            use_embed: false,
            use_semantic_only: false,
            writer_state: Mutex::new(WriterState::default()),
            index_lock: Mutex::new(()),
            emitted_snippets: Mutex::new(HashMap::new()),
            searcher_cache: Mutex::new(None),
            compact_paths: Mutex::new(None),
        }
    }

    /// F19: every tool-args shape parses through one generic boundary with
    /// one object check and one unknown-field mapping.
    #[test]
    fn parse_wire_single_generic_boundary() {
        let server = test_server();
        for raw in [json!("nope"), json!(42), json!([{"root": "/tmp"}])] {
            let err = server.parse_wire::<IndexStatusWire>(&raw).unwrap_err();
            assert!(
                err.to_string().contains("arguments must be an object"),
                "non-object rejected: {raw}"
            );
        }
        let err = server
            .parse_wire::<IndexRepoWire>(&json!({"force": true, "bogus": 1}))
            .unwrap_err();
        assert_eq!(err.to_string(), "unknown argument: bogus");
        let status: IndexStatusWire = server.parse_wire(&json!({})).unwrap();
        assert!(status.root.is_none());
        let repo: IndexRepoWire = server.parse_wire(&json!({"force": true})).unwrap();
        assert_eq!(repo.force, Some(true));
        // Per-tool parsers share the same boundary messages.
        let err = server.parse_agent_search(&json!([1])).unwrap_err();
        assert!(err.to_string().contains("arguments must be an object"));
        let err = server
            .parse_code_read(&json!({"ids": ["x"], "bogus": 1}))
            .unwrap_err();
        assert_eq!(err.to_string(), "unknown argument: bogus");
    }

    /// F20/H1: a stale writer stamp clears every session map through the
    /// single post-index invalidation path and records the stamp observed.
    #[test]
    fn stale_writer_sync_uses_single_invalidation_path() {
        let server = test_server();
        let current =
            ast_sgrep_core::read_writer_generation(&server.root, server.index_path.as_deref());
        *server.writer_state.lock().unwrap() =
            WriterState { writer_generation: current.wrapping_add(1) };
        server.emitted_snippets.lock().unwrap().insert(
            (PathBuf::from("/tmp"), String::from("stale-id")),
            7,
        );
        *server.compact_paths.lock().unwrap() = Some((
            0,
            PathBuf::from("/tmp"),
            HashMap::from([(String::from("a"), String::from("b"))]),
        ));
        server.sync_writer_generation();
        assert!(
            server.emitted_snippets.lock().unwrap().is_empty(),
            "stale sync clears elisions"
        );
        assert!(
            server.compact_paths.lock().unwrap().is_none(),
            "stale sync clears compact intern"
        );
        assert_eq!(
            server.writer_state.lock().unwrap().writer_generation,
            current,
            "stamp re-observed so the next sync is not spuriously stale"
        );
        // H1 structural pin: a stale sync clears every session map through the
        // single post-index invalidation path; the stamp is re-observed above.
    }
}

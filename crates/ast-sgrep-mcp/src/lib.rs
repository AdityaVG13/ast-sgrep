//! MCP stdio server for snippet-first hierarchical ast-sgrep retrieval.
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

use anyhow::Context;
use ast_sgrep_core::{EmbedBackend, IndexOptions, Indexer, SearchOptions, Searcher};
use ast_sgrep_plugins::{
    format_response_with_budget, to_compact_miss_json, CompactBudget, MissContext, OutputFormat,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "ast-sgrep";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_AGENT_LIMIT: usize = 100;
const MAX_READ_REFS: usize = 20;
const MAX_CONTEXT_LINES: usize = 100;
const DEFAULT_READ_CHARS: usize = 100_000;
const MAX_READ_CHARS: usize = 1_000_000;
const MAX_SCAN_BYTES: u64 = 64 * 1024 * 1024;
const INDEX_REPO_DEADLINE: Duration = Duration::from_secs(600);
/// Bound on remembered compact path-id mappings (kxmc). Compact search ids are
/// `<path_id>:<start>-<end>`; `code_read` resolves `<path_id>` through this map,
/// so an agent never has to reconstruct a path it was handed by id.
const MAX_PATH_REGISTRY: usize = 4_096;
/// Bound on remembered emitted snippets (v972).
const MAX_EMITTED_SNIPPETS: usize = 4_096;
/// Marker replacing a snippet this session already sent for the same id and
/// unchanged content (v972). The agent already has the body in its transcript;
/// `code_read` returns it again if not.
const ELIDED_SNIPPET: &str = "~";

#[derive(Clone, Copy)]
enum AgentSearchMode {
    Keyword,
    Ast,
    Semantic,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Clone, PartialEq, Eq)]
struct SearcherKey {
    root: PathBuf,
    index_path: Option<PathBuf>,
    limit: usize,
    use_embed: bool,
}

#[derive(Default)]
struct SearcherCache {
    generation: u64,
    entry: Option<(SearcherKey, Searcher)>,
}

pub struct McpServer {
    /// Configured workspace root; all tool roots must stay under this path.
    root: PathBuf,
    index_path: Option<PathBuf>,
    limit: usize,
    use_embed: bool,
    /// Reused across search-channel calls; cleared after index mutations.
    searcher_cache: Mutex<SearcherCache>,
    /// Single-flight lock for index_repo (es7u).
    index_lock: Mutex<()>,
    /// Compact path-id to project-relative path, learned from emitted search
    /// envelopes (kxmc). Bounded; cleared when the index changes.
    path_registry: Mutex<HashMap<String, String>>,
    /// Hit id to hash of the snippet already sent this session (v972).
    /// Bounded; cleared when the index changes, so an elision can never point
    /// at content from a previous index generation.
    emitted_snippets: Mutex<HashMap<String, u64>>,
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
            searcher_cache: Mutex::new(SearcherCache::default()),
            index_lock: Mutex::new(()),
            path_registry: Mutex::new(HashMap::new()),
            emitted_snippets: Mutex::new(HashMap::new()),
        })
    }

    pub fn run_stdio(&self) -> anyhow::Result<()> {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        for line in stdin.lock().lines() {
            let line = line.context("read stdin")?;
            if line.len() > ast_sgrep_core::MAX_STDIN_LINE_BYTES {
                // Bound JSON-RPC line memory; oversized lines are parse-error, not silent drop.
                // JSON-RPC 2.0: parse/invalid-id errors MUST use id: null (not omit id).
                write_resp(
                    &mut stdout,
                    Some(Value::Null),
                    None,
                    Some(json!({
                        "code": -32600,
                        "message": format!(
                            "request line exceeds max {} bytes",
                            ast_sgrep_core::MAX_STDIN_LINE_BYTES
                        )
                    })),
                )?;
                continue;
            }
            if line.trim().is_empty() {
                continue;
            }
            let request: JsonRpcRequest = match serde_json::from_str(&line) {
                Ok(req) => req,
                Err(e) => {
                    write_resp(
                        &mut stdout,
                        Some(Value::Null),
                        None,
                        Some(json!({"code": -32700, "message": format!("parse error: {e}")})),
                    )?;
                    continue;
                }
            };
            if let Some(response) = self.handle_request(&request) {
                match response {
                    Ok(result) => write_resp(&mut stdout, request.id, Some(result), None)?,
                    Err(error) => write_resp(&mut stdout, request.id, None, Some(error))?,
                }
            }
        }
        Ok(())
    }

    fn handle_request(&self, request: &JsonRpcRequest) -> Option<Result<Value, Value>> {
        request.id.as_ref()?;
        Some(match request.method.as_str() {
            "initialize" => Ok(self.handle_initialize()),
            "tools/list" => Ok(self.handle_tools_list()),
            "tools/call" => return self.handle_tools_call(&request.params).map(Ok),
            "ping" => Ok(json!({})),
            _ => Err(
                json!({"code": -32601, "message": format!("method not found: {}", request.method)}),
            ),
        })
    }

    fn handle_initialize(&self) -> Value {
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
        })
    }

    fn handle_tools_list(&self) -> Value {
        let search_properties = json!({
            "query": {"type": "string", "minLength": 1, "maxLength": ast_sgrep_core::MAX_QUERY_CHARS},
            "root": {"type": "string", "description": "Project root (defaults to ASGREP_ROOT or cwd)"},
            "limit": {"type": "integer", "minimum": 1, "maximum": MAX_AGENT_LIMIT},
            "resend_seen": {"type": "boolean", "description": "Send snippets already returned this session instead of the ~ marker. Set true only if you do not keep earlier results."}
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
            search_tool("keyword_search", &describe("Lexical-only search (FTS/trigram). Does not fuse AST or semantic channels."), search_properties.clone()),
            search_tool("ast_search", &describe("Native AST/pattern search (pattern: semantics). No external ast-grep process."), search_properties.clone()),
            search_tool("semantic_search", &describe("Embedding-only search. Requires a non-empty index with semantic chunks."), search_properties.clone()),
            // Kept for clients still calling the pre-split name; dispatches as Keyword (see dispatch_tool).
            search_tool("code_search", &describe("Deprecated compatibility alias for keyword_search; no automatic fusion across channels."), search_properties),
            {"name": "code_read", "description": "Read full code for result node IDs with optional adjacent-line context. Accepts compact search ids (`<path_id>:<start>-<end>`) and explicit `path#Lstart-Lend` refs. Paths are sandboxed under ASGREP_ROOT.",
             "inputSchema": {"type": "object", "properties": {
                "ids": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": MAX_READ_REFS},
                "root": {"type": "string", "description": "Project root under the configured workspace"},
                "context_lines": {"type": "integer", "minimum": 0, "maximum": MAX_CONTEXT_LINES},
                "max_chars": {"type": "integer", "minimum": 1, "maximum": MAX_READ_CHARS}
             }, "required": ["ids"], "additionalProperties": false}},
            {"name": "index_status", "description": "Show ast-sgrep index statistics for a project root under the configured workspace.",
             "inputSchema": {"type": "object", "properties": {"root": {"type": "string"}}, "additionalProperties": false}},
            {"name": "index_repo", "description": "Build or incrementally update the index. Single-flight with a wall-clock deadline; concurrent calls serialize.",
             "inputSchema": {"type": "object", "properties": {
                "root": {"type": "string"}, "force": {"type": "boolean"}}, "additionalProperties": false}}
        ]})
    }

    fn handle_tools_call(&self, params: &Value) -> Option<Value> {
        let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        let result = self.dispatch_tool(name, &args);
        Some(match result {
            Ok(text) => json!({"content": [{"type": "text", "text": text}], "isError": false}),
            Err(e) => {
                json!({"content": [{"type": "text", "text": e.to_string()}], "isError": true})
            }
        })
    }

    /// Tool name → handler. `code_search` remains a keyword alias (compat; protocol tests pin it).
    fn dispatch_tool(&self, name: &str, args: &Value) -> anyhow::Result<String> {
        match name {
            // keyword_search and deprecated code_search share Keyword mode (compat alias).
            "keyword_search" | "code_search" => {
                self.tool_agent_search(args, AgentSearchMode::Keyword)
            }
            "ast_search" => self.tool_agent_search(args, AgentSearchMode::Ast),
            "semantic_search" => self.tool_agent_search(args, AgentSearchMode::Semantic),
            "code_read" => self.tool_code_read(args),
            "index_status" => self.tool_index_status(args),
            "index_repo" => self.tool_index_repo(args),
            other => Err(anyhow::anyhow!("unknown tool: {other}")),
        }
    }

    fn validate_fields(args: &Value, allowed: &[&str]) -> anyhow::Result<()> {
        let object = args.as_object().context("arguments must be an object")?;
        if let Some(field) = object
            .keys()
            .find(|field| !allowed.contains(&field.as_str()))
        {
            anyhow::bail!("unknown argument: {field}");
        }
        Ok(())
    }

    fn integer_arg(
        args: &Value,
        name: &str,
        default: usize,
        minimum: usize,
        maximum: usize,
    ) -> anyhow::Result<usize> {
        match args.get(name) {
            None => Ok(default),
            Some(value) => {
                let value = value
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .with_context(|| format!("{name} must be an integer"))?;
                anyhow::ensure!(
                    (minimum..=maximum).contains(&value),
                    "{name} must be between {minimum} and {maximum}"
                );
                Ok(value)
            }
        }
    }

    fn root_arg(&self, args: &Value) -> anyhow::Result<PathBuf> {
        let candidate = match args.get("root") {
            None => self.root.clone(),
            Some(value) => PathBuf::from(value.as_str().context("root must be a string")?),
        };
        self.sandbox_root(candidate)
    }

    /// Keep MCP tool roots under the configured workspace (v0mg).
    fn sandbox_root(&self, candidate: PathBuf) -> anyhow::Result<PathBuf> {
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

    fn searcher_key(&self, root: PathBuf, limit: usize) -> SearcherKey {
        SearcherKey {
            root,
            index_path: self.index_path.clone(),
            limit,
            use_embed: self.use_embed,
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

    fn invalidate_searcher_cache(&self) {
        // Advance the generation even when a search temporarily owns the cached
        // Searcher. That prevents the stale Searcher from returning after reindex.
        let mut guard = Self::lock_or_recover(&self.searcher_cache, |cache| {
            cache.entry = None;
        });
        guard.generation = guard.generation.wrapping_add(1);
        guard.entry = None;
    }

    fn searcher_for(&self, root: PathBuf, limit: usize) -> anyhow::Result<(Searcher, u64)> {
        let key = self.searcher_key(root.clone(), limit);
        // Poison fails closed: invalidate and rebuild rather than reuse tainted state.
        let mut guard = Self::lock_or_recover(&self.searcher_cache, |cache| {
            cache.generation = cache.generation.wrapping_add(1);
            cache.entry = None;
        });
        let need_new = match guard.entry.as_ref() {
            None => true,
            Some((cached_key, _)) => cached_key != &key,
        };
        if need_new {
            let searcher = Searcher::new(SearchOptions {
                root,
                index_path: self.index_path.clone(),
                limit,
                use_embed: self.use_embed,
                ..SearchOptions::default()
            })?;
            guard.entry = Some((key, searcher));
        }
        let generation = guard.generation;
        let (_, searcher) = guard
            .entry
            .take()
            .ok_or_else(|| anyhow::anyhow!("searcher cache missing after populate"))?;
        drop(guard);
        Ok((searcher, generation))
    }

    fn restore_searcher(
        &self,
        root: PathBuf,
        limit: usize,
        generation: u64,
        searcher: Searcher,
    ) {
        let key = self.searcher_key(root, limit);
        let mut guard = Self::lock_or_recover(&self.searcher_cache, |cache| {
            cache.generation = cache.generation.wrapping_add(1);
            cache.entry = None;
        });
        if guard.generation == generation && guard.entry.is_none() {
            guard.entry = Some((key, searcher));
        }
    }

    fn tool_agent_search(&self, args: &Value, mode: AgentSearchMode) -> anyhow::Result<String> {
        Self::validate_fields(args, &["query", "root", "limit", "resend_seen"])?;
        // v972: transcript-less clients set resend_seen to keep full snippets.
        let resend_seen = match args.get("resend_seen") {
            None => false,
            Some(value) => value
                .as_bool()
                .context("resend_seen must be a boolean")?,
        };
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|query| {
                !query.is_empty() && query.chars().count() <= ast_sgrep_core::MAX_QUERY_CHARS
            })
            .with_context(|| {
                format!(
                    "query must contain 1 to {} characters",
                    ast_sgrep_core::MAX_QUERY_CHARS
                )
            })?;
        let limit = Self::integer_arg(args, "limit", self.limit, 1, MAX_AGENT_LIMIT)?;
        let root = self.root_arg(args)?;
        let (searcher, generation) = self.searcher_for(root.clone(), limit)?;
        let response = match mode {
            AgentSearchMode::Keyword => searcher.search_lexical(query),
            AgentSearchMode::Ast => searcher.search(&format!("pattern: {query}")),
            AgentSearchMode::Semantic => searcher.search_semantic(query),
        };
        self.restore_searcher(root.clone(), limit, generation, searcher);
        let response = response?;
        // 6a3i: a miss is the cheapest response we can send, and the one where
        // a vague answer costs the most in speculative agent retries.
        if response.hits.is_empty() {
            let miss = to_compact_miss_json(query, &self.diagnose_miss(&root, mode));
            return Ok(serde_json::to_string(&miss)?);
        }
        // kxmc: compact key-free envelope, minified. Object keys and pretty
        // whitespace were the bulk of the old AgentCapsule payload, and the
        // full path was emitted twice per hit (`file` plus `ref`).
        let mut envelope = format_response_with_budget(
            &response,
            OutputFormat::Compact,
            0,
            CompactBudget::default(),
        );
        self.remember_compact_paths(&envelope);
        if !resend_seen {
            self.elide_seen_snippets(&mut envelope);
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
    fn elide_seen_snippets(&self, envelope: &mut Value) {
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
            if seen.get(&id) == Some(&digest) {
                row[4] = Value::String(ELIDED_SNIPPET.to_owned());
                elided += 1;
                continue;
            }
            if seen.len() >= MAX_EMITTED_SNIPPETS && !seen.contains_key(&id) {
                continue;
            }
            seen.insert(id, digest);
        }
        if elided > 0 {
            // Volatile accounting, so it sorts with the `z*` tail (9q0l).
            envelope["ze"] = Value::from(elided);
        }
    }

    /// Gather what is known about a zero-hit search (6a3i).
    ///
    /// The index count is only consulted here, on the miss path, so a normal
    /// search never pays for it.
    fn diagnose_miss(&self, root: &Path, mode: AgentSearchMode) -> MissContext {
        let indexed_files = Indexer::new(self.base_index_options(root.to_path_buf()))
            .ok()
            .and_then(|indexer| indexer.store().status().ok())
            .map(|status| status.file_count);
        let channel = match mode {
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
        }
    }

    /// Record `p` table entries so `code_read` can resolve compact ids later.
    ///
    /// Uses the shared resolver so root-folded tables (am4a) are understood
    /// exactly the way an agent would read them.
    fn remember_compact_paths(&self, envelope: &Value) {
        let mut registry = Self::lock_or_recover(&self.path_registry, |registry| registry.clear());
        for (id, path) in ast_sgrep_plugins::resolve_compact_paths(envelope) {
            if registry.len() >= MAX_PATH_REGISTRY && !registry.contains_key(&id) {
                // Bounded: a hostile or very long session cannot grow this map
                // without limit. Unknown ids simply fail to resolve and the
                // agent falls back to the `path#Lstart-Lend` form.
                break;
            }
            registry.insert(id, path);
        }
    }

    /// Expand a compact `<path_id>:<start>-<end>` id into the node id form that
    /// `read_node` understands. Non-compact ids pass through untouched.
    fn resolve_compact_id(&self, id: &str) -> String {
        if id.contains("#L") {
            return id.to_owned();
        }
        let Some((path_id, range)) = id.rsplit_once(':') else {
            return id.to_owned();
        };
        let Some((start, end)) = range.split_once('-') else {
            return id.to_owned();
        };
        if start.is_empty() || !start.bytes().all(|b| b.is_ascii_digit()) {
            return id.to_owned();
        }
        if end.is_empty() || !end.bytes().all(|b| b.is_ascii_digit()) {
            return id.to_owned();
        }
        let registry = Self::lock_or_recover(&self.path_registry, |registry| registry.clear());
        match registry.get(path_id) {
            Some(path) => format!("{path}#L{start}-L{end}"),
            None => id.to_owned(),
        }
    }

    fn tool_code_read(&self, args: &Value) -> anyhow::Result<String> {
        Self::validate_fields(args, &["ids", "root", "context_lines", "max_chars"])?;
        let ids = args
            .get("ids")
            .and_then(Value::as_array)
            .filter(|ids| !ids.is_empty() && ids.len() <= MAX_READ_REFS)
            .context("ids must contain 1 to 20 node IDs")?;
        let context_lines = Self::integer_arg(args, "context_lines", 0, 0, MAX_CONTEXT_LINES)?;
        let max_chars = match args.get("max_chars") {
            None => DEFAULT_READ_CHARS,
            Some(value) => value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .context("max_chars must be a positive integer")?,
        };
        anyhow::ensure!(
            (1..=MAX_READ_CHARS).contains(&max_chars),
            "max_chars must be between 1 and {MAX_READ_CHARS}"
        );
        let per_ref_chars = max_chars / ids.len();
        let remainder = max_chars % ids.len();
        let root = self
            .root_arg(args)?
            .canonicalize()
            .context("canonicalize project root")?;
        let mut nodes = Vec::with_capacity(ids.len());
        for (index, id) in ids.iter().enumerate() {
            let id = id.as_str().context("every node ID must be a string")?;
            let budget = per_ref_chars + usize::from(index < remainder);
            // kxmc: accept compact search ids as well as `path#Lstart-Lend`.
            let resolved = self.resolve_compact_id(id);
            nodes.push(read_node(&root, &resolved, context_lines, budget)?);
        }
        Ok(serde_json::to_string(&json!({"nodes": nodes}))?)
    }

    fn tool_index_status(&self, args: &Value) -> anyhow::Result<String> {
        Self::validate_fields(args, &["root"])?;
        let indexer = Indexer::new(self.base_index_options(self.root_arg(args)?))?;
        Ok(serde_json::to_string(&indexer.store().status()?)?)
    }

    fn tool_index_repo(&self, args: &Value) -> anyhow::Result<String> {
        Self::validate_fields(args, &["root", "force"])?;
        let force = match args.get("force") {
            None => false,
            Some(value) => value.as_bool().context("force must be a boolean")?,
        };
        let root = self.root_arg(args)?;
        // Single-flight wait counts toward the soft deadline (es7u).
        let started = Instant::now();
        let _flight = Self::lock_or_recover(&self.index_lock, |_| {});
        // Soft deadline: refuse to start when the prior wait already exhausted the budget.
        anyhow::ensure!(
            started.elapsed() < INDEX_REPO_DEADLINE,
            "index_repo exceeded {}s single-flight deadline before start",
            INDEX_REPO_DEADLINE.as_secs()
        );
        let mut indexer = Indexer::new(IndexOptions {
            embed_semantic: self.use_embed,
            embed_backend: if self.use_embed {
                EmbedBackend::Auto
            } else {
                EmbedBackend::Semantic
            },
            ..self.base_index_options(root)
        })?;
        let stats = if force {
            indexer.reindex_all()?
        } else {
            indexer.index_all()?
        };
        // Index mutated on disk — always drop cached Searcher / path ids / elisions
        // before any post-work deadline check. A soft timeout must not leave a
        // stale Searcher serving pre-mutation hits (d2a1.13).
        self.invalidate_searcher_cache();
        Self::lock_or_recover(&self.path_registry, |registry| registry.clear()).clear();
        Self::lock_or_recover(&self.emitted_snippets, |seen| seen.clear()).clear();
        anyhow::ensure!(
            started.elapsed() <= INDEX_REPO_DEADLINE,
            "index_repo exceeded {}s deadline",
            INDEX_REPO_DEADLINE.as_secs()
        );
        Ok(serde_json::to_string(&stats)?)
    }
}

fn parse_node_id(id: &str) -> anyhow::Result<(&str, usize, usize)> {
    let (file, range) = id
        .rsplit_once("#L")
        .context("node ID must end in #Lstart-Lend")?;
    let (start_raw, end_raw) = range
        .split_once("-L")
        .context("node ID must end in #Lstart-Lend")?;
    let start = start_raw
        .parse::<u32>()
        .context("invalid node start line")?;
    let end = end_raw.parse::<u32>().context("invalid node end line")?;
    anyhow::ensure!(
        start > 0 && end >= start && start_raw == start.to_string() && end_raw == end.to_string(),
        "invalid or noncanonical node line range"
    );
    let start = start as usize;
    let end = end as usize;
    anyhow::ensure!(!file.is_empty(), "node ID file is empty");
    anyhow::ensure!(
        Path::new(file)
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir)),
        "node ID must be a relative project path"
    );
    Ok((file, start, end))
}

fn same_opened_file(expected: &std::fs::Metadata, actual: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        expected.dev() == actual.dev() && expected.ino() == actual.ino()
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        expected.volume_serial_number().is_some()
            && expected.volume_serial_number() == actual.volume_serial_number()
            && expected.file_index().is_some()
            && expected.file_index() == actual.file_index()
    }
    #[cfg(not(any(unix, windows)))]
    {
        expected.len() == actual.len() && expected.modified().ok() == actual.modified().ok()
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> (&str, bool) {
    match value.char_indices().nth(max_chars) {
        Some((byte, _)) => (&value[..byte], true),
        None => (value, false),
    }
}

fn read_node(
    root: &Path,
    id: &str,
    context_lines: usize,
    max_chars: usize,
) -> anyhow::Result<Value> {
    let (file, requested_start, requested_end) = parse_node_id(id)?;
    let unresolved = root.join(file);
    anyhow::ensure!(unresolved.starts_with(root), "node ID escapes project root");
    let canonical = unresolved
        .canonicalize()
        .context("canonicalize node file")?;
    anyhow::ensure!(canonical.starts_with(root), "node ID escapes project root");
    let expected = canonical.metadata().context("stat node file")?;
    anyhow::ensure!(
        expected.is_file(),
        "node ID does not reference a regular file"
    );
    let handle = File::open(&canonical).context("open node file")?;
    let actual = handle.metadata().context("stat opened node file")?;
    anyhow::ensure!(
        same_opened_file(&expected, &actual),
        "node file changed while opening"
    );
    let reopened = unresolved.canonicalize().context("recheck node file")?;
    anyhow::ensure!(
        reopened == canonical && reopened.starts_with(root),
        "node file changed while opening"
    );
    let start = requested_start.saturating_sub(context_lines).max(1);
    let wanted_end = requested_end.saturating_add(context_lines);
    let mut reader = std::io::BufReader::new(handle.take(MAX_SCAN_BYTES + 1));
    let mut line_number = 1usize;
    let mut total_lines = 0usize;
    let mut scanned_bytes = 0u64;
    let mut selected = Vec::new();
    loop {
        let mut bytes = Vec::new();
        let count = reader
            .read_until(b'\n', &mut bytes)
            .context("read node file")?;
        if count == 0 {
            if line_number == 1 {
                total_lines = 1;
                if start <= 1 && wanted_end >= 1 {
                    selected.push(String::new());
                }
            }
            break;
        }
        scanned_bytes = scanned_bytes.saturating_add(count as u64);
        anyhow::ensure!(
            scanned_bytes <= MAX_SCAN_BYTES,
            "node file exceeds scan limit"
        );
        if bytes.last() == Some(&b'\n') {
            bytes.pop();
        }
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        let line = String::from_utf8(bytes).context("node file is not valid UTF-8")?;
        total_lines = line_number;
        if line_number >= start && line_number <= wanted_end {
            selected.push(line);
        }
        if line_number >= wanted_end {
            break;
        }
        line_number += 1;
    }
    anyhow::ensure!(
        requested_start <= total_lines && requested_end <= total_lines,
        "node line range is beyond end of file"
    );
    let end = wanted_end.min(total_lines);
    let selected = selected.join("\n");
    let (content, truncated) = truncate_chars(&selected, max_chars);
    Ok(json!({
        "id": id,
        "file": file,
        "lines": {"start": start, "end": end},
        "content": content,
        "truncated": truncated
    }))
}

fn write_resp(
    stdout: &mut impl Write,
    id: Option<Value>,
    result: Option<Value>,
    error: Option<Value>,
) -> io::Result<()> {
    let mut body = json!({"jsonrpc": "2.0"});
    if let Some(id) = id {
        body["id"] = id;
    }
    if let Some(result) = result {
        body["result"] = result;
    }
    if let Some(error) = error {
        body["error"] = error;
    }
    writeln!(stdout, "{body}")
}


#[cfg(test)]
mod cache_tests {
    use super::*;

    fn test_server(root: PathBuf) -> McpServer {
        McpServer {
            root,
            index_path: None,
            limit: 10,
            use_embed: false,
            searcher_cache: Mutex::new(SearcherCache::default()),
            index_lock: Mutex::new(()),
            path_registry: Mutex::new(HashMap::new()),
            emitted_snippets: Mutex::new(HashMap::new()),
        }
    }

    #[test]
    fn reindex_generation_rejects_in_flight_stale_searcher() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let server = test_server(root.clone());
        let (searcher, generation) = server.searcher_for(root.clone(), 10).unwrap();
        server.invalidate_searcher_cache();
        server.restore_searcher(root, 10, generation, searcher);
        let cache = McpServer::lock_or_recover(&server.searcher_cache, |_| {});
        assert!(cache.entry.is_none(), "stale searcher returned after reindex");
    }

    #[test]
    fn index_repo_invalidates_searcher_after_disk_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), "fn hello() {}\n").unwrap();
        let server = test_server(root.clone());
        let (searcher, generation) = server.searcher_for(root.clone(), 10).unwrap();
        server.restore_searcher(root.clone(), 10, generation, searcher);
        {
            let cache = McpServer::lock_or_recover(&server.searcher_cache, |_| {});
            assert!(cache.entry.is_some());
            assert_eq!(cache.generation, generation);
        }
        // Seed session maps that must not survive reindex.
        McpServer::lock_or_recover(&server.path_registry, |_| {})
            .insert("p0".into(), "lib.rs".into());
        McpServer::lock_or_recover(&server.emitted_snippets, |_| {})
            .insert("p0:1-1".into(), 42);

        let body = server
            .tool_index_repo(&json!({}))
            .expect("index_repo should succeed on tiny fixture");
        assert!(body.contains("files_indexed") || body.contains("files"), "{body}");

        let cache = McpServer::lock_or_recover(&server.searcher_cache, |_| {});
        assert!(
            cache.entry.is_none(),
            "searcher cache must be empty after index_repo mutation"
        );
        assert!(
            cache.generation != generation,
            "generation must advance so in-flight restore cannot reinstall stale Searcher"
        );
        assert!(
            McpServer::lock_or_recover(&server.path_registry, |_| {}).is_empty(),
            "path registry must clear on index mutation"
        );
        assert!(
            McpServer::lock_or_recover(&server.emitted_snippets, |_| {}).is_empty(),
            "emitted snippets must clear on index mutation"
        );
    }
}
/// FNV-1a over snippet bytes (v972). Content-keyed so an edited file re-sends.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

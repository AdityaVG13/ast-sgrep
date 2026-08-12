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
use ast_sgrep_core::io_bounds::{read_bounded_line, BoundedLine};
use ast_sgrep_core::{
    force_sidecar_rebuild_err, EmbedBackend, IndexOptions, Indexer, SearchOptions, Searcher,
};
use ast_sgrep_plugins::{
    format_response_with_budget, to_budgeted_compact_json, to_compact_miss_json, CompactBudget,
    DetailLevel, MissContext, OutputBudget, OutputFormat,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Current handshake-based MCP revision this server implements (r2lu).
///
/// The 2026-07-28 revision removed `initialize`, requires per-request protocol
/// metadata, and adds `server/discover`. Advertising it from this legacy stdio
/// lifecycle would make modern clients select a protocol we do not implement.
const PROTOCOL_VERSION: &str = "2025-11-25";
/// Handshake-era revision kept for existing clients (r2lu).
const LEGACY_PROTOCOL_VERSION: &str = "2024-11-05";
/// Revisions this server will negotiate down to, newest first.
const SUPPORTED_PROTOCOL_VERSIONS: [&str; 2] = [PROTOCOL_VERSION, LEGACY_PROTOCOL_VERSION];
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
    Keyword,
    Ast,
    Semantic,
}

/// Wire JSON for keyword / ast / semantic search tools (`deny_unknown_fields`).
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
}

/// Trusted agent-search args after MCP tools/call boundary parse.
struct AgentSearchArgs {
    query: String,
    root: PathBuf,
    limit: usize,
    resend_seen: bool,
    budget_tokens: Option<usize>,
}

/// Wire JSON for `code_read`.
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
struct CodeReadArgs {
    ids: Vec<String>,
    root: PathBuf,
    context_lines: usize,
    max_chars: usize,
}

/// Wire JSON for `index_status`.
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

/// Wire JSON for `index_repo`.
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

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
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
    /// Last observed on-disk writer stamp; mismatch forces reopen.
    writer_generation: u64,
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
        let mut input = stdin.lock();
        let mut stdout = io::stdout();
        loop {
            let Some(line) = read_bounded_line(&mut input, ast_sgrep_core::MAX_STDIN_LINE_BYTES)
                .context("read stdin")?
            else {
                break;
            };
            let line = match line {
                BoundedLine::Line(line) => line,
                BoundedLine::TooLong => {
                    // Reject before allocating the complete attacker-controlled line.
                    // JSON-RPC 2.0 parse/invalid-id errors use id: null.
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
            };
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let request: JsonRpcRequest = match serde_json::from_slice(&line) {
                Ok(req) => req,
                Err(e) => {
                    let code = if serde_json::from_slice::<Value>(&line).is_ok() {
                        -32600
                    } else {
                        -32700
                    };
                    let label = if code == -32600 {
                        "invalid request"
                    } else {
                        "parse error"
                    };
                    write_resp(
                        &mut stdout,
                        Some(Value::Null),
                        None,
                        Some(json!({"code": code, "message": format!("{label}: {e}")})),
                    )?;
                    continue;
                }
            };
            if request.jsonrpc != "2.0" {
                write_resp(
                    &mut stdout,
                    Some(Value::Null),
                    None,
                    Some(
                        json!({"code": -32600, "message": "invalid request: jsonrpc must be 2.0"}),
                    ),
                )?;
                continue;
            }
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
            "initialize" => Ok(self.handle_initialize(&request.params)),
            "tools/list" => Ok(self.handle_tools_list()),
            "tools/call" => return self.handle_tools_call(&request.params).map(Ok),
            "ping" => Ok(json!({})),
            _ => Err(
                json!({"code": -32601, "message": format!("method not found: {}", request.method)}),
            ),
        })
    }

    /// Negotiate a protocol revision (r2lu).
    ///
    /// A client that asks for a revision we support gets that revision back, so
    /// handshake-era clients keep working unchanged. Anything else is answered
    /// with our current revision, which is what the spec asks a server to do
    /// when it cannot satisfy the request exactly.
    fn handle_initialize(&self, params: &Value) -> Value {
        let requested = params.get("protocolVersion").and_then(Value::as_str);
        let negotiated = requested
            .filter(|version| SUPPORTED_PROTOCOL_VERSIONS.contains(version))
            .unwrap_or(PROTOCOL_VERSION);
        json!({
            "protocolVersion": negotiated,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
        })
    }

    fn handle_tools_list(&self) -> Value {
        let search_properties = json!({
            "query": {"type": "string", "minLength": 1, "maxLength": ast_sgrep_core::MAX_QUERY_CHARS},
            "root": {"type": "string", "description": "Project root (defaults to ASGREP_ROOT or cwd)"},
            "limit": {"type": "integer", "minimum": 1, "maximum": MAX_AGENT_LIMIT},
            "resend_seen": {"type": "boolean", "description": "Send snippets already returned this session instead of the ~ marker. Set true only if you do not keep earlier results."},
            "budget_tokens": {"type": "integer", "minimum": 1, "maximum": MAX_BUDGET_TOKENS, "description": "Whole-response token budget. Each hit gains a trailing detail level (metadata|signature|block|full) and omitted source is marked with a gap marker."}
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
                "outputSchema": search_output_schema.clone()
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
            // r2lu: typed structuredContent for current clients, with the
            // minified text kept as the fallback older clients still read.
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(structured) => json!({
                    "content": [{"type": "text", "text": text}],
                    "structuredContent": structured,
                    "isError": false
                }),
                Err(_) => json!({"content": [{"type": "text", "text": text}], "isError": false}),
            },
            Err(e) => {
                json!({"content": [{"type": "text", "text": e.to_string()}], "isError": true})
            }
        })
    }

    /// Tool name → handler. `code_search` remains a keyword alias (compat; protocol tests pin it).
    /// Argument shapes are parsed once here into trusted structs; handlers never re-validate wire keys.
    fn dispatch_tool(&self, name: &str, args: &Value) -> anyhow::Result<String> {
        match name {
            // keyword_search and deprecated code_search share Keyword mode (compat alias).
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
                self.tool_index_repo(parsed)
            }
            other => Err(anyhow::anyhow!("unknown tool: {other}")),
        }
    }

    fn map_wire_error(err: serde_json::Error) -> anyhow::Error {
        let msg = err.to_string();
        // Preserve the prior unknown-key phrasing agents already see.
        if let Some(rest) = msg.strip_prefix("unknown field `") {
            if let Some(field) = rest.split('`').next() {
                return anyhow::anyhow!("unknown argument: {field}");
            }
        }
        anyhow::anyhow!(msg)
    }

    fn bounded_usize(
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

    fn resolve_root(&self, root: Option<String>) -> anyhow::Result<PathBuf> {
        let candidate = match root {
            None => self.root.clone(),
            Some(value) => PathBuf::from(value),
        };
        self.sandbox_root(candidate)
    }

    fn parse_agent_search(&self, args: &Value) -> anyhow::Result<AgentSearchArgs> {
        anyhow::ensure!(args.is_object(), "arguments must be an object");
        let wire: AgentSearchWire =
            serde_json::from_value(args.clone()).map_err(Self::map_wire_error)?;
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
        Ok(AgentSearchArgs {
            query: query.to_owned(),
            root: self.resolve_root(wire.root)?,
            limit,
            resend_seen: wire.resend_seen.unwrap_or(false),
            budget_tokens,
        })
    }

    fn parse_code_read(&self, args: &Value) -> anyhow::Result<CodeReadArgs> {
        anyhow::ensure!(args.is_object(), "arguments must be an object");
        let wire: CodeReadWire =
            serde_json::from_value(args.clone()).map_err(Self::map_wire_error)?;
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
            Some(value) => {
                let value = usize::try_from(value)
                    .ok()
                    .context("max_chars must be a positive integer")?;
                anyhow::ensure!(
                    (1..=MAX_READ_CHARS).contains(&value),
                    "max_chars must be between 1 and {MAX_READ_CHARS}"
                );
                value
            }
        };
        Ok(CodeReadArgs {
            ids: wire.ids,
            root: self.resolve_root(wire.root)?,
            context_lines,
            max_chars,
        })
    }

    fn parse_index_status(&self, args: &Value) -> anyhow::Result<IndexStatusArgs> {
        anyhow::ensure!(args.is_object(), "arguments must be an object");
        let wire: IndexStatusWire =
            serde_json::from_value(args.clone()).map_err(Self::map_wire_error)?;
        Ok(IndexStatusArgs {
            root: self.resolve_root(wire.root)?,
        })
    }

    fn parse_index_repo(&self, args: &Value) -> anyhow::Result<IndexRepoArgs> {
        anyhow::ensure!(args.is_object(), "arguments must be an object");
        let wire: IndexRepoWire =
            serde_json::from_value(args.clone()).map_err(Self::map_wire_error)?;
        Ok(IndexRepoArgs {
            root: self.resolve_root(wire.root)?,
            force: wire.force.unwrap_or(false),
        })
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
        // Refresh observed stamp so a local invalidate does not immediately
        // thrash against an unchanged on-disk epoch.
        guard.writer_generation = ast_sgrep_core::read_writer_generation(
            &self.root,
            self.index_path.as_deref(),
        );
    }

    /// Drop warm Searcher (+ session maps) when an external writer bumped the stamp.
    fn sync_writer_generation(&self) {
        let current =
            ast_sgrep_core::read_writer_generation(&self.root, self.index_path.as_deref());
        let mut guard = Self::lock_or_recover(&self.searcher_cache, |cache| {
            cache.entry = None;
        });
        if guard.writer_generation != current {
            guard.generation = guard.generation.wrapping_add(1);
            guard.entry = None;
            guard.writer_generation = current;
            drop(guard);
            Self::lock_or_recover(&self.path_registry, |registry| registry.clear()).clear();
            Self::lock_or_recover(&self.emitted_snippets, |seen| seen.clear()).clear();
        }
    }

    fn searcher_for(&self, root: PathBuf, limit: usize) -> anyhow::Result<(Searcher, u64)> {
        self.sync_writer_generation();
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
            guard.writer_generation = ast_sgrep_core::read_writer_generation(
                &self.root,
                self.index_path.as_deref(),
            );
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

    fn restore_searcher(&self, root: PathBuf, limit: usize, generation: u64, searcher: Searcher) {
        let key = self.searcher_key(root, limit);
        let mut guard = Self::lock_or_recover(&self.searcher_cache, |cache| {
            cache.generation = cache.generation.wrapping_add(1);
            cache.entry = None;
        });
        if guard.generation == generation && guard.entry.is_none() {
            guard.entry = Some((key, searcher));
        }
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
        } = args;
        let (searcher, generation) = self.searcher_for(root.clone(), limit)?;
        let response = match mode {
            AgentSearchMode::Keyword => searcher.search_lexical(&query),
            AgentSearchMode::Ast => searcher.search(&format!("pattern: {query}")),
            AgentSearchMode::Semantic => searcher.search_semantic(&query),
        };
        self.restore_searcher(root.clone(), limit, generation, searcher);
        let response = response?;
        // 6a3i: a miss is the cheapest response we can send, and the one where
        // a vague answer costs the most in speculative agent retries.
        if response.hits.is_empty() {
            let miss = to_compact_miss_json(&query, &self.diagnose_miss(&root, mode));
            return Ok(serde_json::to_string(&miss)?);
        }
        // kxmc: compact key-free envelope, minified. Object keys and pretty
        // whitespace were the bulk of the old AgentCapsule payload, and the
        // full path was emitted twice per hit (`file` plus `ref`).
        let mut envelope = match budget_tokens {
            Some(max_tokens) => to_budgeted_compact_json(
                &response,
                OutputBudget {
                    max_tokens,
                    default_detail: DetailLevel::Full,
                },
            ),
            None => format_response_with_budget(
                &response,
                OutputFormat::Compact,
                0,
                CompactBudget::default(),
            ),
        };
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
            let resolved = self.resolve_compact_id(id);
            nodes.push(read_node(&root, &resolved, context_lines, budget)?);
        }
        Ok(serde_json::to_string(&json!({"nodes": nodes}))?)
    }

    fn tool_index_status(&self, args: IndexStatusArgs) -> anyhow::Result<String> {
        let indexer = Indexer::new(self.base_index_options(args.root))?;
        Ok(serde_json::to_string(&indexer.store().status()?)?)
    }

    fn tool_index_repo(&self, args: IndexRepoArgs) -> anyhow::Result<String> {
        let IndexRepoArgs { root, force } = args;
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
        // index_all commits SQLite before sidecar rebuild; Err may still mean
        // durable mutation. Capture result then always sync session caches.
        let result = if force {
            indexer.reindex_all()
        } else {
            indexer.index_all()
        };
        // Always drop cached Searcher / path ids / elisions after a mutative
        // attempt (Ok or Err). Mid-sidecar Err must not leave a warm Searcher
        // serving pre-mutation hits (R-INDEX-ERR-CACHE-SYNC / d2a1.13).
        self.invalidate_after_index_attempt();
        let stats = result?;
        anyhow::ensure!(
            started.elapsed() <= INDEX_REPO_DEADLINE,
            "index_repo exceeded {}s deadline after mutation (index may have committed; caches were invalidated)",
            INDEX_REPO_DEADLINE.as_secs()
        );
        Ok(serde_json::to_string(&stats)?)
    }

    /// Invalidate Searcher + session path/snippet maps after any index attempt.
    ///
    /// Call on both Ok and Err once `Indexer::new` succeeded and `index_all` /
    /// `reindex_all` returned: bulk commit can land before sidecar rebuild fails.
    fn invalidate_after_index_attempt(&self) {
        self.invalidate_searcher_cache();
        Self::lock_or_recover(&self.path_registry, |registry| registry.clear()).clear();
        Self::lock_or_recover(&self.emitted_snippets, |seen| seen.clear()).clear();
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
    let (selected, total_lines) = scan_line_window(handle, start, wanted_end)?;
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

/// Scan a file handle for lines in `[start, wanted_end]`. TOCTOU checks stay in `read_node`.
fn scan_line_window(
    handle: File,
    start: usize,
    wanted_end: usize,
) -> anyhow::Result<(Vec<String>, usize)> {
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
    Ok((selected, total_lines))
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
    // NDJSON over a pipe is block-buffered; without flush a long-lived MCP host
    // (Cursor/Claude/etc.) never sees the response until the process exits.
    writeln!(stdout, "{body}")?;
    stdout.flush()
}

#[cfg(test)]
mod write_resp_tests {
    use super::*;
    use std::io::{self, Write};

    /// Captures writes and whether `flush` was called (pipe hosts require it).
    struct FlushProbe {
        buf: Vec<u8>,
        flushed: bool,
    }

    impl Write for FlushProbe {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.buf.extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.flushed = true;
            Ok(())
        }
    }

    #[test]
    fn write_resp_flushes_after_each_envelope() {
        let mut probe = FlushProbe {
            buf: Vec::new(),
            flushed: false,
        };
        write_resp(
            &mut probe,
            Some(Value::from(1)),
            Some(json!({"ok": true})),
            None,
        )
        .expect("write");
        assert!(
            probe.flushed,
            "MCP NDJSON over a pipe must flush or clients hang"
        );
        let line = std::str::from_utf8(&probe.buf).expect("utf8");
        assert!(line.ends_with('\n'), "NDJSON line terminator required");
        let value: Value = serde_json::from_str(line.trim_end()).expect("json");
        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["id"], 1);
        assert_eq!(value["result"]["ok"], true);
    }
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
        assert!(
            cache.entry.is_none(),
            "stale searcher returned after reindex"
        );
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
        McpServer::lock_or_recover(&server.emitted_snippets, |_| {}).insert("p0:1-1".into(), 42);

        let args = server
            .parse_index_repo(&json!({}))
            .expect("empty index_repo args should parse");
        let body = server
            .tool_index_repo(args)
            .expect("index_repo should succeed on tiny fixture");
        assert!(
            body.contains("files_indexed") || body.contains("files"),
            "{body}"
        );

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

    /// Pins R-INDEX-ERR-CACHE-SYNC: mid-sidecar Err after bulk commit must still
    /// advance generation and clear path/snippet session maps.
    #[test]
    fn index_repo_invalidates_searcher_on_index_err() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), "fn hello() {}\n").unwrap();
        let server = test_server(root.clone());
        let (searcher, generation) = server.searcher_for(root.clone(), 10).unwrap();
        server.restore_searcher(root.clone(), 10, generation, searcher);
        McpServer::lock_or_recover(&server.path_registry, |_| {})
            .insert("p0".into(), "lib.rs".into());
        McpServer::lock_or_recover(&server.emitted_snippets, |_| {})
            .insert("p0:1-1".into(), 42);

        let args = server
            .parse_index_repo(&json!({}))
            .expect("empty index_repo args should parse");
        let _fail = force_sidecar_rebuild_err();
        let err = server
            .tool_index_repo(args)
            .expect_err("forced sidecar rebuild must surface as index_repo Err");
        assert!(
            err.to_string().contains("forced sidecar rebuild failure"),
            "unexpected error: {err}"
        );

        let cache = McpServer::lock_or_recover(&server.searcher_cache, |_| {});
        assert!(
            cache.entry.is_none(),
            "searcher cache must clear on index_repo Err after possible disk mutation"
        );
        assert!(
            cache.generation != generation,
            "generation must advance on index_repo Err so restore cannot reinstall stale Searcher"
        );
        assert!(
            McpServer::lock_or_recover(&server.path_registry, |_| {}).is_empty(),
            "path registry must clear on index_repo Err"
        );
        assert!(
            McpServer::lock_or_recover(&server.emitted_snippets, |_| {}).is_empty(),
            "emitted snippets must clear on index_repo Err"
        );
    }

    /// Pins R-XPROC-MULTIWRITER Option C lite: an external writer bumping the
    /// durable stamp must drop a warm Searcher without an in-process index_repo.
    #[test]
    fn external_writer_generation_invalidates_warm_searcher() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::write(root.join("lib.rs"), "fn hello() {}\n").unwrap();
        let server = test_server(root.clone());

        let (searcher, generation) = server.searcher_for(root.clone(), 10).unwrap();
        server.restore_searcher(root.clone(), 10, generation, searcher);
        {
            let cache = McpServer::lock_or_recover(&server.searcher_cache, |_| {});
            assert!(cache.entry.is_some(), "precondition: warm Searcher");
        }
        McpServer::lock_or_recover(&server.path_registry, |_| {})
            .insert("p0".into(), "lib.rs".into());

        // Simulate watch / CLI index in another process: bump stamp only.
        let bumped = ast_sgrep_core::bump_writer_generation(&root, None).unwrap();
        assert!(bumped >= 1);

        let (searcher2, generation2) = server.searcher_for(root.clone(), 10).unwrap();
        assert!(
            generation2 != generation,
            "in-process generation must advance when writer stamp changes"
        );
        server.restore_searcher(root, 10, generation2, searcher2);
        let cache = McpServer::lock_or_recover(&server.searcher_cache, |_| {});
        assert_eq!(cache.writer_generation, bumped);
        assert!(
            McpServer::lock_or_recover(&server.path_registry, |_| {}).is_empty(),
            "path registry must clear across writer generations"
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

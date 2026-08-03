//! MCP stdio server for snippet-first hierarchical ast-sgrep retrieval.
//!
//! Warm path: a single process reuses one `Searcher` across search-channel calls
//! (invalidated on `index_repo`) so AI agents avoid per-request SQLite open cost.


#![forbid(unsafe_code)]

use anyhow::Context;
use ast_sgrep_core::{EmbedBackend, IndexOptions, Indexer, SearchOptions, Searcher};
use ast_sgrep_plugins::{format_response_with, OutputFormat};
use serde::Deserialize;
use serde_json::{json, Value};
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

pub struct McpServer {
    /// Configured workspace root; all tool roots must stay under this path.
    root: PathBuf,
    index_path: Option<PathBuf>,
    limit: usize,
    use_embed: bool,
    /// Reused across search-channel calls; cleared after index mutations.
    searcher_cache: Mutex<Option<(SearcherKey, Searcher)>>,
    /// Single-flight lock for index_repo (es7u).
    index_lock: Mutex<()>,
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
            searcher_cache: Mutex::new(None),
            index_lock: Mutex::new(()),
        })
    }

    pub fn run_stdio(&self) -> anyhow::Result<()> {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        for line in stdin.lock().lines() {
            let line = line.context("read stdin")?;
            if line.trim().is_empty() {
                continue;
            }
            let request: JsonRpcRequest = match serde_json::from_str(&line) {
                Ok(req) => req,
                Err(e) => {
                    write_resp(
                        &mut stdout,
                        None,
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
            "query": {"type": "string", "minLength": 1, "maxLength": 4096},
            "root": {"type": "string", "description": "Project root (defaults to ASGREP_ROOT or cwd)"},
            "limit": {"type": "integer", "minimum": 1, "maximum": MAX_AGENT_LIMIT}
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
        json!({"tools": [
            search_tool("keyword_search", "Lexical-only search (FTS/trigram). Returns abbreviated snippets and stable node IDs. Does not fuse AST or semantic channels.", search_properties.clone()),
            search_tool("ast_search", "Native AST/pattern search (pattern: semantics). No external ast-grep process. Returns abbreviated snippets and stable node IDs.", search_properties.clone()),
            search_tool("semantic_search", "Embedding-only search. Requires a non-empty index with semantic chunks. Returns abbreviated snippets and stable node IDs.", search_properties.clone()),
            // Kept for clients still calling the pre-split name; dispatches as Keyword (see dispatch_tool).
            search_tool("code_search", "Deprecated compatibility alias for keyword_search; no automatic fusion across channels.", search_properties),
            {"name": "code_read", "description": "Read full code for result node IDs with optional adjacent-line context. Paths are sandboxed under ASGREP_ROOT.",
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
        // Poison fails closed: clear tainted cache rather than skipping invalidation (bix3).
        let mut guard = Self::lock_or_recover(&self.searcher_cache, |slot| {
            *slot = None;
        });
        *guard = None;
    }

    fn searcher_for(&self, root: PathBuf, limit: usize) -> anyhow::Result<Searcher> {
        let key = self.searcher_key(root.clone(), limit);
        // Poison fails closed: clear and rebuild rather than reuse tainted state (bix3/sxjc).
        let mut guard = Self::lock_or_recover(&self.searcher_cache, |slot| {
            *slot = None;
        });
        let need_new = match guard.as_ref() {
            None => true,
            Some((k, _)) => k != &key,
        };
        if need_new {
            let searcher = Searcher::new(SearchOptions {
                root,
                index_path: self.index_path.clone(),
                limit,
                use_embed: self.use_embed,
                ..SearchOptions::default()
            })?;
            *guard = Some((key.clone(), searcher));
        }
        // Take searcher out so the mutex is not held across search compute (bix3).
        let (_cached_key, searcher) = guard
            .take()
            .ok_or_else(|| anyhow::anyhow!("searcher cache missing after populate"))?;
        drop(guard);
        Ok(searcher)
    }

    fn restore_searcher(&self, root: PathBuf, limit: usize, searcher: Searcher) {
        let key = self.searcher_key(root, limit);
        let mut guard = Self::lock_or_recover(&self.searcher_cache, |slot| {
            *slot = None;
        });
        // Only restore if nothing newer was inserted (single-flight index may have cleared).
        if guard.is_none() {
            *guard = Some((key, searcher));
        }
    }

    fn tool_agent_search(&self, args: &Value, mode: AgentSearchMode) -> anyhow::Result<String> {
        Self::validate_fields(args, &["query", "root", "limit"])?;
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|query| !query.is_empty() && query.chars().count() <= 4_096)
            .context("query must contain 1 to 4096 characters")?;
        let limit = Self::integer_arg(args, "limit", self.limit, 1, MAX_AGENT_LIMIT)?;
        let root = self.root_arg(args)?;
        let searcher = self.searcher_for(root.clone(), limit)?;
        let response = match mode {
            AgentSearchMode::Keyword => searcher.search_lexical(query),
            AgentSearchMode::Ast => searcher.search(&format!("pattern: {query}")),
            AgentSearchMode::Semantic => searcher.search_semantic(query),
        };
        self.restore_searcher(root, limit, searcher);
        let response = response?;
        Ok(serde_json::to_string_pretty(&format_response_with(
            &response,
            OutputFormat::AgentCapsule,
            0,
        ))?)
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
            nodes.push(read_node(&root, id, context_lines, budget)?);
        }
        Ok(serde_json::to_string_pretty(&json!({"nodes": nodes}))?)
    }

    fn tool_index_status(&self, args: &Value) -> anyhow::Result<String> {
        Self::validate_fields(args, &["root"])?;
        let indexer = Indexer::new(self.base_index_options(self.root_arg(args)?))?;
        Ok(serde_json::to_string_pretty(&indexer.store().status()?)?)
    }

    fn tool_index_repo(&self, args: &Value) -> anyhow::Result<String> {
        Self::validate_fields(args, &["root", "force"])?;
        let force = match args.get("force") {
            None => false,
            Some(value) => value.as_bool().context("force must be a boolean")?,
        };
        let root = self.root_arg(args)?;
        let _flight = Self::lock_or_recover(&self.index_lock, |_| {});
        let started = Instant::now();
        let mut indexer = Indexer::new(IndexOptions {
            embed_semantic: self.use_embed,
            embed_backend: if self.use_embed {
                EmbedBackend::Auto
            } else {
                EmbedBackend::Semantic
            },
            ..self.base_index_options(root)
        })?;
        // Soft deadline: refuse to start when the prior wait already exhausted the budget.
        anyhow::ensure!(
            started.elapsed() < INDEX_REPO_DEADLINE,
            "index_repo exceeded {}s single-flight deadline before start",
            INDEX_REPO_DEADLINE.as_secs()
        );
        let stats = if force {
            indexer.reindex_all()?
        } else {
            indexer.index_all()?
        };
        anyhow::ensure!(
            started.elapsed() <= INDEX_REPO_DEADLINE,
            "index_repo exceeded {}s deadline",
            INDEX_REPO_DEADLINE.as_secs()
        );
        // Index changed — drop cached Searcher so next search sees fresh data.
        self.invalidate_searcher_cache();
        Ok(serde_json::to_string_pretty(&stats)?)
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

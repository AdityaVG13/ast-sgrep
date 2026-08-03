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
use std::sync::Mutex;

const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "ast-sgrep";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_AGENT_LIMIT: usize = 100;
const MAX_READ_REFS: usize = 20;
const MAX_CONTEXT_LINES: usize = 100;
const DEFAULT_READ_CHARS: usize = 100_000;
const MAX_READ_CHARS: usize = 1_000_000;
const MAX_SCAN_BYTES: u64 = 64 * 1024 * 1024;

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
    root: PathBuf,
    index_path: Option<PathBuf>,
    limit: usize,
    use_embed: bool,
    /// Reused across search-channel calls; cleared after index mutations.
    searcher_cache: Mutex<Option<(SearcherKey, Searcher)>>,
}

impl McpServer {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            root: std::env::var("ASGREP_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))),
            index_path: std::env::var("ASGREP_INDEX_PATH").ok().map(PathBuf::from),
            limit: std::env::var("ASGREP_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(SearchOptions::default_limit)
                .clamp(1, MAX_AGENT_LIMIT),
            use_embed: std::env::var("ASGREP_NO_EMBED")
                .ok()
                .filter(|v| {
                    matches!(
                        v.trim().to_ascii_lowercase().as_str(),
                        "1" | "true" | "yes" | "on"
                    )
                })
                .is_none(),
            searcher_cache: Mutex::new(None),
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
        json!({"tools": [
            {"name": "keyword_search", "description": "Lexical-only search returning abbreviated snippets and stable node IDs.",
             "inputSchema": {"type": "object", "properties": search_properties.clone(), "required": ["query"], "additionalProperties": false}},
            {"name": "ast_search", "description": "AST pattern search returning abbreviated snippets and stable node IDs.",
             "inputSchema": {"type": "object", "properties": search_properties.clone(), "required": ["query"], "additionalProperties": false}},
            {"name": "semantic_search", "description": "Embedding-only search returning abbreviated snippets and stable node IDs.",
             "inputSchema": {"type": "object", "properties": search_properties.clone(), "required": ["query"], "additionalProperties": false}},
            {"name": "code_search", "description": "Deprecated compatibility alias for keyword_search; no automatic fusion.",
             "inputSchema": {"type": "object", "properties": search_properties, "required": ["query"], "additionalProperties": false}},
            {"name": "code_read", "description": "Read full code for result node IDs with optional adjacent-line context.",
             "inputSchema": {"type": "object", "properties": {
                "ids": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": MAX_READ_REFS},
                "root": {"type": "string", "description": "Project root"},
                "context_lines": {"type": "integer", "minimum": 0, "maximum": MAX_CONTEXT_LINES},
                "max_chars": {"type": "integer", "minimum": 1, "maximum": MAX_READ_CHARS}
             }, "required": ["ids"], "additionalProperties": false}},
            {"name": "index_status", "description": "Show ast-sgrep index statistics for a project root.",
             "inputSchema": {"type": "object", "properties": {"root": {"type": "string"}}, "additionalProperties": false}},
            {"name": "index_repo", "description": "Build or incrementally update the ast-sgrep index.",
             "inputSchema": {"type": "object", "properties": {
                "root": {"type": "string"}, "force": {"type": "boolean"}}, "additionalProperties": false}}
        ]})
    }

    fn handle_tools_call(&self, params: &Value) -> Option<Value> {
        let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        let result = match name {
            "keyword_search" => self.tool_agent_search(&args, AgentSearchMode::Keyword),
            "ast_search" => self.tool_agent_search(&args, AgentSearchMode::Ast),
            "semantic_search" => self.tool_agent_search(&args, AgentSearchMode::Semantic),
            "code_search" => self.tool_agent_search(&args, AgentSearchMode::Keyword),
            "code_read" => self.tool_code_read(&args),
            "index_status" => self.tool_index_status(&args),
            "index_repo" => self.tool_index_repo(&args),
            other => Err(anyhow::anyhow!("unknown tool: {other}")),
        };
        Some(match result {
            Ok(text) => json!({"content": [{"type": "text", "text": text}], "isError": false}),
            Err(e) => {
                json!({"content": [{"type": "text", "text": e.to_string()}], "isError": true})
            }
        })
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
        match args.get("root") {
            None => Ok(self.root.clone()),
            Some(value) => Ok(PathBuf::from(
                value.as_str().context("root must be a string")?,
            )),
        }
    }

    fn invalidate_searcher_cache(&self) {
        if let Ok(mut guard) = self.searcher_cache.lock() {
            *guard = None;
        }
    }

    fn searcher_for(
        &self,
        root: PathBuf,
        limit: usize,
    ) -> anyhow::Result<std::sync::MutexGuard<'_, Option<(SearcherKey, Searcher)>>> {
        let key = SearcherKey {
            root: root.clone(),
            index_path: self.index_path.clone(),
            limit,
            use_embed: self.use_embed,
        };
        let mut guard = self
            .searcher_cache
            .lock()
            .map_err(|_| anyhow::anyhow!("searcher cache lock poisoned"))?;
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
            *guard = Some((key, searcher));
        }
        Ok(guard)
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
        let guard = self.searcher_for(root, limit)?;
        let searcher = &guard.as_ref().expect("searcher_for populates cache").1;
        let response = match mode {
            AgentSearchMode::Keyword => searcher.search_lexical(query)?,
            AgentSearchMode::Ast => searcher.search(&format!("pattern: {query}"))?,
            AgentSearchMode::Semantic => searcher.search_semantic(query)?,
        };
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
        let indexer = Indexer::new(IndexOptions {
            root: self.root_arg(args)?,
            index_path: self.index_path.clone(),
            ..IndexOptions::default()
        })?;
        Ok(serde_json::to_string_pretty(&indexer.store().status()?)?)
    }

    fn tool_index_repo(&self, args: &Value) -> anyhow::Result<String> {
        Self::validate_fields(args, &["root", "force"])?;
        let force = match args.get("force") {
            None => false,
            Some(value) => value.as_bool().context("force must be a boolean")?,
        };
        let mut indexer = Indexer::new(IndexOptions {
            root: self.root_arg(args)?,
            index_path: self.index_path.clone(),
            embed_backend: EmbedBackend::Auto,
            ..IndexOptions::default()
        })?;
        let stats = if force {
            indexer.reindex_all()?
        } else {
            indexer.index_all()?
        };
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

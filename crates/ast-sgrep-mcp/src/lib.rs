use anyhow::Context;
use ast_sgrep_core::{EmbedBackend, IndexOptions, Indexer, SearchOptions, Searcher};
use ast_sgrep_plugins::{format_response, OutputFormat};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "ast-sgrep";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}
pub struct McpServer {
    root: PathBuf,
    index_path: Option<PathBuf>,
    limit: usize,
    use_embed: bool,
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
                .unwrap_or_else(SearchOptions::default_limit),
            use_embed: std::env::var("ASGREP_NO_EMBED").ok().as_deref() != Some("1"),
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
                    write_response(
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
                    Ok(result) => write_response(&mut stdout, request.id, Some(result), None)?,
                    Err(error) => write_response(&mut stdout, request.id, None, Some(error))?,
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
            _ => Err(json!({
                "code": -32601,
                "message": format!("method not found: {}", request.method),
            })),
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
        json!({
            "tools": [
                {
                    "name": "code_search",
                    "description": "Hybrid code search: lexical + symbols + call graph + semantic. Supports defs:, callers:, NL queries.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": { "type": "string", "description": "Search query" },
                            "root": { "type": "string", "description": "Project root (defaults to ASGREP_ROOT or cwd)" },
                            "semantic_only": { "type": "boolean", "description": "Semantic/embed pass only" },
                            "limit": { "type": "integer", "description": "Max hits (default ASGREP_LIMIT or 16)" }
                        },
                        "required": ["query"]
                    }
                },
                {
                    "name": "index_status",
                    "description": "Show ast-sgrep index statistics for a project root.",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "root": { "type": "string", "description": "Project root" } }
                    }
                },
                {
                    "name": "index_repo",
                    "description": "Build or incrementally update the ast-sgrep index.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "root": { "type": "string", "description": "Project root" },
                            "force": { "type": "boolean", "description": "Force full reindex" }
                        }
                    }
                }
            ]
        })
    }

    fn handle_tools_call(&self, params: &Value) -> Option<Value> {
        let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        let result = match name {
            "code_search" => self.tool_code_search(&args),
            "index_status" => self.tool_index_status(&args),
            "index_repo" => self.tool_index_repo(&args),
            other => Err(anyhow::anyhow!("unknown tool: {other}")),
        };
        Some(match result {
            Ok(text) => json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
            Err(e) => {
                json!({ "content": [{ "type": "text", "text": e.to_string() }], "isError": true })
            }
        })
    }

    fn root_arg(&self, args: &Value) -> PathBuf {
        args.get("root")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root.clone())
    }

    fn tool_code_search(&self, args: &Value) -> anyhow::Result<String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .context("query is required")?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(self.limit);
        let semantic_only = args
            .get("semantic_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let searcher = Searcher::new(SearchOptions {
            root: self.root_arg(args),
            index_path: self.index_path.clone(),
            limit,
            use_embed: self.use_embed,
            ..SearchOptions::default()
        })?;
        let response = if semantic_only {
            searcher.search_semantic(query)?
        } else {
            searcher.search(query)?
        };
        Ok(serde_json::to_string_pretty(&format_response(
            &response,
            OutputFormat::Agent,
        ))?)
    }

    fn tool_index_status(&self, args: &Value) -> anyhow::Result<String> {
        let indexer = Indexer::new(IndexOptions {
            root: self.root_arg(args),
            index_path: self.index_path.clone(),
            ..IndexOptions::default()
        })?;
        Ok(serde_json::to_string_pretty(&indexer.store().status()?)?)
    }

    fn tool_index_repo(&self, args: &Value) -> anyhow::Result<String> {
        let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
        let mut indexer = Indexer::new(IndexOptions {
            root: self.root_arg(args),
            index_path: self.index_path.clone(),
            embed_backend: EmbedBackend::Auto,
            ..IndexOptions::default()
        })?;
        let stats = if force {
            indexer.reindex_all()?
        } else {
            indexer.index_all()?
        };
        Ok(serde_json::to_string_pretty(&stats)?)
    }
}
fn write_response(
    stdout: &mut impl Write,
    id: Option<Value>,
    result: Option<Value>,
    error: Option<Value>,
) -> io::Result<()> {
    let mut body = json!({ "jsonrpc": "2.0" });
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

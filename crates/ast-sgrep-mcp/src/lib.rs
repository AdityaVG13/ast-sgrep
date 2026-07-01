//! Minimal MCP (Model Context Protocol) stdio server for ast-sgrep.
//!
//! Exposes hybrid code search, index status, and incremental indexing to AI agents
//! without requiring the rmcp SDK (compatible with Rust 1.83).

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use anyhow::Context;
use ast_sgrep_core::{EmbedBackend, IndexOptions, Indexer, SearchOptions, Searcher};
use ast_sgrep_plugins::{format_response, OutputFormat};
use serde::Deserialize;
use serde_json::{json, Value};

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
        let root = std::env::var("ASGREP_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let index_path = std::env::var("ASGREP_INDEX_PATH").ok().map(PathBuf::from);
        let limit = std::env::var("ASGREP_LIMIT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(SearchOptions::default_limit);
        let use_embed = std::env::var("ASGREP_NO_EMBED").ok().as_deref() != Some("1");
        Ok(Self {
            root,
            index_path,
            limit,
            use_embed,
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
            let response = self.handle_request(&request);
            if let Some(result) = response {
                write_response(&mut stdout, request.id, Some(result), None)?;
            }
        }
        Ok(())
    }

    fn handle_request(&self, request: &JsonRpcRequest) -> Option<Value> {
        if request.id.is_none() {
            // Notifications (e.g. initialized) — no response.
            return None;
        }
        match request.method.as_str() {
            "initialize" => Some(self.handle_initialize()),
            "tools/list" => Some(self.handle_tools_list()),
            "tools/call" => self.handle_tools_call(&request.params),
            "ping" => Some(json!({})),
            _ => Some(json!({
                "content": [{
                    "type": "text",
                    "text": format!("unsupported method: {}", request.method),
                }],
                "isError": true,
            })),
        }
    }

    fn handle_initialize(&self) -> Value {
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": SERVER_NAME,
                "version": SERVER_VERSION,
            }
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
                        "properties": {
                            "root": { "type": "string", "description": "Project root" }
                        }
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
        match result {
            Ok(text) => Some(json!({
                "content": [{ "type": "text", "text": text }],
                "isError": false,
            })),
            Err(e) => Some(json!({
                "content": [{ "type": "text", "text": e.to_string() }],
                "isError": true,
            })),
        }
    }

    fn tool_code_search(&self, args: &Value) -> anyhow::Result<String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .context("query is required")?;
        let root = args
            .get("root")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root.clone());
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
            root: root.clone(),
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
        let value = format_response(&response, OutputFormat::Agent);
        Ok(serde_json::to_string_pretty(&value)?)
    }

    fn tool_index_status(&self, args: &Value) -> anyhow::Result<String> {
        let root = args
            .get("root")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root.clone());
        let indexer = Indexer::new(IndexOptions {
            root,
            index_path: self.index_path.clone(),
            ..IndexOptions::default()
        })?;
        let status = indexer.store().status()?;
        Ok(serde_json::to_string_pretty(&status)?)
    }

    fn tool_index_repo(&self, args: &Value) -> anyhow::Result<String> {
        let root = args
            .get("root")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root.clone());
        let force = args
            .get("force")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let mut indexer = Indexer::new(IndexOptions {
            root,
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
    writeln!(stdout, "{}", body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_list_includes_code_search() {
        let server = McpServer {
            root: PathBuf::from("."),
            index_path: None,
            limit: 16,
            use_embed: true,
        };
        let list = server.handle_tools_list();
        let names: Vec<_> = list["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(names.contains(&"code_search"));
        assert!(names.contains(&"index_status"));
        assert!(names.contains(&"index_repo"));
    }

    #[test]
    fn initialize_returns_capabilities() {
        let server = McpServer {
            root: PathBuf::from("."),
            index_path: None,
            limit: 16,
            use_embed: true,
        };
        let init = server.handle_initialize();
        assert_eq!(init["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(init["serverInfo"]["name"], SERVER_NAME);
    }
}

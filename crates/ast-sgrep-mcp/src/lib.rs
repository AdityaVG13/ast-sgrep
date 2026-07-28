//! MCP stdio server for ast-sgrep hybrid search.
//!
//! Warm path: a single process reuses one `Searcher` across `code_search` calls
//! (invalidated on `index_repo`) so AI agents avoid per-request SQLite open cost.

use anyhow::{bail, Context};
use ast_sgrep_core::{EmbedBackend, IndexOptions, Indexer, SearchOptions, Searcher};
use ast_sgrep_plugins::{format_response, OutputFormat};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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

#[derive(Clone, PartialEq, Eq)]
struct SearcherKey {
    root: PathBuf,
    index_path: Option<PathBuf>,
    limit: usize,
    use_embed: bool,
}

pub struct McpServer {
    /// Canonical allowlisted project root (sandbox base).
    root: PathBuf,
    index_path: Option<PathBuf>,
    limit: usize,
    use_embed: bool,
    /// When true, skip path confinement (explicit escape hatch for trusted hosts).
    allow_any_root: bool,
    /// Reused across tools/call code_search; cleared after index mutations.
    searcher_cache: Mutex<Option<(SearcherKey, Searcher)>>,
}

impl McpServer {
    pub fn from_env() -> anyhow::Result<Self> {
        let allow_any_root = env_truthy("ASGREP_MCP_ALLOW_ANY_ROOT");
        let raw_root = std::env::var("ASGREP_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let root = canonicalize_path(&raw_root).unwrap_or(raw_root);
        let index_path = match std::env::var("ASGREP_INDEX_PATH") {
            Ok(p) => {
                let p = PathBuf::from(p);
                Some(if allow_any_root {
                    p
                } else {
                    confine_path(&root, &p).with_context(|| {
                        format!(
                            "ASGREP_INDEX_PATH must stay under allowed root {} (or set ASGREP_MCP_ALLOW_ANY_ROOT=1)",
                            root.display()
                        )
                    })?
                })
            }
            Err(_) => None,
        };
        Ok(Self {
            root,
            index_path,
            limit: std::env::var("ASGREP_LIMIT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(SearchOptions::default_limit),
            use_embed: std::env::var("ASGREP_NO_EMBED").ok().as_deref() != Some("1"),
            allow_any_root,
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
        json!({"tools": [
            {"name": "code_search", "description": "Hybrid code search: lexical + symbols + call graph + semantic. Supports defs:, callers:, NL queries.",
             "inputSchema": {"type": "object", "properties": {
                "query": {"type": "string", "description": "Search query"},
                "root": {"type": "string", "description": "Project root (defaults to ASGREP_ROOT or cwd)"},
                "semantic_only": {"type": "boolean", "description": "Semantic/embed pass only"},
                "limit": {"type": "integer", "description": "Max hits (default ASGREP_LIMIT or 16)"}
             }, "required": ["query"]}},
            {"name": "index_status",
             "description": "Show ast-sgrep index statistics for a project root.",
             "inputSchema": {"type": "object", "properties": {"root": {"type": "string", "description": "Project root"}}}},
            {"name": "index_repo",
             "description": "Build or incrementally update the ast-sgrep index.",
             "inputSchema": {"type": "object", "properties": {
                "root": {"type": "string", "description": "Project root"},
                "force": {"type": "boolean", "description": "Force full reindex"}}}}
        ]})
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
            Ok(text) => json!({"content": [{"type": "text", "text": text}], "isError": false}),
            Err(e) => {
                json!({"content": [{"type": "text", "text": e.to_string()}], "isError": true})
            }
        })
    }

    fn root_arg(&self, args: &Value) -> anyhow::Result<PathBuf> {
        let raw = args
            .get("root")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.root.clone());
        if self.allow_any_root {
            return Ok(canonicalize_path(&raw).unwrap_or(raw));
        }
        confine_path(&self.root, &raw).with_context(|| {
            format!(
                "tool root must stay under allowed root {} (got {}); set ASGREP_MCP_ALLOW_ANY_ROOT=1 to disable sandbox",
                self.root.display(),
                raw.display()
            )
        })
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

    fn tool_code_search(&self, args: &Value) -> anyhow::Result<String> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .context("query is required")?;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(self.limit)
            .clamp(1, 500);
        let semantic_only = args
            .get("semantic_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let root = self.root_arg(args)?;
        let guard = self.searcher_for(root, limit)?;
        let searcher = &guard.as_ref().expect("searcher_for populates cache").1;
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
            root: self.root_arg(args)?,
            index_path: self.index_path.clone(),
            ..IndexOptions::default()
        })?;
        Ok(serde_json::to_string_pretty(&indexer.store().status()?)?)
    }

    fn tool_index_repo(&self, args: &Value) -> anyhow::Result<String> {
        let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
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

fn env_truthy(name: &str) -> bool {
    matches!(
        std::env::var(name).ok().as_deref(),
        Some("1" | "true" | "TRUE" | "yes" | "YES" | "on" | "ON")
    )
}

fn canonicalize_path(path: &Path) -> Option<PathBuf> {
    path.canonicalize().ok()
}

/// Resolve `path` and require it stays under `allowed_root` (no `..` escape, no absolute outsides).
fn confine_path(allowed_root: &Path, path: &Path) -> anyhow::Result<PathBuf> {
    let base = canonicalize_path(allowed_root).unwrap_or_else(|| allowed_root.to_path_buf());
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    // If the path exists, canonicalize to collapse `..` / symlinks. If not, canonicalize the
    // longest existing prefix and rejoin the rest so non-existent roots still get confined.
    let candidate = match canonicalize_path(&joined) {
        Some(c) => c,
        None => {
            let mut prefix = joined.clone();
            let mut suffix = Vec::new();
            while !prefix.exists() {
                match (prefix.file_name(), prefix.parent()) {
                    (Some(name), Some(parent)) => {
                        suffix.push(name.to_os_string());
                        prefix = parent.to_path_buf();
                    }
                    _ => break,
                }
            }
            let mut resolved = canonicalize_path(&prefix).unwrap_or(prefix);
            for part in suffix.into_iter().rev() {
                resolved.push(part);
            }
            resolved
        }
    };
    if !path_is_within(&candidate, &base) {
        bail!(
            "path {} is outside allowed root {}",
            candidate.display(),
            base.display()
        );
    }
    Ok(candidate)
}

fn path_is_within(path: &Path, root: &Path) -> bool {
    if path == root {
        return true;
    }
    path.starts_with(root)
}

#[cfg(test)]
mod sandbox_tests {
    use super::{confine_path, path_is_within};
    use std::path::PathBuf;

    #[test]
    fn rejects_absolute_escape() {
        let base = std::env::temp_dir();
        let outside = if cfg!(windows) {
            PathBuf::from(r"C:\Windows")
        } else {
            PathBuf::from("/etc")
        };
        assert!(confine_path(&base, &outside).is_err());
    }

    #[test]
    fn accepts_subdir() {
        let base = std::env::temp_dir();
        let sub = base.join("asgrep-mcp-sandbox-test-sub");
        let _ = std::fs::create_dir_all(&sub);
        let got = confine_path(&base, &sub).expect("subdir allowed");
        assert!(path_is_within(&got, &base.canonicalize().unwrap_or(base)));
        let _ = std::fs::remove_dir_all(&sub);
    }

    #[test]
    fn rejects_dotdot_escape() {
        let base = std::env::temp_dir();
        // Relative escape: temp/../.. would leave temp on Unix.
        let escape = PathBuf::from("..").join("..").join("etc");
        // May succeed canonicalize to /etc and then fail confinement.
        let result = confine_path(&base, &escape);
        if let Ok(p) = result {
            assert!(
                path_is_within(&p, &base.canonicalize().unwrap_or(base.clone())),
                "unexpected escape success to {}",
                p.display()
            );
        }
    }
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
    writeln!(stdout, "{body}")?;
    stdout.flush()
}

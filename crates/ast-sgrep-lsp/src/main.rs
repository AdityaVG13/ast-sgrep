use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use ast_sgrep_lsp::LspBackend;
use serde_json::{json, Value};

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut backend: Option<LspBackend> = None;

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = serde_json::from_str(&line)?;
        let id = msg.get("id").cloned();

        if msg.get("method").is_some() && id.is_none() {
            // notification
            if msg["method"] == "initialize" {
                let root = msg["params"]["rootUri"]
                    .as_str()
                    .and_then(|u| u.strip_prefix("file://"))
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."));
                let b = LspBackend::new(root);
                let _ = b.ensure_index();
                backend = Some(b);
                continue;
            }
            continue;
        }

        if let Some(id) = id {
            let method = msg["method"].as_str().unwrap_or("");
            let params = &msg["params"];
            let result = match method {
                "initialize" => {
                    let root = params["rootUri"]
                        .as_str()
                        .and_then(|u| u.strip_prefix("file://"))
                        .map(PathBuf::from)
                        .unwrap_or_else(|| PathBuf::from("."));
                    let b = LspBackend::new(root);
                    let _ = b.ensure_index();
                    backend = Some(b);
                    json!({
                        "capabilities": {
                            "workspaceSymbolProvider": true,
                            "definitionProvider": true,
                            "referencesProvider": true
                        },
                        "serverInfo": { "name": "asgrep-lsp", "version": env!("CARGO_PKG_VERSION") }
                    })
                }
                "workspace/symbol" => {
                    let query = params["query"].as_str().unwrap_or("");
                    backend
                        .as_ref()
                        .map(|b| b.workspace_symbols(query).unwrap_or(json!([])))
                        .unwrap_or(json!([]))
                }
                "textDocument/definition" => {
                    let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                    let sym = uri.rsplit('/').next().unwrap_or("main");
                    backend
                        .as_ref()
                        .map(|b| b.goto_definition(sym).unwrap_or(Value::Null))
                        .unwrap_or(Value::Null)
                }
                "textDocument/references" => {
                    let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
                    let sym = uri.rsplit('/').next().unwrap_or("main");
                    backend
                        .as_ref()
                        .map(|b| b.find_references(sym).unwrap_or(json!([])))
                        .unwrap_or(json!([]))
                }
                "shutdown" => Value::Null,
                _ => Value::Null,
            };
            let resp = json!({ "jsonrpc": "2.0", "id": id, "result": result });
            writeln!(stdout, "{resp}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

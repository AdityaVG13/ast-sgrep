//! LSP request dispatch.

use std::io::{self, BufReader, Write};

use serde_json::Value;

use crate::backend::LspBackend;
use crate::transport::{read_message, send_error, send_response};
use crate::types::{
    CallHierarchyItemParams, CallHierarchyPrepareParams, DocumentSymbolParams,
    ExecuteCommandParams, InitializeParams, NotificationMessage, ReferenceParams, RequestMessage,
    TextDocumentPositionParams, WorkspaceSymbolParams,
};

pub struct LspServer {
    backend: Option<LspBackend>,
    shutdown: bool,
}

impl LspServer {
    pub fn new() -> Self {
        Self {
            backend: None,
            shutdown: false,
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        let mut reader = BufReader::new(stdin.lock());

        while let Some(body) = read_message(&mut reader)? {
            if let Ok(req) = serde_json::from_str::<RequestMessage>(&body) {
                self.handle_request(&mut stdout, req)?;
                if self.shutdown {
                    break;
                }
            } else if let Ok(notif) = serde_json::from_str::<NotificationMessage>(&body) {
                self.handle_notification(notif)?;
            }
        }
        Ok(())
    }

    fn handle_request(&mut self, stdout: &mut impl Write, req: RequestMessage) -> io::Result<()> {
        let result = self.dispatch(&req.method, &req.params);
        match result {
            Ok(value) => send_response(stdout, &req.id, value)?,
            Err(e) => send_error(stdout, &req.id, -32603, &e.to_string())?,
        }
        Ok(())
    }

    fn handle_notification(&mut self, notif: NotificationMessage) -> io::Result<()> {
        match notif.method.as_str() {
            "initialized" => {}
            "textDocument/didSave" => {
                if let Some(backend) = &self.backend {
                    if let Ok(params) = serde_json::from_value::<crate::types::DidSaveTextDocumentParams>(notif.params) {
                        if let Ok(rel) = crate::backend::uri_to_rel_path(
                            &params.text_document.uri,
                            backend.root(),
                        ) {
                            let _ = backend.reindex_file(&rel);
                        }
                    }
                }
            }
            "textDocument/didChange" => {
                if let Some(backend) = &self.backend {
                    if let Ok(params) =
                        serde_json::from_value::<crate::types::DidChangeTextDocumentParams>(notif.params)
                    {
                        let _ = backend.apply_document_changes(
                            &params.text_document.uri,
                            &params.content_changes,
                        );
                    }
                }
            }
            "exit" => self.shutdown = true,
            _ => {}
        }
        Ok(())
    }

    fn dispatch(&mut self, method: &str, params: &Value) -> anyhow::Result<Value> {
        match method {
            "initialize" => {
                let params: InitializeParams = serde_json::from_value(params.clone())?;
                let root = resolve_root(&params);
                let mut backend = LspBackend::new(root);
                backend.start_background_index();
                let result = backend.initialize_result();
                self.backend = Some(backend);
                Ok(result)
            }
            "shutdown" => {
                self.shutdown = true;
                Ok(Value::Null)
            }
            "workspace/symbol" => {
                let backend = self.backend()?;
                let params: WorkspaceSymbolParams = serde_json::from_value(params.clone())?;
                backend.workspace_symbols(&params.query)
            }
            "textDocument/documentSymbol" => {
                let backend = self.backend()?;
                let params: DocumentSymbolParams = serde_json::from_value(params.clone())?;
                backend.document_symbols(&params)
            }
            "textDocument/definition" => {
                let backend = self.backend()?;
                let params: TextDocumentPositionParams = serde_json::from_value(params.clone())?;
                backend.goto_definition(&params)
            }
            "textDocument/references" => {
                let backend = self.backend()?;
                let params: ReferenceParams = serde_json::from_value(params.clone())?;
                backend.find_references(&params)
            }
            "callHierarchy/prepareCallHierarchy" => {
                let backend = self.backend()?;
                let params: CallHierarchyPrepareParams = serde_json::from_value(params.clone())?;
                let pos = TextDocumentPositionParams {
                    text_document: params.text_document,
                    position: params.position,
                };
                backend.prepare_call_hierarchy(&pos)
            }
            "callHierarchy/incomingCalls" => {
                let backend = self.backend()?;
                let params: CallHierarchyItemParams = serde_json::from_value(params.clone())?;
                backend.incoming_calls(&params.item)
            }
            "callHierarchy/outgoingCalls" => {
                let backend = self.backend()?;
                let params: CallHierarchyItemParams = serde_json::from_value(params.clone())?;
                backend.outgoing_calls(&params.item)
            }
            "workspace/executeCommand" => {
                let backend = self.backend()?;
                let params: ExecuteCommandParams = serde_json::from_value(params.clone())?;
                backend.execute_command(&params)
            }
            _ => Ok(Value::Null),
        }
    }

    fn backend(&self) -> anyhow::Result<&LspBackend> {
        self.backend
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("server not initialized"))
    }
}

fn resolve_root(params: &InitializeParams) -> std::path::PathBuf {
    if let Some(folders) = &params.workspace_folders {
        if let Some(first) = folders.first() {
            if let Ok(p) = uri_to_path(&first.uri) {
                return p;
            }
        }
    }
    if let Some(uri) = &params.root_uri {
        if let Ok(p) = uri_to_path(uri) {
            return p;
        }
    }
    if let Some(path) = &params.root_path {
        return std::path::PathBuf::from(path);
    }
    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
}

fn uri_to_path(uri: &str) -> anyhow::Result<std::path::PathBuf> {
    let stripped = uri
        .strip_prefix("file://")
        .or_else(|| uri.strip_prefix("file:///"))
        .unwrap_or(uri);
    Ok(std::path::PathBuf::from(stripped))
}

/// Log to stderr (LSP allows logging without breaking protocol).
pub fn log(msg: &str) {
    let _ = writeln!(io::stderr(), "[asgrep-lsp] {msg}");
}

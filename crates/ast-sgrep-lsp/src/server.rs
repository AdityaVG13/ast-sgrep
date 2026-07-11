use std::io::{self, BufReader, Write};
use serde_json::Value;
use crate::backend::LspBackend;
use crate::transport::{read_message, send_error, send_response};
use crate::types::{
    CallHierarchyItemParams, CallHierarchyPrepareParams, DocumentSymbolParams,
    ExecuteCommandParams, InitializeParams, NotificationMessage, ReferenceParams, RequestMessage,
    SearchParams, TextDocumentPositionParams, WorkspaceSymbolParams,
};
use crate::uri::{canonicalize_workspace_root, file_uri_to_path};
pub struct LspServer {
    backend: Option<LspBackend>,
    shutdown: bool,
}
type RequestHandler = fn(&mut LspServer, &Value) -> anyhow::Result<Value>;
const REQUEST_HANDLERS: &[(&str, RequestHandler)] = &[
    ("initialize", LspServer::dispatch_initialize),
    ("shutdown", LspServer::dispatch_shutdown),
    ("workspace/symbol", LspServer::dispatch_workspace_symbol),
    ("asgrep/search", LspServer::dispatch_search),
    ("textDocument/documentSymbol", LspServer::dispatch_document_symbol),
    ("textDocument/definition", LspServer::dispatch_definition),
    ("textDocument/references", LspServer::dispatch_references),
    ("callHierarchy/prepareCallHierarchy", LspServer::dispatch_prepare_call_hierarchy),
    ("callHierarchy/incomingCalls", LspServer::dispatch_incoming_calls),
    ("callHierarchy/outgoingCalls", LspServer::dispatch_outgoing_calls),
    ("workspace/executeCommand", LspServer::dispatch_execute_command),
];
impl Default for LspServer {
    fn default() -> Self {
        Self::new()
    }
}
impl LspServer {
    pub fn new() -> Self {
        Self { backend: None, shutdown: false }
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
        match self.dispatch(&req.method, &req.params) {
            Ok(value) => send_response(stdout, &req.id, value)?,
            Err(e) => {
                let code = if e.to_string().contains("not found") { -32601 } else { -32603 };
                send_error(stdout, &req.id, code, &e.to_string())?;
            }
        }
        Ok(())
    }

    fn handle_notification(&mut self, notif: NotificationMessage) -> io::Result<()> {
        match notif.method.as_str() {
            "initialized" => {}
            "textDocument/didOpen" => {
                if let (Some(backend), Ok(params)) = (
                    &self.backend,
                    serde_json::from_value::<crate::types::DidOpenTextDocumentParams>(notif.params),
                ) {
                    if let Ok(rel) = crate::uri::uri_to_rel_path(&params.text_document.uri, backend.root()) {
                        let _ = backend.index_content(&rel, &params.text_document.text);
                    }
                }
            }
            "textDocument/didSave" => {
                if let (Some(backend), Ok(params)) = (
                    &self.backend,
                    serde_json::from_value::<crate::types::DidSaveTextDocumentParams>(notif.params),
                ) {
                    if let Ok(rel) = crate::uri::uri_to_rel_path(&params.text_document.uri, backend.root()) {
                        let _ = backend.reindex_file(&rel);
                    }
                }
            }
            "textDocument/didChange" => {
                if let (Some(backend), Ok(params)) = (
                    &self.backend,
                    serde_json::from_value::<crate::types::DidChangeTextDocumentParams>(notif.params),
                ) {
                    let _ = backend.apply_document_changes(
                        &params.text_document.uri,
                        &params.content_changes,
                    );
                }
            }
            "exit" => self.shutdown = true,
            _ => {}
        }
        Ok(())
    }

    fn dispatch(&mut self, method: &str, params: &Value) -> anyhow::Result<Value> {
        REQUEST_HANDLERS
            .iter()
            .find_map(|(name, handler)| (*name == method).then_some(*handler))
            .ok_or_else(|| anyhow::anyhow!("Method not found: {method}"))?(self, params)
    }

    fn dispatch_initialize(&mut self, params: &Value) -> anyhow::Result<Value> {
        let params: InitializeParams = serde_json::from_value(params.clone())?;
        let mut backend = LspBackend::new(canonicalize_workspace_root(resolve_root(&params)));
        if let Some(ref opts) = params.initialization_options {
            backend.apply_settings(crate::settings::AsgrepSettings::from_initialization_options(opts));
        }
        backend.start_background_index();
        let result = backend.initialize_result();
        self.backend = Some(backend);
        Ok(result)
    }

    fn dispatch_shutdown(&mut self, _: &Value) -> anyhow::Result<Value> {
        self.shutdown = true;
        Ok(Value::Null)
    }

    fn dispatch_workspace_symbol(&mut self, params: &Value) -> anyhow::Result<Value> {
        let params: WorkspaceSymbolParams = serde_json::from_value(params.clone())?;
        self.backend()?.workspace_symbols(&params.query)
    }

    fn dispatch_search(&mut self, params: &Value) -> anyhow::Result<Value> {
        let params: SearchParams = serde_json::from_value(params.clone())?;
        self.backend()?.search(&params.query, params.semantic, params.limit.clamp(1, 500))
    }

    fn dispatch_document_symbol(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.backend()?.document_symbols(&serde_json::from_value::<DocumentSymbolParams>(params.clone())?)
    }

    fn dispatch_definition(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.backend()?.goto_definition(&serde_json::from_value(params.clone())?)
    }

    fn dispatch_references(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.backend()?.find_references(&serde_json::from_value::<ReferenceParams>(params.clone())?)
    }

    fn dispatch_prepare_call_hierarchy(&mut self, params: &Value) -> anyhow::Result<Value> {
        let params: CallHierarchyPrepareParams = serde_json::from_value(params.clone())?;
        self.backend()?.prepare_call_hierarchy(&TextDocumentPositionParams {
            text_document: params.text_document,
            position: params.position,
        })
    }

    fn dispatch_incoming_calls(&mut self, params: &Value) -> anyhow::Result<Value> {
        let params: CallHierarchyItemParams = serde_json::from_value(params.clone())?;
        self.backend()?.incoming_calls(&params.item)
    }

    fn dispatch_outgoing_calls(&mut self, params: &Value) -> anyhow::Result<Value> {
        let params: CallHierarchyItemParams = serde_json::from_value(params.clone())?;
        self.backend()?.outgoing_calls(&params.item)
    }

    fn dispatch_execute_command(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.backend()?.execute_command(&serde_json::from_value::<ExecuteCommandParams>(params.clone())?)
    }

    fn backend(&self) -> anyhow::Result<&LspBackend> {
        self.backend.as_ref().ok_or_else(|| anyhow::anyhow!("server not initialized"))
    }
}
fn resolve_root(params: &InitializeParams) -> std::path::PathBuf {
    if let Some(folders) = &params.workspace_folders {
        if let Some(first) = folders.first() {
            if let Ok(p) = file_uri_to_path(&first.uri) { return p; }
        }
    }
    if let Some(uri) = &params.root_uri {
        if let Ok(p) = file_uri_to_path(uri) { return p; }
    }
    if let Some(path) = &params.root_path { return std::path::PathBuf::from(path); }
    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
}
pub fn log(msg: &str) {
    let _ = writeln!(io::stderr(), "[asgrep-lsp] {msg}");
}

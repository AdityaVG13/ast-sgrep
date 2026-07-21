use crate::backend::LspBackend;
use crate::support::{canonicalize_workspace_root, file_uri_to_path};
use crate::support::{read_message, send_error, send_response};
use crate::types::{
    CallHierarchyItemParams, CallHierarchyPrepareParams, DocumentSymbolParams,
    ExecuteCommandParams, InitializeParams, NotificationMessage, ReferenceParams, RequestMessage,
    SearchParams, TextDocumentPositionParams, WorkspaceSymbolParams,
};
use serde_json::Value;
use std::io::{self, BufReader, Write};
pub struct LspServer {
    backend: Option<LspBackend>,
    shutdown: bool,
}
type ReqH = fn(&mut LspServer, &Value) -> anyhow::Result<Value>;
const HANDLERS: &[(&str, ReqH)] = &[
    ("initialize", LspServer::h_init),
    ("shutdown", LspServer::h_shutdown),
    ("workspace/symbol", LspServer::h_wsym),
    ("asgrep/search", LspServer::h_search),
    ("textDocument/documentSymbol", LspServer::h_dsym),
    ("textDocument/definition", LspServer::h_def),
    ("textDocument/references", LspServer::h_refs),
    ("callHierarchy/prepareCallHierarchy", LspServer::h_prep_ch),
    ("callHierarchy/incomingCalls", LspServer::h_in_calls),
    ("callHierarchy/outgoingCalls", LspServer::h_out_calls),
    ("workspace/executeCommand", LspServer::h_exec),
];
impl Default for LspServer {
    fn default() -> Self {
        Self::new()
    }
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
        match self.dispatch(&req.method, &req.params) {
            Ok(v) => send_response(stdout, &req.id, v)?,
            Err(e) => {
                let code = if e.to_string().contains("not found") {
                    -32601
                } else {
                    -32603
                };
                send_error(stdout, &req.id, code, &e.to_string())?;
            }
        }
        Ok(())
    }
    fn handle_notification(&mut self, notif: NotificationMessage) -> io::Result<()> {
        match notif.method.as_str() {
            "initialized" => {}
            "textDocument/didOpen" => {
                if let (Some(b), Ok(p)) = (
                    &self.backend,
                    serde_json::from_value::<crate::types::DidOpenTextDocumentParams>(notif.params),
                ) {
                    if let Ok(rel) = crate::support::uri_to_rel_path(&p.text_document.uri, b.root())
                    {
                        let _ = b.index_content(&rel, &p.text_document.text);
                    }
                }
            }
            "textDocument/didSave" => {
                if let (Some(b), Ok(p)) = (
                    &self.backend,
                    serde_json::from_value::<crate::types::DidSaveTextDocumentParams>(notif.params),
                ) {
                    if let Ok(rel) = crate::support::uri_to_rel_path(&p.text_document.uri, b.root())
                    {
                        let _ = b.reindex_file(&rel);
                    }
                }
            }
            "textDocument/didChange" => {
                if let (Some(b), Ok(p)) = (
                    &self.backend,
                    serde_json::from_value::<crate::types::DidChangeTextDocumentParams>(
                        notif.params,
                    ),
                ) {
                    let _ = b.apply_document_changes(&p.text_document.uri, &p.content_changes);
                }
            }
            "exit" => self.shutdown = true,
            _ => {}
        }
        Ok(())
    }
    fn dispatch(&mut self, method: &str, params: &Value) -> anyhow::Result<Value> {
        HANDLERS
            .iter()
            .find_map(|(n, h)| (*n == method).then_some(*h))
            .ok_or_else(|| anyhow::anyhow!("Method not found: {method}"))?(self, params)
    }
    fn h_init(&mut self, params: &Value) -> anyhow::Result<Value> {
        let params: InitializeParams = serde_json::from_value(params.clone())?;
        let mut backend = LspBackend::new(canonicalize_workspace_root(resolve_root(&params)));
        if let Some(ref opts) = params.initialization_options {
            backend.apply_settings(crate::support::AsgrepSettings::from_initialization_options(
                opts,
            ));
        }
        backend.start_background_index();
        let result = backend.initialize_result();
        self.backend = Some(backend);
        Ok(result)
    }
    fn h_shutdown(&mut self, _: &Value) -> anyhow::Result<Value> {
        self.shutdown = true;
        Ok(Value::Null)
    }
    fn h_wsym(&mut self, params: &Value) -> anyhow::Result<Value> {
        let p: WorkspaceSymbolParams = serde_json::from_value(params.clone())?;
        self.backend()?.workspace_symbols(&p.query)
    }
    fn h_search(&mut self, params: &Value) -> anyhow::Result<Value> {
        let p: SearchParams = serde_json::from_value(params.clone())?;
        self.backend()?
            .search(&p.query, p.semantic, p.limit.clamp(1, 500))
    }
    fn h_dsym(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.backend()?
            .document_symbols(&serde_json::from_value::<DocumentSymbolParams>(
                params.clone(),
            )?)
    }
    fn h_def(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.backend()?
            .goto_definition(&serde_json::from_value(params.clone())?)
    }
    fn h_refs(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.backend()?
            .find_references(&serde_json::from_value::<ReferenceParams>(params.clone())?)
    }
    fn h_prep_ch(&mut self, params: &Value) -> anyhow::Result<Value> {
        let p: CallHierarchyPrepareParams = serde_json::from_value(params.clone())?;
        self.backend()?
            .prepare_call_hierarchy(&TextDocumentPositionParams {
                text_document: p.text_document,
                position: p.position,
            })
    }
    fn h_in_calls(&mut self, params: &Value) -> anyhow::Result<Value> {
        let p: CallHierarchyItemParams = serde_json::from_value(params.clone())?;
        self.backend()?.incoming_calls(&p.item)
    }
    fn h_out_calls(&mut self, params: &Value) -> anyhow::Result<Value> {
        let p: CallHierarchyItemParams = serde_json::from_value(params.clone())?;
        self.backend()?.outgoing_calls(&p.item)
    }
    fn h_exec(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.backend()?
            .execute_command(&serde_json::from_value::<ExecuteCommandParams>(
                params.clone(),
            )?)
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
            if let Ok(p) = file_uri_to_path(&first.uri) {
                return p;
            }
        }
    }
    if let Some(uri) = &params.root_uri {
        if let Ok(p) = file_uri_to_path(uri) {
            return p;
        }
    }
    if let Some(path) = &params.root_path {
        return std::path::PathBuf::from(path);
    }
    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
}
pub fn log(msg: &str) {
    let _ = writeln!(io::stderr(), "[asgrep-lsp] {msg}");
}

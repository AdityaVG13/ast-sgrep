use crate::backend::LspBackend;
use crate::support::{
    canonicalize_workspace_root, file_uri_to_path, read_message, send_error, send_response,
    uri_to_rel_path, write_message, AsgrepSettings,
};
use crate::types::{
    CallHierarchyItemParams, CallHierarchyPrepareParams, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    DocumentSymbolParams, ExecuteCommandParams, InitializeParams, NotificationMessage,
    ReferenceParams, RequestMessage, SearchParams, TextDocumentPositionParams,
    WorkspaceSymbolParams,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::io::{self, BufReader, Write};
use std::path::PathBuf;

pub struct LspServer {
    backend: Option<LspBackend>,
    /// Set after a successful `shutdown` request. Further requests get
    /// InvalidRequest until `exit` (LSP lifecycle / d2a1.14).
    shutdown_received: bool,
    /// Set by the `exit` notification; leaves the message loop.
    exit_requested: bool,
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
            shutdown_received: false,
            exit_requested: false,
        }
    }

    /// Process exit code after `run` returns: 0 if `shutdown` was seen before
    /// `exit` (or clean EOF), 1 if `exit` arrived without a prior `shutdown`.
    pub fn process_exit_code(&self) -> i32 {
        if self.exit_requested && !self.shutdown_received {
            1
        } else {
            0
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        let stdin = io::stdin();
        let mut stdout = io::stdout();
        let mut reader = BufReader::new(stdin.lock());
        self.run_with(&mut reader, &mut stdout)
    }

    /// Drive the LSP loop over arbitrary readers (stdio or tests).
    pub fn run_with(
        &mut self,
        reader: &mut impl io::BufRead,
        stdout: &mut impl Write,
    ) -> io::Result<()> {
        while let Some(body) = read_message(reader)? {
            if let Ok(req) = serde_json::from_str::<RequestMessage>(&body) {
                self.handle_request(stdout, req)?;
            } else if let Ok(notif) = serde_json::from_str::<NotificationMessage>(&body) {
                self.handle_notification(stdout, notif)?;
                if self.exit_requested {
                    break;
                }
            }
        }
        Ok(())
    }

    fn handle_request(&mut self, stdout: &mut impl Write, req: RequestMessage) -> io::Result<()> {
        // After shutdown, only further messages should be exit (notification).
        // Any request is InvalidRequest per LSP.
        if self.shutdown_received {
            send_error(
                stdout,
                &req.id,
                -32600,
                "server is shutting down; send exit notification",
            )?;
            return Ok(());
        }
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

    fn handle_notification(
        &mut self,
        stdout: &mut impl Write,
        notif: NotificationMessage,
    ) -> io::Result<()> {
        match notif.method.as_str() {
            "initialized" => {}
            "textDocument/didOpen" => {
                self.sync_rel_path(stdout, "didOpen", notif.params, |b, p: DidOpenTextDocumentParams| {
                    let rel = uri_to_rel_path(&p.text_document.uri, b.root())?;
                    b.index_content(&rel, &p.text_document.text)
                })?;
            }
            "textDocument/didSave" => {
                self.sync_rel_path(stdout, "didSave", notif.params, |b, p: DidSaveTextDocumentParams| {
                    let rel = uri_to_rel_path(&p.text_document.uri, b.root())?;
                    b.reindex_file(&rel)
                })?;
            }
            "textDocument/didChange" => {
                self.sync_rel_path(stdout, "didChange", notif.params, |b, p: DidChangeTextDocumentParams| {
                    b.apply_document_changes(&p.text_document.uri, &p.content_changes)
                })?;
            }
            "textDocument/didClose" => {
                self.sync_rel_path(stdout, "didClose", notif.params, |b, p: DidCloseTextDocumentParams| {
                    b.close_document(&p.text_document.uri)
                })?;
            }
            "exit" => self.exit_requested = true,
            _ => {}
        }
        Ok(())
    }

    /// Parse a sync notification and surface index errors via `window/showMessage`.
    fn sync_rel_path<P, F>(
        &self,
        stdout: &mut impl Write,
        surface: &str,
        params: Value,
        f: F,
    ) -> io::Result<()>
    where
        P: DeserializeOwned,
        F: FnOnce(&LspBackend, P) -> anyhow::Result<()>,
    {
        let Some(backend) = self.backend.as_ref() else {
            return Ok(());
        };
        let Ok(parsed) = serde_json::from_value::<P>(params) else {
            return Ok(());
        };
        if let Err(e) = f(backend, parsed) {
            show_index_error(stdout, surface, &e)?;
        }
        Ok(())
    }

    fn dispatch(&mut self, method: &str, params: &Value) -> anyhow::Result<Value> {
        HANDLERS
            .iter()
            .find_map(|(n, h)| (*n == method).then_some(*h))
            .ok_or_else(|| anyhow::anyhow!("Method not found: {method}"))?(self, params)
    }

    fn with_parsed<P, F>(&self, params: &Value, f: F) -> anyhow::Result<Value>
    where
        P: DeserializeOwned,
        F: FnOnce(&LspBackend, P) -> anyhow::Result<Value>,
    {
        f(self.backend()?, serde_json::from_value(params.clone())?)
    }

    fn h_init(&mut self, params: &Value) -> anyhow::Result<Value> {
        let params: InitializeParams = serde_json::from_value(params.clone())?;
        let mut backend = LspBackend::new(canonicalize_workspace_root(resolve_root(&params)));
        if let Some(ref opts) = params.initialization_options {
            backend.apply_settings(AsgrepSettings::from_initialization_options(opts));
        }
        backend.start_background_index();
        let result = backend.initialize_result();
        self.backend = Some(backend);
        Ok(result)
    }

    fn h_shutdown(&mut self, _: &Value) -> anyhow::Result<Value> {
        self.shutdown_received = true;
        Ok(Value::Null)
    }

    fn h_wsym(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.with_parsed(params, |b, p: WorkspaceSymbolParams| {
            b.workspace_symbols(&p.query)
        })
    }

    fn h_search(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.with_parsed(params, |b, p: SearchParams| {
            b.search(&p.query, p.semantic, clamp_lsp_search_limit(p.limit))
        })
    }

    fn h_dsym(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.with_parsed(params, |b, p: DocumentSymbolParams| b.document_symbols(&p))
    }

    fn h_def(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.with_parsed(params, |b, p: TextDocumentPositionParams| {
            b.goto_definition(&p)
        })
    }

    fn h_refs(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.with_parsed(params, |b, p: ReferenceParams| b.find_references(&p))
    }

    fn h_prep_ch(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.with_parsed(params, |b, p: CallHierarchyPrepareParams| {
            b.prepare_call_hierarchy(&p)
        })
    }

    fn h_in_calls(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.with_parsed(params, |b, p: CallHierarchyItemParams| {
            b.incoming_calls(&p.item)
        })
    }

    fn h_out_calls(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.with_parsed(params, |b, p: CallHierarchyItemParams| {
            b.outgoing_calls(&p.item)
        })
    }

    fn h_exec(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.with_parsed(params, |b, p: ExecuteCommandParams| b.execute_command(&p))
    }

    fn backend(&self) -> anyhow::Result<&LspBackend> {
        self.backend
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("server not initialized"))
    }
}

fn resolve_root(params: &InitializeParams) -> PathBuf {
    params
        .workspace_folders
        .as_ref()
        .and_then(|folders| folders.first())
        .and_then(|folder| file_uri_to_path(&folder.uri).ok())
        .or_else(|| {
            params
                .root_uri
                .as_ref()
                .and_then(|uri| file_uri_to_path(uri).ok())
        })
        .or_else(|| params.root_path.as_ref().map(PathBuf::from))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// ei0i-style clamp for LSP `asgrep/search`: remap 0→default, hard-cap at 1000.
fn clamp_lsp_search_limit(limit: usize) -> usize {
    const MAX_OUTPUT_RESULTS: usize = 1000;
    let default = ast_sgrep_core::SearchOptions::default_limit();
    let base = if limit == 0 { default.max(1) } else { limit };
    base.clamp(1, MAX_OUTPUT_RESULTS)
}

#[cfg(test)]
mod limit_tests {
    use super::clamp_lsp_search_limit;

    #[test]
    fn remaps_zero_and_caps_ceiling() {
        let def = ast_sgrep_core::SearchOptions::default_limit().max(1);
        assert_eq!(clamp_lsp_search_limit(0), def.min(1000));
        assert_eq!(clamp_lsp_search_limit(32), 32);
        assert_eq!(clamp_lsp_search_limit(500), 500);
        assert_eq!(clamp_lsp_search_limit(10_000), 1000);
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::LspServer;
    use crate::support::read_message;
    use std::io::Cursor;

    fn frame(body: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
    }

    fn drain_messages(stdout: &[u8]) -> Vec<serde_json::Value> {
        let mut reader = std::io::BufReader::new(Cursor::new(stdout));
        let mut out = Vec::new();
        while let Some(body) = read_message(&mut reader).expect("frame") {
            out.push(serde_json::from_str(&body).expect("json"));
        }
        out
    }

    #[test]
    fn exit_without_shutdown_leaves_loop_with_code_1() {
        let mut server = LspServer::new();
        let input = frame(r#"{"jsonrpc":"2.0","method":"exit"}"#);
        let mut reader = Cursor::new(input);
        let mut stdout = Vec::new();
        server.run_with(&mut reader, &mut stdout).unwrap();
        assert!(server.exit_requested);
        assert!(!server.shutdown_received);
        assert_eq!(server.process_exit_code(), 1);
        assert!(stdout.is_empty(), "exit is a notification");
    }

    #[test]
    fn shutdown_stays_up_until_exit_and_rejects_later_requests() {
        let mut server = LspServer::new();
        let mut input = Vec::new();
        input.extend(frame(
            r#"{"jsonrpc":"2.0","id":1,"method":"shutdown","params":{}}"#,
        ));
        input.extend(frame(
            r#"{"jsonrpc":"2.0","id":2,"method":"workspace/symbol","params":{"query":"x"}}"#,
        ));
        input.extend(frame(r#"{"jsonrpc":"2.0","method":"exit"}"#));
        let mut reader = Cursor::new(input);
        let mut stdout = Vec::new();
        server.run_with(&mut reader, &mut stdout).unwrap();
        assert!(server.shutdown_received);
        assert!(server.exit_requested);
        assert_eq!(server.process_exit_code(), 0);
        let messages = drain_messages(&stdout);
        assert_eq!(messages.len(), 2, "{messages:?}");
        assert_eq!(messages[0]["id"], 1);
        assert!(messages[0]["result"].is_null());
        assert_eq!(messages[1]["id"], 2);
        assert_eq!(messages[1]["error"]["code"], -32600);
    }
}

fn show_index_error(stdout: &mut impl Write, surface: &str, err: &anyhow::Error) -> io::Result<()> {
    let message = format!("asgrep index ({surface}): {err}");
    log(&message);
    // Notifications have no JSON-RPC response; surface via window/showMessage
    // so clients see index failures instead of silent Ok (ast-sgrep-x46g).
    write_message(
        stdout,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "method": "window/showMessage",
            "params": { "type": 1, "message": message }
        })
        .to_string(),
    )
}

pub fn log(msg: &str) {
    let _ = writeln!(io::stderr(), "[asgrep-lsp] {msg}");
}

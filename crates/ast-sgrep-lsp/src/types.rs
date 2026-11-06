use serde::{Deserialize, Serialize};
use serde_json::Value;
#[derive(Debug, Deserialize)]
pub struct RequestMessage {
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}
#[derive(Debug, Deserialize)]
pub struct NotificationMessage {
    pub method: String,
    #[serde(default)]
    pub params: Value,
}
#[derive(Debug, Deserialize)]
pub struct InitializeParams {
    #[serde(rename = "rootUri")]
    pub root_uri: Option<String>,
    #[serde(rename = "rootPath")]
    pub root_path: Option<String>,
    #[serde(rename = "workspaceFolders")]
    pub workspace_folders: Option<Vec<WorkspaceFolder>>,
    #[serde(rename = "initializationOptions")]
    pub initialization_options: Option<Value>,
}
#[derive(Debug, Deserialize)]
pub struct WorkspaceFolder {
    pub uri: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct TextDocumentPositionParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TextDocumentIdentifier {
    pub uri: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}
#[derive(Debug, Deserialize)]
pub struct WorkspaceSymbolParams {
    pub query: String,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchParams {
    pub query: String,
    #[serde(default)]
    pub semantic: bool,
    #[serde(default = "default_search_limit")]
    pub limit: usize,
}
fn default_search_limit() -> usize {
    ast_sgrep_core::SearchOptions::default_limit()
}
#[derive(Debug, Deserialize)]
pub struct TextDocumentParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentIdentifier,
}
pub type DocumentSymbolParams = TextDocumentParams;
pub type DidSaveTextDocumentParams = TextDocumentParams;
pub type DidCloseTextDocumentParams = TextDocumentParams;
pub type CallHierarchyPrepareParams = TextDocumentPositionParams;
#[derive(Debug, Deserialize)]
pub struct ReferenceParams {
    #[serde(flatten)]
    pub at: TextDocumentPositionParams,
    #[serde(default)]
    pub context: Option<ReferenceContext>,
}
#[derive(Debug, Deserialize)]
pub struct ReferenceContext {
    #[serde(rename = "includeDeclaration")]
    pub include_declaration: bool,
}
#[derive(Debug, Deserialize)]
pub struct CallHierarchyItemParams {
    pub item: CallHierarchyItem,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CallHierarchyItem {
    pub name: String,
    pub kind: u32,
    pub uri: String,
    pub range: Range,
    #[serde(rename = "selectionRange")]
    pub selection_range: Range,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}
#[derive(Debug, Deserialize)]
pub struct ExecuteCommandParams {
    pub command: String,
    #[serde(default)]
    pub arguments: Vec<Value>,
}
#[derive(Debug, Deserialize)]
pub struct DidChangeTextDocumentParams {
    #[serde(rename = "textDocument")]
    pub text_document: VersionedTextDocumentIdentifier,
    #[serde(rename = "contentChanges")]
    pub content_changes: Vec<TextDocumentContentChangeEvent>,
}
#[derive(Debug, Deserialize)]
pub struct VersionedTextDocumentIdentifier {
    pub uri: String,
    #[serde(default)]
    pub version: Option<i32>,
}
#[derive(Debug, Deserialize)]
pub struct TextDocumentContentChangeEvent {
    #[serde(default)]
    pub range: Option<Range>,
    #[serde(rename = "rangeLength", default)]
    pub range_length: Option<u32>,
    pub text: String,
}
#[derive(Debug, Deserialize)]
pub struct DidOpenTextDocumentParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentItem,
}
#[derive(Debug, Deserialize)]
pub struct TextDocumentItem {
    pub uri: String,
    #[serde(rename = "languageId", default)]
    pub language_id: Option<String>,
    pub version: i32,
    pub text: String,
}
pub const SYMBOL_KIND_FUNCTION: u32 = 12;
pub const SYMBOL_KIND_METHOD: u32 = 6;
pub const SYMBOL_KIND_STRING: u32 = 15;
pub const SYMBOL_KIND_CLASS: u32 = 5;
pub const SYMBOL_KIND_INTERFACE: u32 = 11;
pub const SYMBOL_KIND_ENUM: u32 = 10;
pub const SYMBOL_KIND_STRUCT: u32 = 23;

/// Single-sourced LSP wire strings: request/notification method names, and
/// `workspace/executeCommand` command ids. The `HANDLERS` dispatch table
/// (`server.rs`), the `handle_notification` match, `initialize_result`
/// capabilities, and the `execute_command` match must all draw from here —
/// add a method/command once, not at each site.
pub const METHOD_INITIALIZE: &str = "initialize";
pub const METHOD_SHUTDOWN: &str = "shutdown";
pub const METHOD_WORKSPACE_SYMBOL: &str = "workspace/symbol";
pub const METHOD_ASGREP_SEARCH: &str = "asgrep/search";
pub const METHOD_DOCUMENT_SYMBOL: &str = "textDocument/documentSymbol";
pub const METHOD_DEFINITION: &str = "textDocument/definition";
pub const METHOD_REFERENCES: &str = "textDocument/references";
pub const METHOD_PREPARE_CALL_HIERARCHY: &str = "textDocument/prepareCallHierarchy";
pub const METHOD_INCOMING_CALLS: &str = "callHierarchy/incomingCalls";
pub const METHOD_OUTGOING_CALLS: &str = "callHierarchy/outgoingCalls";
pub const METHOD_EXECUTE_COMMAND: &str = "workspace/executeCommand";
pub const METHOD_INITIALIZED: &str = "initialized";
pub const METHOD_DID_OPEN: &str = "textDocument/didOpen";
pub const METHOD_DID_SAVE: &str = "textDocument/didSave";
pub const METHOD_DID_CHANGE: &str = "textDocument/didChange";
pub const METHOD_DID_CLOSE: &str = "textDocument/didClose";
pub const METHOD_EXIT: &str = "exit";

pub const COMMAND_SEARCH: &str = "asgrep.search";
pub const COMMAND_SEARCH_SEMANTIC: &str = "asgrep.search.semantic";
pub const COMMAND_REINDEX: &str = "asgrep.reindex";
pub const COMMAND_CALLERS: &str = "asgrep.callers";
pub const COMMAND_DEFS: &str = "asgrep.defs";

/// Single-sourced search-result limits: per-call-site budgets plus the
/// `asgrep/search` hard cap (`clamp_lsp_search_limit`).
pub const WORKSPACE_SYMBOL_LIMIT: usize = 50;
pub const GOTO_DEFINITION_LIMIT: usize = 16;
pub const FIND_REFERENCES_LIMIT: usize = 128;
pub const EXECUTE_COMMAND_SEARCH_LIMIT: usize = 32;
pub const MAX_SEARCH_RESULTS: usize = 1000;

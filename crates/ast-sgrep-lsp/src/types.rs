//! LSP protocol types (serde).

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
pub struct DocumentSymbolParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentIdentifier,
}

#[derive(Debug, Deserialize)]
pub struct ReferenceParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
    #[serde(default)]
    pub context: Option<ReferenceContext>,
}

#[derive(Debug, Deserialize)]
pub struct ReferenceContext {
    #[serde(rename = "includeDeclaration")]
    pub include_declaration: bool,
}

#[derive(Debug, Deserialize)]
pub struct CallHierarchyPrepareParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentIdentifier,
    pub position: Position,
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
pub struct DidSaveTextDocumentParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentIdentifier,
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

/// LSP SymbolKind for functions.
pub const SYMBOL_KIND_FUNCTION: u32 = 12;
/// LSP SymbolKind for methods.
pub const SYMBOL_KIND_METHOD: u32 = 6;
/// LSP SymbolKind::String — used for semantic similarity hits.
pub const SYMBOL_KIND_STRING: u32 = 15;

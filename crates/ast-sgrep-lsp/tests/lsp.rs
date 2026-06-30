//! Integration tests for full LSP backend.

use std::path::PathBuf;

use ast_sgrep_lsp::{backend::path_to_uri, LspBackend};
use ast_sgrep_lsp::types::{
    CallHierarchyItem, DocumentSymbolParams, ExecuteCommandParams, Position, Range,
    TextDocumentIdentifier, TextDocumentPositionParams,
};

fn fixture_backend() -> LspBackend {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/sample");
    let backend = LspBackend::new(root.canonicalize().unwrap());
    backend.ensure_index().unwrap();
    backend
}

#[test]
fn initialize_capabilities_include_call_hierarchy() {
    let backend = LspBackend::new(PathBuf::from("."));
    let caps = backend.initialize_result();
    assert!(caps["capabilities"]["callHierarchyProvider"].as_bool() == Some(true));
    assert!(caps["capabilities"]["documentSymbolProvider"].as_bool() == Some(true));
}

#[test]
fn workspace_symbols_finds_process_request() {
    let backend = fixture_backend();
    let result = backend.workspace_symbols("process_request").unwrap();
    let arr = result.as_array().unwrap();
    assert!(!arr.is_empty());
}

#[test]
fn document_symbols_lists_functions() {
    let backend = fixture_backend();
    let params = DocumentSymbolParams {
        text_document: TextDocumentIdentifier {
            uri: path_to_uri(&backend.root().join("src/main.rs")),
        },
    };
    let result = backend.document_symbols(&params).unwrap();
    let arr = result.as_array().unwrap();
    assert!(!arr.is_empty());
    assert!(arr.iter().any(|s| s["name"].as_str().unwrap_or("").contains("process")));
}

#[test]
fn execute_command_search() {
    let backend = fixture_backend();
    let params = ExecuteCommandParams {
        command: "asgrep.search".to_string(),
        arguments: vec![serde_json::json!("auth_refresh")],
    };
    let result = backend.execute_command(&params).unwrap();
    assert!(result["hits"].is_array());
}

#[test]
fn did_change_indexes_unsaved_buffer() {
    let backend = fixture_backend();
    let uri = path_to_uri(&backend.root().join("src/main.rs"));
    let changes = vec![ast_sgrep_lsp::types::TextDocumentContentChangeEvent {
        range: None,
        range_length: None,
        text: "fn main() {\n    process_request(\"edited\");\n}\n\nfn process_request(input: &str) {}\n".to_string(),
    }];
    backend.apply_document_changes(&uri, &changes).unwrap();
    let params = ExecuteCommandParams {
        command: "asgrep.search".to_string(),
        arguments: vec![serde_json::json!("edited")],
    };
    let result = backend.execute_command(&params).unwrap();
    let hits = result["hits"].as_array().unwrap();
    assert!(hits.iter().any(|h| h["excerpt"].as_str().unwrap_or("").contains("edited")));
}

#[test]
fn goto_definition_for_symbol() {
    let backend = fixture_backend();
    let params = TextDocumentPositionParams {
        text_document: TextDocumentIdentifier {
            uri: path_to_uri(&backend.root().join("src/main.rs")),
        },
        position: Position { line: 5, character: 4 },
    };
    let result = backend.goto_definition(&params).unwrap();
    assert!(!result.is_null());
}

#[test]
fn call_hierarchy_incoming_and_outgoing() {
    let backend = fixture_backend();
    let item = CallHierarchyItem {
        name: "process_request".to_string(),
        kind: 12,
        uri: path_to_uri(&backend.root().join("src/main.rs")),
        range: Range {
            start: Position { line: 5, character: 0 },
            end: Position { line: 8, character: 0 },
        },
        selection_range: Range {
            start: Position { line: 5, character: 0 },
            end: Position { line: 5, character: 0 },
        },
        detail: None,
    };
    let incoming = backend.incoming_calls(&item).unwrap();
    let outgoing = backend.outgoing_calls(&item).unwrap();
    assert!(incoming.as_array().map(|a| !a.is_empty()).unwrap_or(false));
    assert!(outgoing.as_array().map(|a| !a.is_empty()).unwrap_or(false));
}

//! Integration tests for full LSP backend.

use std::path::PathBuf;

use ast_sgrep_lsp::{backend::path_to_uri, LspBackend};
use ast_sgrep_lsp::types::{
    CallHierarchyItem, DocumentSymbolParams, ExecuteCommandParams, Position, Range,
    TextDocumentIdentifier, TextDocumentPositionParams,
};
use ast_sgrep_core::{IndexOptions, Indexer};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/sample")
        .canonicalize()
        .expect("fixture")
}

fn fixture_backend() -> (tempfile::TempDir, LspBackend) {
    let root = fixture_root();
    let temp = tempfile::TempDir::new().unwrap();
    let index_path = temp.path().join("index.db");

    let mut indexer = Indexer::new(IndexOptions {
        root: root.clone(),
        index_path: Some(index_path.clone()),
        force_reindex: true,
        ..IndexOptions::default()
    })
    .unwrap();
    indexer.index_all().unwrap();

    let mut backend = LspBackend::new(root);
    backend.set_index_path(index_path);
    backend.ensure_index().unwrap();
    (temp, backend)
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
    let (_temp, backend) = fixture_backend();
    let result = backend.workspace_symbols("process_request").unwrap();
    let arr = result.as_array().unwrap();
    assert!(!arr.is_empty());
}

#[test]
fn document_symbols_lists_functions() {
    let (_temp, backend) = fixture_backend();
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
fn execute_command_semantic_search_finds_synonym() {
    let (_temp, backend) = fixture_backend();
    let params = ExecuteCommandParams {
        command: "asgrep.search.semantic".to_string(),
        arguments: vec![serde_json::json!("credential renewal")],
    };
    let result = backend.execute_command(&params).unwrap();
    let hits = result["hits"].as_array().unwrap();
    assert!(!hits.is_empty());
    assert!(
        hits.iter().any(|h| {
            h["kind"].as_str() == Some("embed")
                && (h["symbol"].as_str() == Some("auth_refresh")
                    || h["excerpt"].as_str().unwrap_or("").contains("auth_refresh"))
        }),
        "semantic command should return embed hits for synonym query"
    );
}

#[test]
fn workspace_symbols_includes_semantic_metadata() {
    let (_temp, backend) = fixture_backend();
    let result = backend.workspace_symbols("credential renewal").unwrap();
    let arr = result.as_array().unwrap();
    assert!(!arr.is_empty());
    let has_semantic = arr.iter().any(|s| {
        s["data"]["semantic"].as_bool() == Some(true)
            || s["detail"].as_str().unwrap_or("").contains("semantic")
    });
    assert!(has_semantic, "workspace symbols should surface semantic hit metadata");
}

#[test]
fn execute_command_search() {
    let (_temp, backend) = fixture_backend();
    let params = ExecuteCommandParams {
        command: "asgrep.search".to_string(),
        arguments: vec![serde_json::json!("auth_refresh")],
    };
    let result = backend.execute_command(&params).unwrap();
    assert!(result["hits"].is_array());
}

#[test]
fn did_change_indexes_unsaved_buffer() {
    let (_temp, backend) = fixture_backend();
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
    let (_temp, backend) = fixture_backend();
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
    let (_temp, backend) = fixture_backend();
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

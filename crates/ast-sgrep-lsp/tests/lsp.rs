//! LSP integration tests.

use ast_sgrep_lsp::backend::path_to_uri;
use ast_sgrep_lsp::types::{
    CallHierarchyItem, DocumentSymbolParams, ExecuteCommandParams, Position, Range,
    TextDocumentIdentifier, TextDocumentPositionParams,
};
use ast_sgrep_testkit::sample_backend;

#[test]
fn initialize_capabilities_include_call_hierarchy() {
    let backend = ast_sgrep_lsp::LspBackend::new(std::path::PathBuf::from("."));
    let caps = backend.initialize_result();
    assert_eq!(caps["capabilities"]["callHierarchyProvider"].as_bool(), Some(true));
    assert_eq!(caps["capabilities"]["documentSymbolProvider"].as_bool(), Some(true));
}

#[test]
fn workspace_symbols_finds_process_request() {
    let (_indexed, backend) = sample_backend();
    let result = backend.workspace_symbols("process_request").unwrap();
    assert!(!result.as_array().unwrap().is_empty());
}

#[test]
fn document_symbols_lists_functions() {
    let (_indexed, backend) = sample_backend();
    let params = DocumentSymbolParams {
        text_document: TextDocumentIdentifier {
            uri: path_to_uri(&backend.root().join("src/main.rs")),
        },
    };
    let arr = backend.document_symbols(&params).unwrap();
    let arr = arr.as_array().unwrap();
    assert!(arr.iter().any(|s| s["name"].as_str().unwrap_or("").contains("process")));
}

#[test]
fn execute_command_semantic_search_finds_synonym() {
    let (_indexed, backend) = sample_backend();
    let params = ExecuteCommandParams {
        command: "asgrep.search.semantic".to_string(),
        arguments: vec![serde_json::json!("credential renewal")],
    };
    let result = backend.execute_command(&params).unwrap();
    let hits = result["hits"].as_array().unwrap();
    assert!(hits.iter().any(|h| {
        h["kind"].as_str() == Some("embed")
            && (h["symbol"].as_str() == Some("auth_refresh")
                || h["excerpt"].as_str().unwrap_or("").contains("auth_refresh"))
    }));
}

#[test]
fn workspace_symbols_include_semantic_metadata() {
    let (_indexed, backend) = sample_backend();
    let arr = backend.workspace_symbols("credential renewal").unwrap();
    let arr = arr.as_array().unwrap();
    assert!(arr.iter().any(|s| {
        s["data"]["semantic"].as_bool() == Some(true)
            || s["detail"].as_str().unwrap_or("").contains("semantic")
    }));
}

#[test]
fn execute_command_hybrid_search() {
    let (_indexed, backend) = sample_backend();
    let params = ExecuteCommandParams {
        command: "asgrep.search".to_string(),
        arguments: vec![serde_json::json!("auth_refresh")],
    };
    assert!(backend.execute_command(&params).unwrap()["hits"].is_array());
}

#[test]
fn did_change_indexes_unsaved_buffer() {
    let (_indexed, backend) = sample_backend();
    let uri = path_to_uri(&backend.root().join("src/main.rs"));
    let changes = vec![ast_sgrep_lsp::types::TextDocumentContentChangeEvent {
        range: None,
        range_length: None,
        text: "fn main() {\n    process_request(\"edited\");\n}\n\nfn process_request(input: &str) {}\n"
            .to_string(),
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
fn did_change_applies_incremental_range_edit() {
    let (_indexed, backend) = sample_backend();
    let uri = path_to_uri(&backend.root().join("src/main.rs"));
    let changes = vec![ast_sgrep_lsp::types::TextDocumentContentChangeEvent {
        range: Some(Range {
            start: Position { line: 1, character: 4 },
            end: Position { line: 1, character: 19 },
        }),
        range_length: Some(15),
        text: "process_request(\"range-edited\")".to_string(),
    }];
    backend.apply_document_changes(&uri, &changes).unwrap();
    let params = ExecuteCommandParams {
        command: "asgrep.search".to_string(),
        arguments: vec![serde_json::json!("range-edited")],
    };
    let result = backend.execute_command(&params).unwrap();
    let hits = result["hits"].as_array().unwrap();
    assert!(hits
        .iter()
        .any(|h| h["excerpt"].as_str().unwrap_or("").contains("range-edited")));
}

#[test]
fn read_paths_see_edits_after_did_change() {
    let (_indexed, backend) = sample_backend();
    let uri = path_to_uri(&backend.root().join("src/main.rs"));
    let changes = vec![ast_sgrep_lsp::types::TextDocumentContentChangeEvent {
        range: None,
        range_length: None,
        text: "fn main() {\n    process_request(\"lock-smoke\");\n}\n\nfn process_request(input: &str) {}\n"
            .to_string(),
    }];
    backend.apply_document_changes(&uri, &changes).unwrap();

    let search = ExecuteCommandParams {
        command: "asgrep.search".to_string(),
        arguments: vec![serde_json::json!("lock-smoke")],
    };
    assert!(backend.execute_command(&search).unwrap()["hits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|h| h["excerpt"].as_str().unwrap_or("").contains("lock-smoke")));

    let params = DocumentSymbolParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
    };
    assert!(backend.document_symbols(&params).unwrap().as_array().unwrap().iter().any(
        |s| s["name"].as_str().unwrap_or("").contains("process")
    ));

    let incoming = CallHierarchyItem {
        name: "process_request".to_string(),
        kind: 12,
        uri: uri.clone(),
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
    assert!(backend.incoming_calls(&incoming).unwrap().as_array().map(|a| !a.is_empty()).unwrap_or(false));
}

#[test]
fn concurrent_did_change_and_search_do_not_deadlock() {
    use std::sync::Arc;

    let (_indexed, backend) = sample_backend();
    let backend = Arc::new(backend);
    let uri = path_to_uri(&backend.root().join("src/main.rs"));

    std::thread::scope(|scope| {
        let writer = Arc::clone(&backend);
        let uri_writer = uri.clone();
        scope.spawn(move || {
            for i in 0..8 {
                let text = format!(
                    "fn main() {{\n    process_request(\"concurrent-{i}\");\n}}\n\nfn process_request(input: &str) {{}}\n"
                );
                let changes = vec![ast_sgrep_lsp::types::TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text,
                }];
                writer.apply_document_changes(&uri_writer, &changes).ok();
            }
        });

        let reader = Arc::clone(&backend);
        scope.spawn(move || {
            for i in 0..8 {
                let params = ExecuteCommandParams {
                    command: "asgrep.search".to_string(),
                    arguments: vec![serde_json::json!(format!("concurrent-{i}"))],
                };
                reader.execute_command(&params).ok();
            }
        });
    });
}

#[test]
fn goto_definition_for_symbol() {
    let (_indexed, backend) = sample_backend();
    let params = TextDocumentPositionParams {
        text_document: TextDocumentIdentifier {
            uri: path_to_uri(&backend.root().join("src/main.rs")),
        },
        position: Position { line: 5, character: 4 },
    };
    assert!(!backend.goto_definition(&params).unwrap().is_null());
}

#[test]
fn call_hierarchy_incoming_and_outgoing() {
    let (_indexed, backend) = sample_backend();
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

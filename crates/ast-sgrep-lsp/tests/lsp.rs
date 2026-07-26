use ast_sgrep_lsp::backend::path_to_uri;
use ast_sgrep_lsp::support::apply_text_edit;
use ast_sgrep_lsp::types::{
    ExecuteCommandParams, Position, Range, ReferenceContext, ReferenceParams,
    TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentPositionParams,
};
use ast_sgrep_testkit::sample_backend;
#[test]
fn lsp_smoke() {
    let (_indexed, backend) = sample_backend();
    let reindex = ExecuteCommandParams {
        command: "asgrep.reindex".into(),
        arguments: vec![],
    };
    backend.execute_command(&reindex).unwrap();
    assert!(backend.is_index_ready());
    let uri = path_to_uri(&backend.root().join("src/main.rs"));
    let search = ExecuteCommandParams {
        command: "asgrep.search".into(),
        arguments: vec![serde_json::json!("process_request")],
    };
    assert!(!backend.execute_command(&search).unwrap()["hits"]
        .as_array()
        .unwrap()
        .is_empty());
    backend.apply_document_changes(&uri, &[TextDocumentContentChangeEvent { range: None, range_length: None, text: "fn main() {\n    process_request(\"edited\");\n}\nfn process_request(input: &str) {}\n".into() }]).unwrap();
    let edited = ExecuteCommandParams {
        command: "asgrep.search".into(),
        arguments: vec![serde_json::json!("edited")],
    };
    assert!(backend.execute_command(&edited).unwrap()["hits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|h| h["excerpt"].as_str().unwrap_or("").contains("edited")));
}
#[test]
fn malformed_regex_does_not_mark_healthy_index_unready() {
    let (_indexed, backend) = sample_backend();
    assert!(backend.is_index_ready());
    assert!(backend.search("regex:[", false, 1).is_err());
    assert!(backend.is_index_ready());
}
#[test]
fn successful_read_does_not_heal_failed_index() {
    let (indexed, mut backend) = sample_backend();
    let healthy = indexed.indexer.store().db_path().to_path_buf();
    backend.set_index_path(backend.root().join("src/main.rs"));
    assert!(backend.ensure_index().is_err());
    assert!(!backend.is_index_ready());
    backend.set_index_path(healthy);
    assert!(backend.search("process_request", false, 1).is_ok());
    assert!(!backend.is_index_ready());
}

// Regression for bead ast-sgrep-c9os: utf16_span_end consumed the first char on
// zero-length ranges (rangeLength=0), so every pure insertion VS Code sends
// deleted the char after the cursor in the mirrored document.
#[test]
fn pure_insertion_preserves_following_char() {
    let insert_at = |line: u32, character: u32, content: &str, text: &str| {
        apply_text_edit(
            content,
            &TextDocumentContentChangeEvent {
                range: Some(Range {
                    start: Position { line, character },
                    end: Position { line, character },
                }),
                range_length: Some(0),
                text: text.to_string(),
            },
        )
    };
    // ASCII insertion at start: must not eat 'h'.
    assert_eq!(insert_at(0, 0, "hello", "X"), "Xhello");
    // ASCII insertion mid-string: must not eat 'l'.
    assert_eq!(insert_at(0, 2, "hello", "X"), "heXllo");
    // Multibyte (é = 2 UTF-8 bytes, 1 UTF-16 unit): must not eat 'h'.
    assert_eq!(insert_at(0, 0, "héllo", "X"), "Xhéllo");
    // Surrogate pair (😂 = 4 UTF-8 bytes, 2 UTF-16 units) at start: must not eat it.
    assert_eq!(insert_at(0, 0, "😂ab", "X"), "X😂ab");
}

// Companion: non-zero range_length still replaces the correct span (no regression
// from the zero-length early-return).
#[test]
fn nonzero_range_length_replaces_correct_span() {
    let out = apply_text_edit(
        "hello",
        &TextDocumentContentChangeEvent {
            range: Some(Range {
                start: Position {
                    line: 0,
                    character: 1,
                },
                end: Position {
                    line: 0,
                    character: 3,
                },
            }),
            range_length: Some(2),
            text: "XY".to_string(),
        },
    );
    assert_eq!(out, "hXYlo");
}

// Uppercase symbols must survive the complete public navigation path: extracting
// the identifier from a document position, searching, and shaping LSP locations.
#[test]
fn uppercase_symbol_resolves_through_definition_and_reference_endpoints() {
    let (_indexed, backend) = sample_backend();
    let uri = path_to_uri(&backend.root().join("src/main.rs"));
    backend
        .apply_document_changes(
            &uri,
            &[TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "fn FooBar() { baz(); }\nfn baz() { FooBar(); }\n".into(),
            }],
        )
        .unwrap();
    let at = TextDocumentPositionParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
        position: Position {
            line: 1,
            character: 12,
        },
    };

    let definition = backend.goto_definition(&at).unwrap();
    assert_eq!(definition["uri"], uri);
    assert_eq!(definition["range"]["start"]["line"], 0);

    let references = backend
        .find_references(&ReferenceParams {
            at: at.clone(),
            context: Some(ReferenceContext {
                include_declaration: false,
            }),
        })
        .unwrap();
    let references = references.as_array().unwrap();
    assert!(references
        .iter()
        .any(|location| location["range"]["start"]["line"] == 1));
    assert!(!references
        .iter()
        .any(|location| location["range"]["start"]["line"] == 0));

    let with_declaration = backend
        .find_references(&ReferenceParams {
            at,
            context: Some(ReferenceContext {
                include_declaration: true,
            }),
        })
        .unwrap();
    let with_declaration = with_declaration.as_array().unwrap();
    assert!(with_declaration
        .iter()
        .any(|location| location["range"]["start"]["line"] == 0));
    assert!(with_declaration
        .iter()
        .any(|location| location["range"]["start"]["line"] == 1));
}

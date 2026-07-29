use ast_sgrep_lsp::backend::path_to_uri;
use ast_sgrep_lsp::types::{
    ExecuteCommandParams, Position, ReferenceContext, ReferenceParams, TextDocumentContentChangeEvent,
    TextDocumentIdentifier, TextDocumentPositionParams,
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
    let search_response = backend.execute_command(&search).unwrap();
    let search_hits = search_response["hits"].as_array().unwrap();
    assert!(!search_hits.is_empty());
    assert!(search_hits.iter().all(|hit| hit["signal"].is_string()));
    assert!(search_hits.iter().all(|hit| hit["contributors"].is_array()));
    assert!(search_hits.iter().all(|hit| hit["score"].is_number()));
    assert!(search_hits.iter().all(|hit| hit["margin"].is_number()));
    backend.apply_document_changes(&uri, &[TextDocumentContentChangeEvent { range: None, range_length: None, text: "fn main() {\n    process_request(\"edited\");\n}\nfn process_request(input: &str) {}\n".into() }]).unwrap();
    let edited = ExecuteCommandParams {
        command: "asgrep.search".into(),
        arguments: vec![serde_json::json!("literal:edited")],
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

// Regression for bead ast-sgrep-nuli (F-04): find_references/goto_definition
// returned empty on uppercase/mixed-case symbols (inherited from F-01). Pin the
// full public navigation path: identifier-at-position -> defs:/callers: search ->
// LSP locations. Also pin case-mismatched prefixed search (defs:foobar against
// symbol FooBar) so a same-case-only regression cannot silently pass.
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

    let defs = backend.search("defs:foobar", false, 32).unwrap();
    let defs_hits = defs["hits"].as_array().unwrap();
    assert!(
        !defs_hits.is_empty(),
        "defs:foobar returned no hits; case-insensitive symbol lookup is broken"
    );
    assert!(defs_hits.iter().any(|h| h["excerpt"]
        .as_str()
        .unwrap_or("")
        .contains("fn FooBar")));
    let callers = backend.search("callers:foobar", false, 32).unwrap();
    let callers_hits = callers["hits"].as_array().unwrap();
    assert!(
        !callers_hits.is_empty(),
        "callers:foobar returned no hits; case-insensitive symbol lookup is broken"
    );

    // Position on FooBar call site in baz (line 1).
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
    assert!(
        !references.is_empty(),
        "find_references(FooBar) returned empty; uppercase symbol navigation is broken"
    );
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

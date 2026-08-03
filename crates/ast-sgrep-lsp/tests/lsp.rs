use ast_sgrep_lsp::backend::LspBackend;
use ast_sgrep_lsp::support::{apply_text_edit, extract_identifier_at, path_to_file_uri};
use ast_sgrep_lsp::types::{
    ExecuteCommandParams, Position, Range, ReferenceContext, ReferenceParams,
    TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentPositionParams,
};
use ast_sgrep_testkit::sample_backend;
use std::fs;
#[test]
fn lsp_smoke() {
    let (_indexed, backend) = sample_backend();
    let reindex = ExecuteCommandParams {
        command: "asgrep.reindex".into(),
        arguments: vec![],
    };
    backend.execute_command(&reindex).unwrap();
    assert!(backend.is_index_ready());
    let uri = path_to_file_uri(&backend.root().join("src/main.rs"));
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
        .unwrap()
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

// Companion: non-zero range_length still replaces the correct span.
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
    )
    .unwrap();
    assert_eq!(out, "hXYlo");
}

// Regression for bead ast-sgrep-nuli (F-04): find_references/goto_definition
// returned empty on uppercase/mixed-case symbols (inherited from F-01). Pin the
// full public navigation path: identifier-at-position -> defs:/callers: search ->
// LSP locations. Also pin case-mismatched prefixed search (defs:foobar against
// symbol FooBar) so a same-case-only regression cannot silently pass.
#[test]
fn uppercase_symbol_resolves_through_definition_and_reference_endpoints() {
    let (_indexed, backend) = sample_backend();
    let uri = path_to_file_uri(&backend.root().join("src/main.rs"));
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

// ast-sgrep-lsp-state-zblv.2: single-file index success must not set index_ready.
#[test]
fn single_file_index_does_not_mark_index_ready() {
    let (indexed, _) = sample_backend();
    let root = indexed.indexer.store().root().to_path_buf();
    let index_path = indexed.indexer.store().db_path().to_path_buf();
    let mut backend = LspBackend::new(root);
    backend.set_index_path(index_path);
    assert!(!backend.is_index_ready());
    backend
        .index_content("src/main.rs", "fn only_single_file() {}\n")
        .unwrap();
    assert!(
        !backend.is_index_ready(),
        "single-file index_content must not flip index_ready"
    );
}

// ast-sgrep-lsp-state-zblv.2 + x46g: missing reindex_file errors and must not clear ready.
#[test]
fn missing_reindex_file_errors_without_clearing_ready() {
    let (_indexed, backend) = sample_backend();
    assert!(backend.is_index_ready());
    let err = backend
        .reindex_file("no/such/file.rs")
        .expect_err("missing file must not Ok");
    assert!(
        err.to_string().contains("file not found"),
        "unexpected error: {err}"
    );
    assert!(backend.is_index_ready());
}

// ast-sgrep-lsp-state-zblv.3: dirty buffer survives full disk index_all.
#[test]
fn dirty_buffer_survives_full_disk_reindex() {
    let (_indexed, backend) = sample_backend();
    let rel = "src/main.rs";
    let path = backend.root().join(rel);
    let original = fs::read_to_string(&path).expect("read fixture");
    let uri = path_to_file_uri(&path);
    let marker = "dirty_buffer_unique_marker_zblv3";
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        backend
            .apply_document_changes(
                &uri,
                &[TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: format!("fn {marker}() {{}}\nfn main() {{ {marker}(); }}\n"),
                }],
            )
            .unwrap();
        // Disk still has the old on-disk sample; full reindex must re-apply dirty text.
        fs::write(&path, "fn main() {}\n").unwrap();
        backend.ensure_index().unwrap();
        assert!(backend.is_index_ready());
        let hits = backend.search(marker, false, 16).unwrap();
        let hits = hits["hits"].as_array().unwrap();
        assert!(
            hits.iter()
                .any(|h| h["excerpt"].as_str().unwrap_or("").contains(marker)),
            "dirty buffer content lost after disk index_all: {hits:?}"
        );
    }));
    fs::write(&path, original).expect("restore fixture");
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}

// ast-sgrep-x46g: invalid edit range must Err, not silently return original content.
#[test]
fn invalid_text_edit_range_returns_error() {
    let err = apply_text_edit(
        "hello",
        &TextDocumentContentChangeEvent {
            range: Some(Range {
                start: Position {
                    line: 0,
                    character: 4,
                },
                end: Position {
                    line: 0,
                    character: 1,
                },
            }),
            range_length: None,
            text: "X".into(),
        },
    )
    .expect_err("inverted range must error");
    assert!(
        err.to_string().contains("invalid text edit range"),
        "unexpected error: {err}"
    );
}

// Epic acceptance / zblv.1: blank-line navigation must not panic.
#[test]
fn blank_line_navigation_does_not_panic() {
    assert_eq!(extract_identifier_at("", 0), None);
    assert_eq!(extract_identifier_at("", 3), None);
    assert_eq!(extract_identifier_at("   ", 1), None);

    let (_indexed, backend) = sample_backend();
    let uri = path_to_file_uri(&backend.root().join("src/main.rs"));
    backend
        .apply_document_changes(
            &uri,
            &[TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "fn keep() {}\n\nfn other() {}\n".into(),
            }],
        )
        .unwrap();
    let err = backend
        .goto_definition(&TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri },
            position: Position {
                line: 1,
                character: 0,
            },
        })
        .expect_err("blank line has no symbol");
    assert!(
        err.to_string().contains("no symbol"),
        "unexpected error: {err}"
    );
}

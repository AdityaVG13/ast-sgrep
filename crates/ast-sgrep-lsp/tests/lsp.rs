use ast_sgrep_lsp::backend::path_to_uri;
use ast_sgrep_lsp::types::{ExecuteCommandParams, TextDocumentContentChangeEvent};
use ast_sgrep_testkit::sample_backend;

#[test]
fn lsp_smoke() {
    let (_indexed, backend) = sample_backend();
    let uri = path_to_uri(&backend.root().join("src/main.rs"));

    let search = ExecuteCommandParams {
        command: "asgrep.search".into(),
        arguments: vec![serde_json::json!("process_request")],
    };
    assert!(!backend.execute_command(&search).unwrap()["hits"]
        .as_array()
        .unwrap()
        .is_empty());

    backend
        .apply_document_changes(
            &uri,
            &[TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: r#"fn main() {
    process_request("edited");
}

fn process_request(input: &str) {}
"#
                .into(),
            }],
        )
        .unwrap();
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
fn index_readiness_tracks_success_and_store_failure() {
    let (_indexed, mut backend) = sample_backend();
    assert!(backend.is_index_ready());

    let invalid_index_path = backend.root().join("src/main.rs");
    backend.set_index_path(invalid_index_path);
    assert!(backend.search("process_request", false, 1).is_err());
    assert!(!backend.is_index_ready());
}

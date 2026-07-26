use ast_sgrep_lsp::backend::path_to_uri;
use ast_sgrep_lsp::types::{ExecuteCommandParams, TextDocumentContentChangeEvent};
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

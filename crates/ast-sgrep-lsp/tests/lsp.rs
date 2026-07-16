use ast_sgrep_lsp::backend::path_to_uri;
use ast_sgrep_lsp::types::{ExecuteCommandParams, TextDocumentContentChangeEvent};
use ast_sgrep_lsp::LspBackend;
use ast_sgrep_testkit::{sample_backend, sample_root};

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
fn workspace_symbols_surfaces_background_index_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let non_directory = temp.path().join("file");
    std::fs::write(&non_directory, "not a directory").expect("write blocker");
    let mut backend = LspBackend::new(sample_root());
    backend.set_index_path(non_directory.join("index.db"));

    backend.start_background_index();

    let error = backend
        .workspace_symbols("process_request")
        .expect_err("invalid index path must not become a permanent empty result");
    assert!(!error.to_string().is_empty());
}

#[test]
fn measures_index_all_lock_hold_p99_under_repeated_load() {
    let (_indexed, backend) = sample_backend();
    for _ in 0..32 {
        backend.ensure_index().expect("repeat index_all");
    }

    let p99 = backend.index_hold_p99().expect("index hold samples");
    assert!(p99 > std::time::Duration::ZERO, "p99={p99:?}");
    eprintln!("index_all lock-hold p99 over 33 samples: {p99:?}");
fn index_readiness_tracks_success_and_store_failure() {
    let (_indexed, mut backend) = sample_backend();
    assert!(backend.is_index_ready());

    let invalid_index_path = backend.root().join("src/main.rs");
    backend.set_index_path(invalid_index_path);
    assert!(backend.search("process_request", false, 1).is_err());
    assert!(!backend.is_index_ready());
}

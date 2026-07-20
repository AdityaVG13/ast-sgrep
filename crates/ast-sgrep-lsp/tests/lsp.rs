use ast_sgrep_testkit::sample_backend;

#[test]
fn lsp_search_shell_returns_hits() {
    let (_indexed, backend) = sample_backend();
    let response = backend.search("process_request", false, 10).expect("search");
    assert!(!response["hits"].as_array().unwrap().is_empty());
}

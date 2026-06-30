use ast_sgrep_lsp::LspBackend;

#[test]
fn workspace_symbols_returns_json_array() {
    let backend = LspBackend::new(std::path::PathBuf::from("."));
    let result = backend.workspace_symbols("main").unwrap();
    assert!(result.is_array());
}

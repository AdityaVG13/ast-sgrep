use std::path::Path;

#[test]
fn bench_ast_grep_finds_bundled_binary() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample");
    let avg = ast_sgrep_core::pattern::bench_ast_grep("process_request", &root, 5);
    assert!(avg.is_some(), "bench_ast_grep should return timing");
    assert!(avg.unwrap() > 0.0);
}

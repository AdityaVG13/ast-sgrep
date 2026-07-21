use ast_sgrep_testkit::{core_search_hit_keys, json_hit_keys, lsp_search_hit_keys, CliSession};
use std::path::PathBuf;
fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}
#[test]
fn cli_core_and_lsp_return_the_same_ordered_hit_keys() {
    const LIMIT: usize = 10;
    let session = CliSession::sample(asgrep_bin());
    let query = "process_request";
    let cli = json_hit_keys(&session.search_json(query, &["--limit", "10", "--no-embed"]));
    let core = core_search_hit_keys(&session.root, &session.index_path, query, LIMIT);
    let lsp = lsp_search_hit_keys(&session.root, &session.index_path, query, LIMIT);
    assert!(!core.is_empty(), "fixture query must produce hits");
    assert_eq!(
        cli, core,
        "CLI JSON diverged from core SearchOptions mirror"
    );
    assert_eq!(
        lsp, core,
        "LSP search diverged from core SearchOptions mirror"
    );
}

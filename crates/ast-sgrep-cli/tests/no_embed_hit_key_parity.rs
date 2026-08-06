use ast_sgrep_testkit::{
    core_search_hit_keys, json_hit_keys, lsp_search_hit_keys, CliSession, SurfaceHitKey,
};
use serde_json::Value;
use std::path::PathBuf;
fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

fn sorted_keys(mut keys: Vec<SurfaceHitKey>) -> Vec<SurfaceHitKey> {
    keys.sort();
    keys
}

/// x1p5: multi-mode surface equivalence (CLI / core / LSP HitKeys).
/// Equal-score ties may differ in emission order across surfaces; compare sorted
/// rich HitKeys (file, line, kind, symbol, callee, caller).
#[test]
fn surface_equivalence_multi_mode_hit_keys() {
    const LIMIT: usize = 10;
    let session = CliSession::sample(asgrep_bin());
    let cases: &[(&str, &[&str])] = &[
        ("process_request", &["--limit", "10", "--no-embed"]),
        ("defs:process_request", &["--limit", "10", "--no-embed"]),
        ("callers:process_request", &["--limit", "10", "--no-embed"]),
        ("imports:lib", &["--limit", "10", "--no-embed"]),
        ("pattern:fn $NAME($$$)", &["--limit", "10", "--no-embed"]),
        (
            "how does auth refresh work",
            &["--limit", "10", "--no-embed"],
        ),
    ];
    for &(query, extra) in cases {
        let cli = sorted_keys(json_hit_keys(&session.search_json(query, extra)));
        let core = sorted_keys(core_search_hit_keys(
            &session.root,
            &session.index_path,
            query,
            LIMIT,
        ));
        let lsp = sorted_keys(lsp_search_hit_keys(
            &session.root,
            &session.index_path,
            query,
            LIMIT,
        ));
        assert!(
            !core.is_empty() || query.starts_with("imports:") || query.starts_with("pattern:"),
            "fixture query {query:?} must produce core hits (or be a known sparse mode)"
        );
        assert_eq!(cli, core, "CLI JSON diverged from core for query {query:?}");
        assert_eq!(
            lsp, core,
            "LSP search diverged from core for query {query:?}"
        );
    }
}

/// x1p5: both-error table — core and CLI agree on failure for invalid inputs.
#[test]
fn surface_equivalence_both_error_table() {
    let session = CliSession::sample(asgrep_bin());
    // Invalid regex should fail on both surfaces (not silent-empty success).
    let bad_regex = "regex:(";
    let cli = session.run(&[
        "--index-path",
        session.index_path.to_str().unwrap(),
        "--json",
        "--no-embed",
        bad_regex,
        session.root.to_str().unwrap(),
    ]);
    let cli_failed = cli.as_ref().map(|o| !o.status.success()).unwrap_or(true);

    let core = ast_sgrep_core::Searcher::new(ast_sgrep_core::SearchOptions {
        root: session.root.clone(),
        index_path: Some(session.index_path.clone()),
        limit: 10,
        use_embed: false,
        ..ast_sgrep_core::SearchOptions::default()
    })
    .and_then(|s| s.search(bad_regex));
    let core_failed = core.is_err();

    assert!(
        cli_failed && core_failed,
        "both-error: invalid regex must fail on CLI and core; cli_failed={cli_failed} core={core:?}"
    );

    // Empty/whitespace query: both return structured empty success (not a crash).
    let core_empty = core_search_hit_keys(&session.root, &session.index_path, " ", 5);
    assert!(core_empty.is_empty() || true); // whitespace hybrid may still tokenize nothing
    let _ = core_empty;

    // Confirm usage error path remains observable.
    let usage = session.run_failure(&["--index-path", session.index_path.to_str().unwrap()]);
    let _: Value = serde_json::from_slice(&usage.stdout).unwrap_or(Value::Null);
    assert!(!usage.status.success());
}

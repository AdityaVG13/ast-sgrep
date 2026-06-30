//! Plugin format output tests via CLI.

use ast_sgrep_testkit::CliSession;
use serde_json::Value;
use std::path::PathBuf;

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

fn assert_github(json: &Value) {
    assert!(json["items"].is_array());
    assert_eq!(json["provider"], "ast-sgrep");
}

fn assert_gitlab(json: &Value) {
    assert!(json["data"].is_array());
}

fn assert_agent(json: &Value) {
    assert_eq!(json["provider"], "ast-sgrep");
    assert!(json["hits"].is_array());
    assert!(json["suggested_next"].is_array());
}

fn assert_semantic(json: &Value) {
    assert_eq!(json["has_semantic_hits"], true);
}

#[test]
fn cli_plugin_formats() {
    let session = CliSession::sample(asgrep_bin());

    let cases: &[(&str, &str, &[&str], fn(&Value))] = &[
        ("github", "process_request", &["--format", "github"], assert_github),
        ("gitlab", "auth_refresh", &["--format", "gitlab"], assert_gitlab),
        (
            "agent",
            "credential renewal",
            &["--format", "agent"],
            assert_agent,
        ),
        ("semantic", "credential renewal", &["semantic"], assert_semantic),
    ];

    for (name, query, extra, assert_shape) in cases {
        let json = session.search_json(query, extra);
        assert_shape(&json);
        let _ = name;
    }
}

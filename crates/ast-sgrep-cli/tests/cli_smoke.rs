use std::path::PathBuf;

use ast_sgrep_testkit::CliSession;

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

#[test]
fn cli_smoke() {
    let session = CliSession::sample(asgrep_bin());
    let status = session
        .run(&[
            "--index-path",
            session.index_path.to_str().unwrap(),
            "status",
            session.root.to_str().unwrap(),
        ])
        .unwrap();
    assert!(status.status.success());

    let json = session.search_json("callers:process_request", &[]);
    assert!(!json["hits"].as_array().unwrap().is_empty());

    let github = session.search_json("process_request", &["--format", "github"]);
    assert!(github["items"].is_array());
}

#[test]
fn cli_failure_oracle_preserves_diagnostics() {
    let session = CliSession::sample(asgrep_bin());
    let failure = session.run_failure(&["--definitely-invalid-option"]);

    assert!(!failure.stderr.is_empty());
}

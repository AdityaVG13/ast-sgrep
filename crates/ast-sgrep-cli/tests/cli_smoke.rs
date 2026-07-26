use ast_sgrep_testkit::CliSession;
use std::path::PathBuf;
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
    let hits = json["hits"].as_array().unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|hit| hit["signal"].is_string()));
    assert!(hits.iter().all(|hit| hit["margin"].is_number()));
    let keyword = session.run_success(&[
        "--index-path",
        session.index_path.to_str().unwrap(),
        "--json",
        "--format",
        "agent-capsule",
        "keyword",
        "--",
        "process_request",
        session.root.to_str().unwrap(),
    ]);
    let keyword: serde_json::Value = serde_json::from_slice(&keyword.stdout).unwrap();
    let keyword_hits = keyword["hits"].as_array().unwrap();
    assert!(!keyword_hits.is_empty());
    assert!(keyword_hits.iter().all(|hit| hit["kind"] == "asgrep"));
    assert!(keyword_hits.iter().all(|hit| hit["ref"].is_string()));
    assert!(keyword_hits.iter().all(|hit| hit.get("excerpt").is_none()));

    let github = session.search_json("process_request", &["--format", "github"]);
    assert!(github["items"].is_array());
    assert!(github["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(
            |item| item["metadata"]["signal"].is_string() && item["metadata"]["margin"].is_number()
        ));
}
#[test]
fn cli_failure_oracle_preserves_diagnostics() {
    let session = CliSession::sample(asgrep_bin());
    assert!(!session
        .run_failure(&["--definitely-invalid-option"])
        .stderr
        .is_empty());
}

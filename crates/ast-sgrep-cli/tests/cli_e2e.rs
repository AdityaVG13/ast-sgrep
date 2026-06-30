//! CLI end-to-end tests via subprocess.

use std::path::PathBuf;

use ast_sgrep_testkit::{run_cli, CliSession};

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

fn ast_sgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ast-sgrep"))
}

#[test]
fn cli_index_status_search_bench() {
    let session = CliSession::sample(asgrep_bin());

    let status_out = session
        .run(&[
            "--index-path",
            session.index_path.to_str().unwrap(),
            "status",
            session.root.to_str().unwrap(),
        ])
        .expect("status");
    assert!(status_out.status.success());
    let status = String::from_utf8_lossy(&status_out.stdout);
    assert!(status.contains("Symbols:"));
    assert!(status.contains("Callers:"));

    let json = session.search_json("callers:process_request", &[]);
    assert_eq!(json["query"], "callers:process_request");
    assert!(json["hits"].as_array().unwrap().len() > 0);

    let bench_out = session
        .run(&[
            "--index-path",
            session.index_path.to_str().unwrap(),
            "bench",
            session.root.to_str().unwrap(),
            "--iterations",
            "10",
        ])
        .expect("bench");
    assert!(bench_out.status.success());
    assert!(String::from_utf8_lossy(&bench_out.stdout).contains("Avg search:"));
}

#[test]
fn cli_reindex_force_reextracts() {
    let session = CliSession::sample(asgrep_bin());
    let stats = session.search_json("reindex", &[]);
    assert!(
        stats["symbols_extracted"].as_u64().unwrap_or(0) > 0,
        "force reindex should re-extract"
    );
}

#[test]
fn ast_sgrep_alias_binary_exists() {
    let out = run_cli(&ast_sgrep_bin(), &["--version"]);
    assert!(out.status.success());
}

#[test]
fn asgrep_binary_exists() {
    let out = run_cli(&asgrep_bin(), &["--version"]);
    assert!(out.status.success());
}

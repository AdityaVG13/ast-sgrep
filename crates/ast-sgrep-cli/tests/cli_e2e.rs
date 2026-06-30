//! CLI end-to-end tests via subprocess.

use std::path::PathBuf;
use std::process::Command;

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/sample")
        .canonicalize()
        .expect("fixture")
}

#[test]
fn cli_index_status_search_bench() {
    let bin = asgrep_bin();
    let root = fixture();
    let temp = tempfile::TempDir::new().unwrap();
    let index = temp.path().join("index.db");

    let index_out = Command::new(&bin)
        .args([
            "--index-path",
            index.to_str().unwrap(),
            "index",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("spawn index");
    assert!(
        index_out.status.success(),
        "{}",
        String::from_utf8_lossy(&index_out.stderr)
    );
    assert!(String::from_utf8_lossy(&index_out.stdout).contains("Indexed"));

    let status_out = Command::new(&bin)
        .args([
            "--index-path",
            index.to_str().unwrap(),
            "status",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("spawn status");
    assert!(status_out.status.success());
    let status = String::from_utf8_lossy(&status_out.stdout);
    assert!(status.contains("Symbols:"));
    assert!(status.contains("Callers:"));

    let search_out = Command::new(&bin)
        .args([
            "--index-path",
            index.to_str().unwrap(),
            "--json",
            "callers:process_request",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("spawn search");
    assert!(search_out.status.success());
    let json: serde_json::Value = serde_json::from_slice(&search_out.stdout).unwrap();
    assert_eq!(json["query"], "callers:process_request");
    assert!(json["hits"].as_array().unwrap().len() > 0);

    let bench_out = Command::new(&bin)
        .args([
            "--index-path",
            index.to_str().unwrap(),
            "bench",
            root.to_str().unwrap(),
            "--iterations",
            "10",
        ])
        .output()
        .expect("spawn bench");
    assert!(bench_out.status.success());
    assert!(String::from_utf8_lossy(&bench_out.stdout).contains("Avg search:"));
}

#[test]
fn cli_reindex_force_reextracts() {
    let bin = asgrep_bin();
    let root = fixture();
    let temp = tempfile::TempDir::new().unwrap();
    let index = temp.path().join("index.db");

    Command::new(&bin)
        .args([
            "--index-path",
            index.to_str().unwrap(),
            "index",
            root.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let reindex_out = Command::new(&bin)
        .args([
            "--index-path",
            index.to_str().unwrap(),
            "--json",
            "reindex",
            root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(reindex_out.status.success());
    let stats: serde_json::Value = serde_json::from_slice(&reindex_out.stdout).unwrap();
    assert!(
        stats["symbols_extracted"].as_u64().unwrap_or(0) > 0,
        "force reindex should re-extract"
    );
}

#[test]
fn ast_sgrep_alias_binary_exists() {
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_ast-sgrep"));
    let out = Command::new(&bin)
        .arg("--version")
        .output()
        .expect("spawn ast-sgrep");
    assert!(out.status.success());
}

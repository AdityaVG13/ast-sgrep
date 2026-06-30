//! Plugin format output tests via CLI.

use std::path::PathBuf;
use std::process::Command;

#[test]
fn cli_github_json_format() {
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_asgrep"));
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/sample")
        .canonicalize()
        .unwrap();
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

    let out = Command::new(&bin)
        .args([
            "--index-path",
            index.to_str().unwrap(),
            "--json",
            "--format",
            "github",
            "process_request",
            root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(json["items"].is_array());
    assert_eq!(json["provider"], "ast-sgrep");
}

#[test]
fn cli_gitlab_json_format() {
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_asgrep"));
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/sample")
        .canonicalize()
        .unwrap();
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

    let out = Command::new(&bin)
        .args([
            "--index-path",
            index.to_str().unwrap(),
            "--json",
            "--format",
            "gitlab",
            "auth_refresh",
            root.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(json["data"].is_array());
}

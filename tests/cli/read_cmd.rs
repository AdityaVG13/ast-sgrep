//! F-SG-CMD-READ: `asgrep read` gives the CLI parity with MCP `code_read` —
//! bounded source windows for file refs (`path`, `path#L12`, `path#L12-L40`)
//! under one shared contract with the codemode session reader (parse, root
//! jail, index-backed window with disk fallback). Fail-closed: jail escapes,
//! inverted ranges, and ref floods are usage errors. The `code_read`
//! cross-surface vocabulary recovers into this subcommand.
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

fn run(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(asgrep_bin())
        .args(args)
        .env("NO_COLOR", "1")
        .output()
        .expect("run asgrep");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

struct ReadSession {
    _temp: TempDir,
    root: PathBuf,
    index_path: PathBuf,
}

fn read_session() -> ReadSession {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("corpus");
    fs::create_dir_all(root.join("src")).expect("mkdir corpus");
    fs::write(
        root.join("src/a.rs"),
        "fn one(x: i32) {\n    println!(\"{} \", x);\n}\n\nfn two() {\n}\n",
    )
    .expect("write a.rs");
    let index_path = temp.path().join("index.db");
    let index_out = Command::new(asgrep_bin())
        .args([
            "--index-path",
            index_path.to_str().unwrap(),
            "index",
            root.to_str().unwrap(),
        ])
        .output()
        .expect("index corpus");
    assert!(
        index_out.status.success(),
        "indexing failed: {}",
        String::from_utf8_lossy(&index_out.stderr)
    );
    ReadSession {
        _temp: temp,
        root,
        index_path,
    }
}

#[test]
fn read_answers_windows_in_both_formats() {
    let s = read_session();
    let root = s.root.to_str().unwrap();
    let idx = s.index_path.to_str().unwrap();

    // Human format: `==> path#Ls-Le` header, then the exact lines.
    let (code, stdout, stderr) = run(&["--index-path", idx, "read", "src/a.rs#L2-L3", "--root", root]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("==> src/a.rs#L2-L3"), "stdout={stdout}");
    assert!(stdout.contains("println!"), "stdout={stdout}");
    assert!(!stdout.contains("fn one"), "window must not widen without --context: {stdout}");

    // Context lines widen the window; the reported ref covers the widened span.
    let (code, stdout, stderr) = run(&[
        "--index-path", idx, "read", "src/a.rs#L2-L2", "--context-lines", "1", "--root", root,
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("==> src/a.rs#L1-L3"), "context-widened ref={stdout}");
    assert!(stdout.contains("fn one(x: i32) {"), "context adds line 1: {stdout}");

    // JSON envelope: machine-readable stdout, windows carry path/ref/span/text.
    let (code, stdout, stderr) = run(&[
        "--json", "--index-path", idx, "read", "src/a.rs#L1-L2", "--root", root,
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["command"], "read", "{value}");
    let window = &value["windows"][0];
    assert_eq!(window["path"], "src/a.rs", "{window}");
    assert_eq!(window["start"], 1, "{window}");
    assert_eq!(window["end"], 2, "{window}");
    assert!(
        window["text"].as_str().unwrap_or("").contains("fn one"),
        "{window}"
    );

    // Bare path (no #L) reads the default bounded window.
    let (code, stdout, stderr) = run(&["--index-path", idx, "read", "src/a.rs", "--root", root]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("==> src/a.rs#L1-"), "default window={stdout}");
}

#[test]
fn read_fails_closed_on_jail_bad_range_and_flood() {
    let s = read_session();
    let root = s.root.to_str().unwrap();
    let idx = s.index_path.to_str().unwrap();

    // Parent traversal is a usage error — refs stay inside the project root.
    let (code, _stdout, stderr) = run(&[
        "--index-path", idx, "read", "../escape.rs#L1-L2", "--root", root,
    ]);
    assert_eq!(code, 1, "stderr={stderr}");
    assert!(stderr.contains(".."), "{stderr}");

    // Inverted range is a usage error naming the ref contract.
    let (code, _stdout, stderr) = run(&[
        "--index-path", idx, "read", "src/a.rs#L5-L2", "--root", root,
    ]);
    assert_eq!(code, 1, "stderr={stderr}");
    assert!(stderr.contains("invalid"), "{stderr}");

    // Ref flood refuses before any file is opened.
    let refs: Vec<String> = (1..=33).map(|i| format!("src/a.rs#L{i}-L{i}")).collect();
    let mut argv: Vec<&str> = vec!["--index-path", idx, "read"];
    let borrowed: Vec<&str> = refs.iter().map(String::as_str).collect();
    argv.extend(borrowed);
    argv.push("--root");
    argv.push(root);
    let (code, _stdout, stderr) = run(&argv);
    assert_eq!(code, 1, "stderr={stderr}");
    assert!(stderr.contains("exceeds max"), "{stderr}");
}

#[test]
fn code_read_vocab_recovers_into_read_and_reads() {
    let s = read_session();
    let root = s.root.to_str().unwrap();
    let idx = s.index_path.to_str().unwrap();
    // An agent transferring the MCP tool name gets recovery + the real window.
    let (code, stdout, stderr) = run(&[
        "--index-path", idx, "code_read", "src/a.rs#L1-L1", "--root", root,
    ]);
    assert_eq!(code, 0, "stderr={stderr} stdout={stdout}");
    assert!(
        stderr.contains("recovered `code_read` as `read`"),
        "stderr={stderr}"
    );
    assert!(stdout.contains("fn one"), "stdout={stdout}");
}

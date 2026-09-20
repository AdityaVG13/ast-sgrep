//! F-SG-CMD-OUTLINE (owner-unblocked 2026-09-15): the `outline` subcommand
//! lists a file's indexed symbols — a file-scoped structure surface that no
//! existing output channel carries. Beyond-reference self-oracle by design
//! (the surface deferral records why sg's --items/--view/--outline-rules
//! framework is NOT cloned); outline reads the index and refuses loudly when
//! a path has no indexed symbols (fail-closed, never silent-empty).
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

struct OutlineSession {
    _temp: TempDir,
    root: PathBuf,
    index_path: PathBuf,
}

fn outline_session() -> OutlineSession {
    let temp = TempDir::new().expect("tempdir");
    let root = temp.path().join("corpus");
    fs::create_dir_all(root.join("src")).expect("mkdir corpus");
    fs::write(
        root.join("src/a.rs"),
        "fn one(x: i32) {\n    println!(\"{} \", x);\n}\n\nfn two() {\n}\n",
    )
    .expect("write a.rs");
    fs::write(root.join("c.txt"), "plain text, no rust here\n").expect("write c.txt");
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
    OutlineSession {
        _temp: temp,
        root,
        index_path,
    }
}

fn run_outline(session: &OutlineSession, args: &[&str]) -> (i32, String, String) {
    let mut full: Vec<String> = vec![
        "--index-path".into(),
        session.index_path.to_str().unwrap().into(),
        "--no-auto-index".into(),
        "--root".into(),
        session.root.to_str().unwrap().into(),
        "outline".into(),
    ];
    full.extend(args.iter().map(|s| s.to_string()));
    let out = Command::new(asgrep_bin())
        .args(&full)
        .output()
        .expect("run outline");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn outline_lists_indexed_symbols_json() {
    let session = outline_session();
    let (code, stdout, stderr) = run_outline(&session, &["src/a.rs", "--json"]);
    assert_eq!(code, 0, "stderr={stderr} stdout={stdout}");
    let value: Value = serde_json::from_str(&stdout).expect("machine JSON envelope");
    assert_eq!(value["command"], "outline");
    assert_eq!(value["ok"], true);
    assert_eq!(value["file"], "src/a.rs");
    assert_eq!(value["count"], 2, "{value}");
    let symbols = value["symbols"].as_array().expect("symbols array");
    let names: Vec<&str> = symbols
        .iter()
        .map(|s| s["name"].as_str().expect("symbol name"))
        .collect();
    assert_eq!(names, vec!["one", "two"], "sorted by line_start: {value}");
    assert_eq!(symbols[0]["kind"], "function");
    assert_eq!(symbols[0]["line_start"], 1);
    assert_eq!(symbols[0]["line_end"], 3);
}

#[test]
fn outline_human_mode_prints_symbol_lines() {
    let session = outline_session();
    let (code, stdout, stderr) = run_outline(&session, &["src/a.rs"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(
        stdout.contains("one"),
        "human view names symbols: {stdout:?}"
    );
    assert!(
        stdout.contains("two"),
        "human view names symbols: {stdout:?}"
    );
    assert!(
        stdout.contains("function"),
        "human view shows the stored kind vocabulary: {stdout:?}"
    );
}

#[test]
fn outline_unindexed_path_refuses_loudly() {
    let session = outline_session();
    let (code, stdout, stderr) = run_outline(&session, &["c.txt", "--json"]);
    assert_eq!(
        code, 2,
        "unindexed/unsupported path must fail closed, not silent-empty: stdout={stdout} stderr={stderr}"
    );
    let value: Value = serde_json::from_str(&stdout).expect("machine JSON error envelope");
    assert_eq!(value["ok"], false);
    let message = value["error"]["message"].as_str().expect("error message");
    assert!(
        message.contains("c.txt") && message.contains("index"),
        "error must name the path and point at indexing: {message}"
    );
}

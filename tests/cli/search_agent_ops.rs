//! Agent/MCP search contract: read-only, abort, --path alias, schema channel.
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

fn run_json(args: &[&str]) -> (i32, Value, String) {
    let output = Command::new(asgrep_bin())
        .args(args)
        .env("NO_COLOR", "1")
        .output()
        .expect("run asgrep");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!("stdout is not JSON: {error}\nstdout: {stdout}\nstderr: {stderr}")
    });
    (output.status.code().expect("exit code"), value, stderr)
}

fn index_fixture(root: &Path, index: &Path) {
    fs::create_dir_all(root.join("pkg")).expect("pkg");
    fs::write(
        root.join("pkg/auth.rs"),
        "pub fn refresh_token() {}\npub fn process_request() { refresh_token(); }\n",
    )
    .expect("auth.rs");
    fs::write(root.join("other.rs"), "pub fn unrelated() {}\n").expect("other.rs");
    let (code, value, stderr) = run_json(&[
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "index",
        root.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "index failed stderr={stderr} value={value}");
}

fn wal_len(index: &Path) -> u64 {
    let wal = index.with_file_name(format!(
        "{}-wal",
        index.file_name().unwrap().to_string_lossy()
    ));
    fs::metadata(&wal).map(|m| m.len()).unwrap_or(0)
}

#[test]
fn version_prints_index_schema_channel() {
    let (code, value, stderr) = run_json(&["version", "--json"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(value["ok"], true);
    assert_eq!(
        value["index_schema_version"],
        ast_sgrep_core::INDEX_SCHEMA_VERSION
    );
    let human = Command::new(asgrep_bin())
        .args(["version"])
        .env("NO_COLOR", "1")
        .output()
        .expect("version");
    let stdout = String::from_utf8_lossy(&human.stdout);
    assert!(
        stdout.contains("index_schema"),
        "human version must name index schema: {stdout}"
    );
}

#[test]
fn search_is_read_only_and_does_not_grow_wal() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("src");
    let index = temp.path().join("index.db");
    index_fixture(&root, &index);
    let before = wal_len(&index);
    let (code, value, stderr) = run_json(&[
        "--json",
        "--no-embed",
        "--no-auto-index",
        "--limit",
        "3",
        "--index-path",
        index.to_str().unwrap(),
        "search",
        "refresh_token",
        root.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert_eq!(value["ok"], true);
    assert!(!value["hits"].as_array().unwrap().is_empty());
    let after = wal_len(&index);
    assert!(
        after <= before,
        "search must not grow WAL: before={before} after={after}"
    );
}

#[test]
fn search_accepts_path_as_file_filter_alias() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("src");
    let index = temp.path().join("index.db");
    index_fixture(&root, &index);
    let (code, value, stderr) = run_json(&[
        "--json",
        "--no-embed",
        "--limit",
        "8",
        "--index-path",
        index.to_str().unwrap(),
        "--path",
        "pkg/**",
        "refresh_token",
        root.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert_eq!(value["ok"], true);
    let hits = value["hits"].as_array().unwrap();
    assert!(!hits.is_empty(), "{value}");
    assert!(
        hits.iter().all(|hit| hit["file"]
            .as_str()
            .is_some_and(|file| file.starts_with("pkg/"))),
        "hits escaped --path filter: {hits:?}"
    );
}

#[test]
fn subcommand_format_alone_selects_machine_success_and_failure() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("src");
    let index = temp.path().join("index.db");
    index_fixture(&root, &index);
    let missing = temp.path().join("missing");
    for command in ["search", "keyword", "semantic"] {
        for (target, expected_code) in [(&root, 0), (&missing, 2)] {
            let (code, value, stderr) = run_json(&[
                "--no-embed",
                "--index-path",
                index.to_str().unwrap(),
                command,
                "refresh_token",
                target.to_str().unwrap(),
                "--format",
                "compact",
            ]);
            assert_eq!(code, expected_code, "{command}: {value} {stderr}");
            assert_eq!(value["command"], command);
            assert_eq!(value["ok"], expected_code == 0);
            assert!(
                stderr.is_empty(),
                "machine diagnostics belong on stdout: {stderr}"
            );
            if expected_code == 0 {
                assert!(value["h"].is_array(), "compact output required: {value}");
                if command != "semantic" {
                    assert!(!value["h"].as_array().unwrap().is_empty(), "{value}");
                }
            } else {
                assert_eq!(value["error"]["kind"], "operational");
            }
        }
    }
}

#[test]
fn root_alias_survives_graph_reader_and_auto_index_boundaries() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("src");
    let index = temp.path().join("index.db");
    index_fixture(&root, &index);
    for tail in [
        vec!["chain", "refresh_token"],
        vec!["call-path", "process_request", "refresh_token"],
        vec!["--auto-index", "search", "refresh_token"],
    ] {
        let mut args = vec![
            "--json",
            "--no-embed",
            "--index-path",
            index.to_str().unwrap(),
            "--root",
            root.to_str().unwrap(),
        ];
        args.extend(tail.iter().copied());
        let (code, value, stderr) = run_json(&args);
        assert_eq!(
            code, 0,
            "one --root is not ambiguous: {args:?} {value} {stderr}"
        );
        match value["command"].as_str().unwrap() {
            "chain" => assert!(value["node_count"].as_u64().unwrap() > 0, "{value}"),
            "call-path" => assert_eq!(value["found"], true, "{value}"),
            "search" => assert!(!value["hits"].as_array().unwrap().is_empty(), "{value}"),
            other => panic!("unexpected command {other}"),
        }
        args.push(root.to_str().unwrap());
        let (code, value, _) = run_json(&args);
        assert_eq!(
            code, 1,
            "an actual positional ROOT conflict must still fail: {value}"
        );
        assert_eq!(value["error"]["kind"], "usage");
    }
}

#[test]
fn index_path_alias_is_not_changed_by_a_root_named_search() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("search");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("lib.rs"), "fn rooted_search() {}\n").unwrap();
    let output = Command::new(asgrep_bin())
        .current_dir(temp.path())
        .args([
            "--json",
            "--no-embed",
            "index",
            "search",
            "--path",
            "lib.rs",
        ])
        .output()
        .unwrap();
    let value = ast_sgrep_testkit::assert_success(&output, "index");
    assert_eq!(value["targeted"], true, "{value}");
    assert_eq!(value["stats"]["files_indexed"], 1, "{value}");
}

#[test]
fn short_json_flag_applies_to_usage_errors() {
    let (code, value, stderr) = run_json(&["search", "needle", "-j", "--limit", "not-a-number"]);
    assert_eq!(code, 1);
    assert_eq!(value["command"], "search");
    assert_eq!(value["error"]["kind"], "usage");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn concurrent_searches_all_succeed() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("src");
    let index = temp.path().join("index.db");
    index_fixture(&root, &index);
    let index_s = index.to_str().unwrap().to_owned();
    let root_s = root.to_str().unwrap().to_owned();
    let bin = asgrep_bin();
    let children: Vec<std::process::Child> = (0..4)
        .map(|_| {
            Command::new(&bin)
                .args([
                    "--json",
                    "--no-embed",
                    "--no-auto-index",
                    "--limit",
                    "3",
                    "--index-path",
                    &index_s,
                    "search",
                    "refresh_token",
                    &root_s,
                ])
                .env("NO_COLOR", "1")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn search")
        })
        .collect();
    for (i, child) in children.into_iter().enumerate() {
        let output = child.wait_with_output().expect("wait search");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "search {i} failed code={:?} stdout={stdout} stderr={stderr}",
            output.status.code()
        );
        let value: Value = serde_json::from_slice(&output.stdout).expect("json");
        assert_eq!(value["ok"], true, "search {i}: {value}");
        assert_ne!(value["error"]["kind"], "index_open");
    }
}

#[test]
fn index_path_is_not_rewritten_to_file_filter() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("src");
    let index = temp.path().join("index.db");
    index_fixture(&root, &index);
    let (code, value, stderr) = run_json(&[
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "index",
        root.to_str().unwrap(),
        "--path",
        "pkg/auth.rs",
    ]);
    assert_eq!(
        code, 0,
        "index --path must remain an index flag; stderr={stderr} value={value}"
    );
    assert_eq!(value["ok"], true);
}

#[test]
fn doctor_names_on_disk_vs_binary_schema() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("src");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("lib.rs"), "pub fn x() {}\n").unwrap();
    let index = temp.path().join("index.db");
    {
        let store = ast_sgrep_core::IndexStore::open(&root, Some(&index)).expect("open writer");
        store
            .connection()
            .execute_batch("PRAGMA user_version=99;")
            .expect("force newer user_version");
    }
    let (code, value, stderr) = run_json(&[
        "--json",
        "--index-path",
        index.to_str().unwrap(),
        "doctor",
        root.to_str().unwrap(),
    ]);
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    assert_eq!(value["ok"], false);
    let issues = value["issues"].as_array().expect("issues");
    let mismatch = issues
        .iter()
        .find(|issue| issue["kind"] == "schema_mismatch")
        .expect("schema_mismatch issue");
    assert_eq!(mismatch["on_disk"], 99);
    assert_eq!(mismatch["supported"], ast_sgrep_core::INDEX_SCHEMA_VERSION);
    let suggested = value["suggested_commands"]
        .as_array()
        .expect("suggested_commands");
    assert!(
        suggested.iter().any(|cmd| cmd
            .as_str()
            .is_some_and(|s| s.contains("asgrep version --json"))),
        "newer-on-disk recovery must name install/version; got {suggested:?}"
    );
}

#[test]
fn doctor_older_schema_suggests_reindex() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("src");
    let index = temp.path().join("index.db");
    index_fixture(&root, &index);
    {
        let store = ast_sgrep_core::IndexStore::open(&root, Some(&index)).expect("open writer");
        store
            .connection()
            .execute_batch("PRAGMA user_version=1;")
            .expect("force older user_version");
    }
    let (code, value, stderr) = run_json(&[
        "--json",
        "--index-path",
        index.to_str().unwrap(),
        "doctor",
        root.to_str().unwrap(),
    ]);
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    let issues = value["issues"].as_array().expect("issues");
    let mismatch = issues
        .iter()
        .find(|issue| issue["kind"] == "schema_mismatch")
        .expect("schema_mismatch issue");
    assert_eq!(mismatch["on_disk"], 1);
    assert_eq!(mismatch["supported"], ast_sgrep_core::INDEX_SCHEMA_VERSION);
    let suggested = value["suggested_commands"]
        .as_array()
        .expect("suggested_commands");
    assert!(
        suggested
            .iter()
            .any(|cmd| cmd.as_str().is_some_and(|s| s.contains("asgrep reindex"))),
        "older-on-disk recovery must name reindex; got {suggested:?}"
    );
}

#[cfg(unix)]
#[test]
fn sigterm_releases_lock_within_200ms() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("src");
    let index = temp.path().join("index.db");
    index_fixture(&root, &index);
    let mut child = Command::new(asgrep_bin())
        .args([
            "--json",
            "--no-embed",
            "--no-auto-index",
            "--limit",
            "3",
            "--index-path",
            index.to_str().unwrap(),
            "search",
            "refresh_token",
            root.to_str().unwrap(),
        ])
        .env("NO_COLOR", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn search");
    let pid = child.id();
    std::thread::sleep(Duration::from_millis(50));
    let _ = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status();
    let start = Instant::now();
    loop {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_millis(200),
            "asgrep pid {pid} still alive after SIGTERM"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let (code, value, stderr) = run_json(&[
        "--json",
        "--no-embed",
        "--no-auto-index",
        "--limit",
        "3",
        "--index-path",
        index.to_str().unwrap(),
        "search",
        "refresh_token",
        root.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "follow-up search stderr={stderr} value={value}");
    assert_eq!(value["ok"], true);
}

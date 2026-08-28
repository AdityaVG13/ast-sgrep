use ast_sgrep_testkit::CliSession;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;
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
    assert!(
        status.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&status.stdout),
        String::from_utf8_lossy(&status.stderr)
    );
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

    let compact = session.run_success(&[
        "--index-path",
        session.index_path.to_str().unwrap(),
        "--json",
        "--no-embed",
        "--format",
        "compact",
        "--snippet-tokens",
        "8",
        "--response-snippet-tokens",
        "10",
        "--",
        "process_request",
        session.root.to_str().unwrap(),
    ]);
    assert_eq!(
        compact.stdout.iter().filter(|byte| **byte == b'\n').count(),
        1,
        "compact output has no pretty-print decoration"
    );
    let compact: serde_json::Value = serde_json::from_slice(&compact.stdout).unwrap();
    assert_eq!(compact["zb"][0], 8);
    assert_eq!(compact["zb"][1], 10);
    assert!(compact["zb"][2].as_u64().unwrap() <= 10);
    assert!(!compact["h"].as_array().unwrap().is_empty());
    assert!(compact["p"].is_object());

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

fn run_json(args: &[&str]) -> (i32, Value, String, String) {
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
    (
        output.status.code().expect("exit code"),
        value,
        stdout,
        stderr,
    )
}

#[test]
fn search_does_not_auto_index_an_empty_checkout() {
    let root = TempDir::new().expect("root");
    fs::write(root.path().join("planted.rs"), "fn planted_symbol() {}\n").expect("source");
    let index = root.path().join("index.db");
    let (code, value, _stdout, stderr) = run_json(&[
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "search",
        "planted_symbol",
        root.path().to_str().unwrap(),
    ]);
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    assert!(stderr.is_empty(), "machine mode must stay silent: {stderr}");
    assert_eq!(value["ok"], false);
    let message = value["error"]["message"].as_str().unwrap_or("");
    assert!(
        message.contains("index is empty"),
        "expected empty-index error, got {message}"
    );
}

#[test]
fn search_auto_index_opt_in_indexes_an_empty_checkout() {
    let root = TempDir::new().expect("root");
    fs::write(root.path().join("planted.rs"), "fn planted_symbol() {}\n").expect("source");
    let index = root.path().join("index.db");
    let (code, value, _stdout, stderr) = run_json(&[
        "--auto-index",
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "search",
        "planted_symbol",
        root.path().to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert!(stderr.is_empty(), "machine mode must stay silent: {stderr}");
    assert_eq!(value["ok"], true);
    let hits = value["hits"].as_array().expect("hits");
    assert!(
        hits.iter()
            .any(|hit| { hit["symbol"] == "planted_symbol" || hit["file"] == "planted.rs" }),
        "expected planted_symbol hit, got {hits:?}"
    );
}

#[test]
fn search_no_auto_index_fails_closed_when_empty() {
    let root = TempDir::new().expect("root");
    fs::write(root.path().join("planted.rs"), "fn planted_symbol() {}\n").expect("source");
    let index = root.path().join("index.db");
    let (code, value, _stdout, stderr) = run_json(&[
        "--no-auto-index",
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "search",
        "planted_symbol",
        root.path().to_str().unwrap(),
    ]);
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    assert!(stderr.is_empty(), "machine mode must stay silent: {stderr}");
    assert_eq!(value["ok"], false);
    let message = value["error"]["message"].as_str().unwrap_or("");
    assert!(
        message.contains("index is empty"),
        "expected empty-index error, got {message}"
    );
}

#[test]
fn search_does_not_refresh_stale_index_after_edit() {
    let root = TempDir::new().expect("root");
    let planted = root.path().join("planted.rs");
    fs::write(&planted, "fn planted_symbol() {}\n").expect("source");
    let index = root.path().join("index.db");
    let (code, value, _stdout, stderr) = run_json(&[
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "index",
        root.path().to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr={stderr} value={value}");

    fs::write(
        &planted,
        "fn planted_symbol() {}\nfn planted_after_edit() {}\n",
    )
    .expect("edit");
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    fs::File::options()
        .write(true)
        .open(&planted)
        .expect("open planted")
        .set_modified(later)
        .expect("bump mtime");

    let (code, value, _stdout, stderr) = run_json(&[
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "search",
        "word:planted_after_edit",
        root.path().to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert!(stderr.is_empty(), "machine mode must stay silent: {stderr}");
    assert_eq!(value["ok"], true);
    let hits = value["hits"].as_array().expect("hits");
    assert!(hits.is_empty(), "search must not refresh; got {hits:?}");
}

#[test]
fn search_no_auto_index_skips_refresh_after_edit() {
    let root = TempDir::new().expect("root");
    let planted = root.path().join("planted.rs");
    fs::write(&planted, "fn planted_symbol() {}\n").expect("source");
    let index = root.path().join("index.db");
    let (code, value, _stdout, stderr) = run_json(&[
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "index",
        root.path().to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr={stderr} value={value}");

    fs::write(&planted, "fn planted_symbol() {}\nfn planted_frozen() {}\n").expect("edit");
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    fs::File::options()
        .write(true)
        .open(&planted)
        .expect("open planted")
        .set_modified(later)
        .expect("bump mtime");

    let (code, value, _stdout, stderr) = run_json(&[
        "--no-auto-index",
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "search",
        "word:planted_frozen",
        root.path().to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert!(stderr.is_empty(), "machine mode must stay silent: {stderr}");
    assert_eq!(value["ok"], true);
    let hits = value["hits"].as_array().expect("hits");
    assert!(
        hits.is_empty(),
        "--no-auto-index must not refresh; got {hits:?}"
    );
}

#[test]
fn chain_does_not_auto_index_an_empty_checkout() {
    let root = TempDir::new().expect("root");
    fs::write(
        root.path().join("planted.rs"),
        "fn planted_caller() { planted_symbol(); }\nfn planted_symbol() {}\n",
    )
    .expect("source");
    let index = root.path().join("index.db");
    let (code, value, _stdout, stderr) = run_json(&[
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "chain",
        "planted_symbol",
        root.path().to_str().unwrap(),
    ]);
    assert_eq!(code, 2, "stderr={stderr} value={value}");
    assert!(stderr.is_empty(), "machine mode must stay silent: {stderr}");
    assert_eq!(value["ok"], false);
    let message = value["error"]["message"].as_str().unwrap_or("");
    assert!(
        message.contains("index is empty"),
        "expected empty-index error, got {message}"
    );
}

#[test]
fn call_path_runs_against_the_real_indexed_fixture() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    fs::create_dir(&root).unwrap();
    fs::write(
        root.join("main.rs"),
        "fn main() { process_request(); }\n\
         fn process_request() { validate_input(); }\n\
         fn validate_input() {}\n",
    )
    .unwrap();
    let session = CliSession {
        index_path: temp.path().join("index.db"),
        bin: asgrep_bin(),
        root,
        _temp: temp,
    };
    session.run_success(&[
        "--index-path",
        session.index_path.to_str().unwrap(),
        "index",
        "--no-embed",
        session.root.to_str().unwrap(),
    ]);
    let output = session.run_success(&[
        "--index-path",
        session.index_path.to_str().unwrap(),
        "--json",
        "call-path",
        "main",
        "validate_input",
        session.root.to_str().unwrap(),
    ]);
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["command"], "call-path");
    assert_eq!(response["found"], true);
    assert_eq!(response["semantics"], "call_graph_only");
    assert_eq!(response["depth"], 2);
    assert_eq!(response["path"][0]["caller"], "main");
    assert_eq!(response["path"][1]["callee"], "validate_input");
}

#[test]
fn conceptual_query_fans_out_through_the_real_cli() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    fs::create_dir(&root).unwrap();
    fs::write(
        root.join("cookie.rs"),
        "/// Write the session cookie after authentication succeeds.\n\
         pub fn commit_auth_state() {\n\
             let _cookie = \"session cookie\";\n\
         }\n",
    )
    .unwrap();
    fs::write(
        root.join("login.rs"),
        "pub fn complete_login() {\n\
             commit_auth_state();\n\
         }\n",
    )
    .unwrap();
    let session = CliSession {
        index_path: temp.path().join("index.db"),
        bin: asgrep_bin(),
        root,
        _temp: temp,
    };
    session.run_success(&[
        "--index-path",
        session.index_path.to_str().unwrap(),
        "index",
        session.root.to_str().unwrap(),
    ]);

    let response = session.search_json(
        "all functions that write the session cookie",
        &["--limit", "32"],
    );
    let path = response["hits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|hit| hit["file"] == "login.rs" && hit["callee"] == "commit_auth_state")
        .expect("real CLI search must return the indexed caller path");
    let contributors = path["contributors"].as_array().unwrap();
    for channel in ["caller", "graph", "pattern"] {
        assert!(
            contributors.iter().any(|kind| kind == channel),
            "missing {channel} evidence in {path}"
        );
    }
}

#[test]
fn repository_vocabulary_closes_a_real_cli_lexical_gap() {
    let temp = TempDir::new().unwrap();
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/fixtures/native_semantic");
    let session = CliSession {
        index_path: temp.path().join("index.db"),
        bin: asgrep_bin(),
        root,
        _temp: temp,
    };
    session.run_success(&[
        "--index-path",
        session.index_path.to_str().unwrap(),
        "index",
        session.root.to_str().unwrap(),
    ]);

    let response = session.search_json("renewal", &["--limit", "5"]);
    assert!(
        response["hits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hit| { hit["file"] == "targets.rs" && hit["symbol"] == "rotate_live_token" }),
        "repository-learned vocabulary must recover the judged target: {response}"
    );
    assert!(response["query_expansions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|expansion| expansion["term"] == "renewal" && expansion["related"] == "rotate"));
}

#[test]
fn codemod_dry_run_and_apply_use_the_real_indexed_fixture() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    fs::create_dir(&root).unwrap();
    let first = root.join("first.rs");
    let second = root.join("second.rs");
    fs::write(&first, "fn first() { legacy(alpha); }\n").unwrap();
    fs::write(&second, "fn second() { legacy(beta); }\n").unwrap();
    let session = CliSession {
        index_path: temp.path().join("index.db"),
        bin: asgrep_bin(),
        root,
        _temp: temp,
    };
    session.run_success(&[
        "--index-path",
        session.index_path.to_str().unwrap(),
        "index",
        "--no-embed",
        session.root.to_str().unwrap(),
    ]);

    let dry_run = session.run_success(&[
        "--index-path",
        session.index_path.to_str().unwrap(),
        "codemod",
        "--no-embed",
        "--dry-run",
        "--pattern",
        "legacy($ARG)",
        "--rewrite",
        "modern($ARG)",
        session.root.to_str().unwrap(),
    ]);
    let dry_run: serde_json::Value = serde_json::from_slice(&dry_run.stdout).unwrap();
    assert_eq!(dry_run["command"], "codemod");
    assert_eq!(dry_run["dry_run"], true);
    assert_eq!(dry_run["plan"]["files_changed"], 2);
    assert_eq!(dry_run["plan"]["edit_count"], 2);
    assert_eq!(
        fs::read_to_string(&first).unwrap(),
        "fn first() { legacy(alpha); }\n"
    );
    assert_eq!(
        fs::read_to_string(&second).unwrap(),
        "fn second() { legacy(beta); }\n"
    );

    let applied = session.run_success(&[
        "--index-path",
        session.index_path.to_str().unwrap(),
        "--json",
        "codemod",
        "--yes",
        "--no-embed",
        "--pattern",
        "legacy($ARG)",
        "--rewrite",
        "modern($ARG)",
        session.root.to_str().unwrap(),
    ]);
    let applied: serde_json::Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(applied["files_changed"], 2);
    assert_eq!(applied["edits_applied"], 2);
    assert_eq!(
        fs::read_to_string(&first).unwrap(),
        "fn first() { modern(alpha); }\n"
    );
    assert_eq!(
        fs::read_to_string(&second).unwrap(),
        "fn second() { modern(beta); }\n"
    );

    let search = session.search_json("modern", &["--no-embed", "--limit", "20"]);
    let hit_files = search["hits"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|hit| hit["file"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        hit_files,
        std::collections::BTreeSet::from(["first.rs", "second.rs"])
    );
}

#[cfg(unix)]
#[test]
fn codemod_apply_refuses_parent_symlink_swap() {
    use ast_sgrep_core::codemod::{apply_codemod, plan_codemod};
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fixture");
    let source_dir = root.join("src");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("lib.rs"), "fn run() { legacy(alpha); }\n").unwrap();
    let session = CliSession {
        index_path: temp.path().join("index.db"),
        bin: asgrep_bin(),
        root,
        _temp: temp,
    };
    session.run_success(&[
        "--index-path",
        session.index_path.to_str().unwrap(),
        "index",
        "--no-embed",
        session.root.to_str().unwrap(),
    ]);
    let plan = plan_codemod(
        &session.root,
        Some(&session.index_path),
        "legacy($ARG)",
        "modern($ARG)",
    )
    .unwrap();

    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join("lib.rs");
    let original = "fn run() { legacy(alpha); }\n";
    fs::write(&outside_file, original).unwrap();
    fs::rename(&source_dir, session.root.join("saved-src")).unwrap();
    symlink(outside.path(), &source_dir).unwrap();

    let error = apply_codemod(&plan).expect_err("symlink escape must be rejected");
    assert!(error.to_string().contains("failed to verify"), "{error:#}");
    assert_eq!(fs::read_to_string(outside_file).unwrap(), original);
}

#[test]
fn search_file_filter_reuses_one_repository_index() {
    let root = TempDir::new().expect("root");
    fs::create_dir_all(root.path().join("a")).unwrap();
    fs::create_dir_all(root.path().join("b")).unwrap();
    fs::write(root.path().join("a/one.rs"), "fn shared_symbol() {}\n").unwrap();
    fs::write(root.path().join("b/two.rs"), "fn shared_symbol() {}\n").unwrap();
    fs::write(
        root.path().join("README.md"),
        "# untyped indexed document\n",
    )
    .unwrap();
    let index = root.path().join(".asgrep/index.db");
    let (code, value, _stdout, stderr) = run_json(&[
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "index",
        root.path().to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "index stderr={stderr} value={value}");

    let (code, value, _stdout, stderr) = run_json(&[
        "--json",
        "--no-embed",
        "--index-path",
        index.to_str().unwrap(),
        "search",
        "--file-filter",
        "a/**",
        "shared_symbol",
        root.path().to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    let hits = value["hits"].as_array().expect("hits");
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|hit| {
        hit["file"]
            .as_str()
            .is_some_and(|file| file.starts_with("a/"))
    }));
    assert!(index.is_file());
    assert!(!root.path().join("a/.asgrep/index.db").exists());
    assert!(!root.path().join("b/.asgrep/index.db").exists());

    let (code, value, _stdout, stderr) = run_json(&[
        "--json",
        "--no-embed",
        "--lang",
        "rust",
        "--index-path",
        index.to_str().unwrap(),
        "search",
        "--file-filter",
        "a/**",
        "shared_symbol",
        root.path().to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "stderr={stderr} value={value}");
    assert_eq!(value["ok"], true);
}

#[test]
fn file_filter_is_rejected_by_non_search_commands() {
    let root = TempDir::new().expect("root");
    let (code, value, _stdout, stderr) = run_json(&[
        "--json",
        "index",
        "--file-filter",
        "src/**",
        root.path().to_str().unwrap(),
    ]);
    assert_eq!(code, 1, "stderr={stderr} value={value}");
    assert!(stderr.is_empty());
    assert!(value["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("--file-filter applies only")));
}

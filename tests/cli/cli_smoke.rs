use ast_sgrep_testkit::CliSession;
use std::fs;
use std::path::PathBuf;
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

#[test]
fn call_path_runs_against_the_real_indexed_fixture() {
    let session = CliSession::sample(asgrep_bin());
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

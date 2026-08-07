use ast_sgrep_testkit::CliSession;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;
fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}
fn run(bin: &Path, args: &[&str]) -> Output {
    Command::new(bin)
        .args(args)
        .env("NO_COLOR", "1")
        .output()
        .expect("run asgrep")
}
fn parse_stdout(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not one standalone JSON value: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}
fn assert_success(output: &Output, command: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "unexpected success diagnostic: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_stdout(output);
    assert_eq!(value["schema_version"], "1.0.0");
    assert_eq!(value["tool"], "asgrep");
    assert_eq!(value["command"], command);
    assert_eq!(value["ok"], true);
    assert_eq!(value["exit_code"], 0);
    value
}
fn assert_doctor_unhealthy(output: &Output) -> Value {
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "unexpected diagnostic: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_stdout(output);
    assert_eq!(value["schema_version"], "1.0.0");
    assert_eq!(value["tool"], "asgrep");
    assert_eq!(value["command"], "doctor");
    assert_eq!(value["ok"], false);
    assert_eq!(value["exit_code"], 2);
    assert_eq!(value["healthy"], false);
    value
}
fn fixture(name: &str) -> Value {
    let raw = match name {
        "capabilities" => include_str!("fixtures/capabilities.json"),
        "shapes" => include_str!("fixtures/machine_shapes.json"),
        "envelopes" => include_str!("fixtures/envelopes.json"),
        _ => panic!("unknown fixture {name}"),
    };
    serde_json::from_str(raw).expect("valid JSON fixture")
}
fn assert_shape(value: &Value, shape: &Value) {
    let mut actual: Vec<_> = value
        .as_object()
        .expect("JSON object")
        .keys()
        .cloned()
        .collect();
    actual.sort();
    let expected: Vec<_> = shape
        .as_array()
        .expect("key array")
        .iter()
        .map(|key| key.as_str().expect("string key").to_owned())
        .collect();
    assert_eq!(actual, expected);
}
#[test]
fn capabilities_and_version_match_goldens() {
    let bin = asgrep_bin();
    let mut capabilities = assert_success(&run(&bin, &["capabilities", "--json"]), "capabilities");
    capabilities["version"] = "<version>".into();
    assert_eq!(capabilities, fixture("capabilities"));
    let mut version = assert_success(&run(&bin, &["version", "--json"]), "version");
    version["version"] = "<version>".into();
    assert_eq!(version, fixture("envelopes")["version"]);
}
#[test]
fn index_reindex_status_and_doctor_have_stable_shapes() {
    let session = CliSession::sample(asgrep_bin());
    let index = session.index_path.to_str().expect("index utf8");
    let root = session.root.to_str().expect("root utf8");
    let shapes = fixture("shapes");
    for command in ["index", "reindex"] {
        assert_shape(
            &assert_success(
                &run(
                    &session.bin,
                    &["--json", "--no-embed", "--index-path", index, command, root],
                ),
                command,
            ),
            &shapes["index"],
        );
    }
    assert_shape(
        &assert_success(
            &run(
                &session.bin,
                &[
                    "--json",
                    "--no-embed",
                    "--index-path",
                    index,
                    "status",
                    root,
                ],
            ),
            "status",
        ),
        &shapes["status"],
    );
    let blocked = TempDir::new().expect("tempdir");
    let blocked_index = blocked.path().join("blocked.db");
    std::fs::create_dir(&blocked_index).expect("blocking directory");
    let blocked_index = blocked_index.to_str().expect("blocked path utf8");
    let doctor = assert_doctor_unhealthy(&run(
        &session.bin,
        &["--json", "--index-path", blocked_index, "doctor", root],
    ));
    assert_shape(&doctor, &shapes["doctor"]);
    assert_eq!(doctor["healthy"], false);
    assert_eq!(doctor["status"], Value::Null);
    assert!(!doctor["issues"].as_array().expect("issues").is_empty());
}
#[test]
fn agent_search_modes_are_stable_and_bounded() {
    let session = CliSession::sample(asgrep_bin());
    let shapes = fixture("shapes");
    let agent = session.search_json(
        "process_request",
        &["--no-embed", "--limit", "2", "--format", "agent"],
    );
    assert_shape(&agent, &shapes["agent"]);
    assert_eq!(agent["command"], "search");
    assert_eq!(agent["ok"], true);
    assert!(agent["hits"].as_array().expect("agent hits").len() <= 2);
    let capsule = session.search_json(
        "process_request",
        &[
            "--no-embed",
            "--limit",
            "2",
            "--format",
            "agent-capsule",
            "--excerpt-lines",
            "2",
        ],
    );
    assert_shape(&capsule, &shapes["agent-capsule"]);
    let hits = capsule["hits"].as_array().expect("capsule hits");
    assert!(hits.len() <= 2);
    for hit in hits {
        assert!(hit["preview"].as_str().expect("preview").chars().count() <= 121);
        assert!(hit["excerpt"].as_str().expect("excerpt").lines().count() <= 2);
    }
    let compact = session.search_json(
        "process_request",
        &[
            "--no-embed",
            "--limit",
            "2",
            "--format",
            "compact",
            "--snippet-tokens",
            "12",
            "--response-snippet-tokens",
            "16",
        ],
    );
    assert_shape(&compact, &shapes["compact"]);
    assert!(compact["h"].as_array().expect("compact hits").len() <= 2);
    assert!(compact["p"].is_object());
    assert_eq!(compact["zb"][0], 12);
    assert_eq!(compact["zb"][1], 16);
    assert!(compact["zb"][2].as_u64().expect("used budget") <= 16);
}
/// Embed-default-ON machine contract (mock-free e2e gap lbx1.4).
///
/// Production default is embed-on; most CLI tests pass `--no-embed`. This
/// contract indexes the sample fixture with hashed semantic (CLI default) and
/// searches **without** `--no-embed`, asserting:
/// - index status exposes embed backend + semantic chunks
/// - agent hybrid search surfaces semantic/embed signal
/// - `asgrep semantic` returns embed-kind hits
///
/// A suite that only runs with `--no-embed` must not satisfy this bead.
#[test]
fn agent_search_embed_default_on_surfaces_semantic_hits() {
    let session = CliSession::sample(asgrep_bin());
    let index = session.index_path.to_str().expect("index utf8");
    let root = session.root.to_str().expect("root utf8");

    // Status after default index (no --no-embed on index path).
    let status = assert_success(
        &run(
            &session.bin,
            &["--json", "--index-path", index, "status", root],
        ),
        "status",
    );
    let chunk_count = status["semantic_chunk_count"].as_u64().unwrap_or(0);
    assert!(
        chunk_count > 0,
        "embed-on index must store semantic chunks; status={status}"
    );
    let backend = status["embed_backend"].as_str().unwrap_or("");
    assert!(
        !backend.is_empty(),
        "status.embed_backend must be set after semantic index; status={status}"
    );

    // Hybrid agent search WITHOUT --no-embed (production default channel).
    let agent = session.search_json(
        "credential renewal",
        &["--limit", "16", "--format", "agent"],
    );
    assert_eq!(agent["ok"], true);
    assert_eq!(agent["command"], "search");
    assert_eq!(agent["provider"], "ast-sgrep");
    let hits = agent["hits"].as_array().expect("agent hits");
    assert!(
        !hits.is_empty(),
        "embed-on hybrid agent search must return hits; agent={agent}"
    );
    let has_semantic_flag = agent["has_semantic_hits"].as_bool().unwrap_or(false);
    let has_embed_kind = hits
        .iter()
        .any(|h| h["kind"].as_str() == Some("embed"));
    let has_semantic_contrib = hits.iter().any(|h| h.get("semantic") == Some(&Value::Bool(true)));
    assert!(
        has_semantic_flag || has_embed_kind || has_semantic_contrib,
        "embed-on agent JSON must surface semantic/embed path          (has_semantic_hits / kind=embed / hit.semantic);          has_semantic_hits={has_semantic_flag} hits={hits:?}"
    );

    // Pure semantic subcommand path — all hits must be embed-kind.
    let semantic_out = session.run_success(&[
        "--index-path",
        index,
        "--json",
        "--format",
        "agent",
        "--limit",
        "16",
        "semantic",
        "--",
        "credential renewal",
        root,
    ]);
    let semantic: Value =
        serde_json::from_slice(&semantic_out.stdout).expect("semantic agent json");
    assert_eq!(semantic["ok"], true);
    assert_eq!(semantic["command"], "semantic");
    let semantic_hits = semantic["hits"].as_array().expect("semantic hits");
    assert!(
        !semantic_hits.is_empty(),
        "semantic CLI must return embed hits after hashed index; semantic={semantic}"
    );
    assert!(
        semantic_hits
            .iter()
            .any(|h| h["kind"].as_str() == Some("embed")),
        "semantic CLI hits must include kind=embed; hits={semantic_hits:?}"
    );
    // Soft-skip empty embed is forbidden: hard-require auth_refresh relevance.
    assert!(
        semantic_hits.iter().any(|h| {
            h["symbol"].as_str() == Some("auth_refresh")
                || h["preview"]
                    .as_str()
                    .map(|p| p.contains("auth_refresh"))
                    .unwrap_or(false)
                || h.get("excerpt")
                    .and_then(|e| e.as_str())
                    .map(|e| e.contains("auth_refresh"))
                    .unwrap_or(false)
        }),
        "semantic embed path must surface auth_refresh; hits={semantic_hits:?}"
    );
}


#[test]
fn chain_eval_and_bench_successes_use_machine_envelope() {
    let session = CliSession::sample(asgrep_bin());
    let index = session.index_path.to_str().expect("index utf8");
    let root = session.root.to_str().expect("root utf8");
    let chain = assert_success(
        &run(
            &session.bin,
            &[
                "--json",
                "--no-embed",
                "--index-path",
                index,
                "chain",
                "process_request",
                root,
            ],
        ),
        "chain",
    );
    assert!(chain["nodes"].is_array());
    let bench = assert_success(
        &run(
            &session.bin,
            &[
                "--json",
                "--no-embed",
                "--index-path",
                index,
                "bench",
                root,
                "--query",
                "process_request",
                "--iterations",
                "1",
                "--skip-index",
            ],
        ),
        "bench",
    );
    assert_eq!(bench["iterations"], 1);
    let gold = session._temp.path().join("gold.json");
    std::fs::write(&gold, serde_json::json!({"corpus": "sample", "queries": [{"name": "process", "query": "process_request", "k": 5, "relevant": [{"file": "src/main.rs", "symbol": "process_request"}]}]}).to_string()).unwrap();
    let eval = assert_success(
        &run(
            &session.bin,
            &[
                "--json",
                "--no-embed",
                "--index-path",
                index,
                "eval",
                "--gold",
                gold.to_str().unwrap(),
                root,
            ],
        ),
        "eval",
    );
    assert_eq!(eval["corpus"], "sample");
}
#[test]
fn operational_failures_are_json_and_exit_two() {
    let bin = asgrep_bin();
    let temp = TempDir::new().expect("tempdir");
    let blocked_index = temp.path().join("blocked.db");
    std::fs::create_dir(&blocked_index).expect("blocking directory");
    let blocked_index = blocked_index.to_str().expect("blocked path utf8");
    let root = temp.path().to_str().expect("root utf8");
    let golden = &fixture("envelopes")["operational"];
    for (command, args) in [
        (
            "index",
            vec!["--json", "--index-path", blocked_index, "index", root],
        ),
        (
            "reindex",
            vec!["--json", "--index-path", blocked_index, "reindex", root],
        ),
        (
            "status",
            vec!["--json", "--index-path", blocked_index, "status", root],
        ),
        (
            "search",
            vec!["--json", "--index-path", blocked_index, "query", root],
        ),
    ] {
        let output = run(&bin, &args);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut value = parse_stdout(&output);
        assert_eq!(value["command"], command);
        assert_eq!(value["error"]["kind"], "operational");
        assert!(
            value["error"]["message"]
                .as_str()
                .expect("message")
                .chars()
                .count()
                <= 4_097
        );
        value["command"] = "<command>".into();
        value["error"]["message"] = "<message>".into();
        assert_eq!(&value, golden);
    }
}
#[test]
fn bounded_arguments_are_json_usage_errors() {
    let bin = asgrep_bin();
    let golden = &fixture("envelopes")["usage"];
    for args in [
        ["--json", "--limit", "1001", "query", "."],
        ["--json", "--limit", "-1", "query", "."],
        ["--json", "--excerpt-lines", "101", "query", "."],
    ] {
        let output = run(&bin, &args);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stderr.is_empty());
        let mut value = parse_stdout(&output);
        assert_eq!(value["error"]["kind"], "usage");
        value["error"]["message"] = "<message>".into();
        assert_eq!(&value, golden);
    }
}

#[test]
fn agent_discovery_defaults_and_boolish_envs_are_round_trip_free() {
    let bin = asgrep_bin();
    for value in ["1", "0", "true", "false", "yes", "no", "on", "off"] {
        let output = Command::new(&bin)
            .arg("capabilities")
            .env("ASGREP_NO_EMBED", value)
            .env("ASGREP_CLOUD_EMBED", value)
            .env("ASGREP_OLLAMA_EMBED", value)
            .env("ASGREP_NEURAL_EMBED", value)
            .env("ASGREP_SEMANTIC_ONLY", value)
            .env("ASGREP_TANTIVY", value)
            .env("ASGREP_RERANK", value)
            .env("NO_COLOR", "1")
            .output()
            .expect("run capabilities");
        assert_success(&output, "capabilities");
    }
    let output = run(&bin, &["--robot-help"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).contains("agent handbook"));
    // --json must wrap the handbook (agents parse stdout as JSON).
    let json_help = run(&bin, &["--json", "--robot-help"]);
    assert_eq!(json_help.status.code(), Some(0), "robot-help --json exit");
    let help_v: Value = serde_json::from_slice(&json_help.stdout).expect("robot-help --json envelope");
    assert_eq!(help_v["ok"], true);
    assert_eq!(help_v["command"], "robot-docs");
    assert_eq!(help_v["format"], "markdown");
    assert_eq!(help_v["topic"], "guide");
    assert!(
        help_v["body"].as_str().unwrap_or("").contains("agent handbook"),
        "body should carry markdown handbook"
    );
    let json_docs = run(&bin, &["robot-docs", "--json"]);
    assert_eq!(json_docs.status.code(), Some(0), "robot-docs --json exit");
    let docs_v: Value = serde_json::from_slice(&json_docs.stdout).expect("robot-docs --json envelope");
    assert_eq!(docs_v["command"], "robot-docs");
    assert!(docs_v["body"].as_str().unwrap_or("").contains("agent handbook"));
    let missing = TempDir::new().expect("tempdir").path().join("missing");
    let doctor = assert_doctor_unhealthy(&run(
        &bin,
        &["doctor", missing.to_str().expect("utf8")],
    ));
    assert_eq!(doctor["issues"][0]["kind"], "missing_root");
}

#[test]
fn format_aliases_typos_and_root_failures_are_unambiguous() {
    let session = CliSession::sample(asgrep_bin());
    let index = session.index_path.to_str().expect("index utf8");
    let root = session.root.to_str().expect("root utf8");
    for command in ["search", "find", "query"] {
        let output = run(
            &session.bin,
            &[
                "--no-embed",
                "--index-path",
                index,
                "--format",
                "compact",
                command,
                "process_request",
                root,
            ],
        );
        let value = assert_success(&output, "search");
        assert_eq!(value["v"], 1);
    }
    for args in [
        vec!["--json", "serach"],
        vec!["--json", "chian"],
        vec!["--json", "evall"],
        vec!["--format", "invalid", "query", "/definitely/missing"],
        vec!["--format", "compact", "status", root],
        // d2a1.12: --format must not be silently accepted on index/reindex/bench
        vec!["--format", "compact", "index", root],
        vec!["--format", "compact", "reindex", root],
        vec!["--format", "compact", "bench", root, "--query", "x"],
        vec!["--json", "--root", root, "status", root],
    ] {
        let output = run(&session.bin, &args);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stderr.is_empty());
        assert_eq!(parse_stdout(&output)["error"]["kind"], "usage");
    }
    let static_query = assert_success(
        &run(
            &session.bin,
            &[
                "--no-embed",
                "--index-path",
                index,
                "--format",
                "compact",
                "static",
                root,
            ],
        ),
        "search",
    );
    assert_eq!(static_query["q"], "static");
    let missing = session._temp.path().join("missing");
    let output = run(
        &session.bin,
        &[
            "--format",
            "compact",
            "search",
            "needle",
            missing.to_str().expect("utf8"),
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty());
    assert!(parse_stdout(&output)["error"]["message"]
        .as_str()
        .expect("message")
        .contains("project root does not exist"));
    let empty = TempDir::new().expect("tempdir");
    let output = run(
        &session.bin,
        &[
            "--json",
            "--no-embed",
            "search",
            "needle",
            empty.path().to_str().expect("utf8"),
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(parse_stdout(&output)["error"]["message"]
        .as_str()
        .expect("message")
        .contains("index is empty"));
    let chain = run(
        &session.bin,
        &[
            "--json",
            "--no-embed",
            "chain",
            "needle",
            empty.path().to_str().expect("utf8"),
        ],
    );
    assert_eq!(chain.status.code(), Some(2));
    assert!(parse_stdout(&chain)["error"]["message"]
        .as_str()
        .expect("message")
        .contains("index is empty"));
}

#[test]
fn doctor_suggested_commands_echo_effective_root() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    let bin = asgrep_bin();
    let doctor = assert_doctor_unhealthy(&run(
        &bin,
        &[
            "doctor",
            "--robot-triage",
            root.to_str().expect("utf8"),
        ],
    ));
    let root_s = root.to_str().expect("utf8");
    assert_eq!(doctor["root"], root_s);
    let suggested = doctor["suggested_commands"].as_array().expect("cmds");
    assert!(
        suggested
            .iter()
            .any(|c| c.as_str().is_some_and(|s| s.contains(root_s) && s.contains("index"))),
        "suggested_commands must echo effective root, got {suggested:?}"
    );
}

#[test]
fn format_alone_implies_json_machine_output() {
    let session = CliSession::sample(asgrep_bin());
    let index = session.index_path.to_str().expect("index utf8");
    let root = session.root.to_str().expect("root utf8");
    let output = run(
        &session.bin,
        &[
            "--no-embed",
            "--index-path",
            index,
            "--format",
            "agent",
            "process_request",
            root,
        ],
    );
    let value = assert_success(&output, "search");
    assert!(value.get("hits").is_some() || value.get("hit_count").is_some() || value.get("q").is_some() || value.get("query").is_some());
}

#[test]
fn capabilities_lists_all_clap_subcommands_and_siblings() {
    let bin = asgrep_bin();
    let caps = assert_success(&run(&bin, &["capabilities", "--json"]), "capabilities");
    let names: Vec<_> = caps["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .map(|c| c["name"].as_str().expect("name"))
        .collect();
    for required in [
        "index", "status", "reindex", "search", "bench", "watch", "keyword", "semantic",
        "chain", "capabilities", "version", "robot-docs", "doctor", "eval",
    ] {
        assert!(names.contains(&required), "missing command {required} in {names:?}");
    }
    assert!(caps["sibling_binaries"].as_array().unwrap().len() >= 2);
    assert!(caps["integrations"]["mcp"]["binary"] == "asgrep-mcp");
    assert!(caps["root_specification"]["canonical"]
        .as_str()
        .unwrap()
        .contains("positional"));
    let help = run(&bin, &["capabilities", "--help"]);
    let help_text = String::from_utf8_lossy(&help.stdout);
    assert!(
        !help_text.contains("--ann-probes") && !help_text.contains("--rerank"),
        "capabilities --help must not list search-tuning flags"
    );
    let root_help = run(&bin, &["--help"]);
    let root_text = format!(
        "{}{}",
        String::from_utf8_lossy(&root_help.stdout),
        String::from_utf8_lossy(&root_help.stderr)
    );
    assert!(
        root_text.contains("asgrep-mcp") && root_text.contains("asgrep-lsp"),
        "root --help must surface sibling binaries"
    );
}

#[test]
fn edit_distance_two_typos_are_rejected_before_search() {
    let bin = asgrep_bin();
    // distance 2 from `index`
    let output = run(&bin, &["--json", "indxx"]);
    assert_eq!(output.status.code(), Some(1));
    let value = parse_stdout(&output);
    let msg = value["error"]["message"].as_str().expect("message");
    assert!(
        msg.contains("did you mean") && msg.contains("index"),
        "expected edit-distance≤2 suggestion, got {msg}"
    );
}

#[test]
fn index_dry_run_does_not_mutate() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path().join("proj");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.rs"), "fn hello() {}\n").unwrap();
    let bin = asgrep_bin();
    let out = run(
        &bin,
        &[
            "--json",
            "index",
            "--dry-run",
            root.to_str().expect("utf8"),
        ],
    );
    let value = assert_success(&out, "index");
    assert_eq!(value["dry_run"], true);
    assert_eq!(value["mutates_index"], false);
    assert_eq!(value["walk_errors"], false);
    assert!(!root.join(".asgrep").exists() || !root.join(".asgrep/index.db").exists());
}

#[test]
fn index_dry_run_reports_walk_errors_when_read_dir_fails() {
    // d2a1.11: unreadable subdirs must not silently under-count as files_would_index: 0.
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path().join("proj");
    let blocked = root.join("blocked");
    std::fs::create_dir_all(&blocked).unwrap();
    std::fs::write(blocked.join("hidden.rs"), "fn hidden() {}\n").unwrap();
    std::fs::write(root.join("visible.rs"), "fn visible() {}\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&blocked).unwrap().permissions();
        perms.set_mode(0o000);
        std::fs::set_permissions(&blocked, perms).unwrap();
    }
    #[cfg(not(unix))]
    {
        // Non-unix: still assert the field exists on a clean walk.
        let bin = asgrep_bin();
        let out = run(
            &bin,
            &["--json", "index", "--dry-run", root.to_str().expect("utf8")],
        );
        let value = assert_success(&out, "index");
        assert!(value.get("walk_errors").is_some());
        return;
    }
    let bin = asgrep_bin();
    let out = run(
        &bin,
        &["--json", "index", "--dry-run", root.to_str().expect("utf8")],
    );
    // Restore perms so TempDir cleanup can remove blocked/.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&blocked).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&blocked, perms).unwrap();
    }
    let value = assert_success(&out, "index");
    assert_eq!(value["walk_errors"], true, "{value:#}");
    // Visible file still counted; blocked subtree is incomplete, not total zero.
    assert_eq!(value["files_would_index"], 1, "{value:#}");
}

#[test]
fn bench_json_emits_cv_pct_and_skips_vacuous_ast_grep_speedup() {
    let session = CliSession::sample(asgrep_bin());
    let index = session.index_path.to_str().expect("index utf8");
    let root = session.root.to_str().expect("root utf8");
    let history = session._temp.path().join("bench-history.json");
    let output = Command::new(&session.bin)
        .args([
            "--json",
            "--no-embed",
            "--index-path",
            index,
            "bench",
            root,
            "--query",
            "process_request",
            "--iterations",
            "3",
            "--skip-index",
        ])
        .env("NO_COLOR", "1")
        .env("ASGREP_BENCH_HISTORY_PATH", &history)
        .output()
        .expect("bench");
    let value = assert_success(&output, "bench");
    assert!(value["cv_pct"].as_f64().is_some());
    assert_eq!(value["ast_grep_comparison"]["compared"], false);
    assert!(value["ast_grep_comparison"]["skipped_reason"]
        .as_str()
        .unwrap_or("")
        .contains("pattern:"));
    assert!(value.get("speedup_vs_ast_grep").is_none());
    assert!(history.exists(), "bench history file should be written");
}

#[test]
fn bench_suite_json_is_single_envelope_even_on_failure() {
    let session = CliSession::sample(asgrep_bin());
    let index = session.index_path.to_str().expect("index utf8");
    let root = session.root.to_str().expect("root utf8");
    let output = Command::new(&session.bin)
        .args([
            "--json",
            "--no-embed",
            "--index-path",
            index,
            "bench",
            root,
            "--suite",
            "default",
            "--fixture",
            "sample",
            "--iterations",
            "1",
            "--skip-index",
        ])
        .env("NO_COLOR", "1")
        .env("ASGREP_BENCH_HISTORY", "0")
        .output()
        .expect("bench suite");
    let value = parse_stdout(&output);
    assert_eq!(value["command"], "bench");
    assert_eq!(value["tool"], "asgrep");
    assert!(value.get("cases").and_then(|c| c.as_array()).is_some());
    assert!(value.get("suite_ok").is_some());
    assert!(value.get("cv_pct").is_some());
    assert_eq!(value["ok"], value["suite_ok"]);
    if value["suite_ok"] == true {
        assert_eq!(output.status.code(), Some(0));
    } else {
        assert_eq!(output.status.code(), Some(2));
    }
}

/// d2a1.9: oversized batch file is rejected before OOM; machine envelope on failure.
#[test]
fn codemode_batch_oversized_file_is_machine_failure() {
    let dir = TempDir::new().expect("tempdir");
    // MAX_BATCH_REQUEST_BYTES = 4 * MAX_STDIN_LINE_BYTES (1 MiB) = 4 MiB.
    // Write slightly over the cap so metadata fast-path rejects.
    let path = dir.path().join("huge.json");
    {
        use std::io::{Seek, SeekFrom, Write};
        let mut f = std::fs::File::create(&path).expect("create");
        // MAX_BATCH_REQUEST_BYTES = 4 * 1_048_576. One byte past the cap.
        let over = (1_048_576u64 * 4) + 1;
        f.write_all(b"{").unwrap();
        f.seek(SeekFrom::Start(over - 1)).unwrap();
        f.write_all(b"}").unwrap();
        f.sync_all().unwrap();
        assert!(
            std::fs::metadata(&path).unwrap().len() >= over,
            "fixture must exceed batch cap"
        );
    }
    let bin = asgrep_bin();
    // No --json: codemode-batch must still emit a machine failure envelope (d2a1.10).
    let output = Command::new(&bin)
        .args(["codemode-batch", "--requests", path.to_str().expect("utf8")])
        .env("NO_COLOR", "1")
        .output()
        .expect("run");
    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        output.stderr.is_empty(),
        "machine failure must not also print human stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_stdout(&output);
    assert_eq!(value["ok"], false);
    assert_eq!(value["exit_code"], 2);
    assert_eq!(value["command"], "codemode-batch");
    assert_eq!(value["error"]["kind"], "operational");
    let msg = value["error"]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("exceeds max") || msg.contains("batch requests"),
        "unexpected message: {msg}"
    );
}

/// d2a1.9: stdin path also caps (never fully slurp oversize); d2a1.10 envelope without --json.
#[test]
fn codemode_batch_oversized_stdin_is_machine_failure() {
    use std::io::Write;
    use std::process::Stdio;
    let bin = asgrep_bin();
    let mut child = Command::new(&bin)
        .args(["codemode-batch", "--requests", "-"])
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        // Stream more than 4 MiB; take() must stop allocation near the cap.
        let chunk = vec![b'a'; 64 * 1024];
        let target = (1_048_576usize * 4) + (128 * 1024);
        let mut written = 0usize;
        while written < target {
            match stdin.write_all(&chunk) {
                Ok(()) => written += chunk.len(),
                Err(_) => break, // peer closed after rejecting
            }
        }
        // Drop stdin to close pipe.
    }
    let output = child.wait_with_output().expect("wait");
    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_stdout(&output);
    assert_eq!(value["ok"], false);
    assert_eq!(value["exit_code"], 2);
    assert_eq!(value["command"], "codemode-batch");
    let msg = value["error"]["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("exceeds max") || msg.contains("stdin") || msg.contains("batch"),
        "unexpected message: {msg}"
    );
}

/// d2a1.10: missing batch file without --json still yields machine operational envelope.
#[test]
fn codemode_batch_missing_file_machine_envelope_without_json_flag() {
    let bin = asgrep_bin();
    let missing = TempDir::new().expect("temp").path().join("nope.json");
    let output = Command::new(&bin)
        .args([
            "codemode-batch",
            "--requests",
            missing.to_str().expect("utf8"),
        ])
        .env("NO_COLOR", "1")
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(2));
    assert!(
        output.stderr.is_empty(),
        "stderr should be empty in machine mode: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_stdout(&output);
    assert_eq!(value["ok"], false);
    assert_eq!(value["command"], "codemode-batch");
    assert_eq!(value["error"]["kind"], "operational");
}


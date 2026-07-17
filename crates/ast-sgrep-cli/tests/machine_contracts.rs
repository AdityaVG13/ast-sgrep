use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ast_sgrep_testkit::CliSession;
use serde_json::Value;
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
            String::from_utf8_lossy(&output.stderr),
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
        let value = assert_success(
            &run(
                &session.bin,
                &["--json", "--no-embed", "--index-path", index, command, root],
            ),
            command,
        );
        assert_shape(&value, &shapes["index"]);
    }

    let status = assert_success(
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
    );
    assert_shape(&status, &shapes["status"]);

    let blocked = TempDir::new().expect("tempdir");
    let blocked_index = blocked.path().join("blocked.db");
    std::fs::create_dir(&blocked_index).expect("blocking directory");
    let blocked_index = blocked_index.to_str().expect("blocked path utf8");
    let doctor = assert_success(
        &run(
            &session.bin,
            &["--json", "--index-path", blocked_index, "doctor", root],
        ),
        "doctor",
    );
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
    std::fs::write(
        &gold,
        serde_json::json!({
            "corpus": "sample",
            "queries": [{
                "name": "process",
                "query": "process_request",
                "k": 5,
                "relevant": [{"file": "src/main.rs", "symbol": "process_request"}]
            }]
        })
        .to_string(),
    )
    .unwrap();
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
        ["--json", "--excerpt-lines", "101", "query", "."],
    ] {
        let output = run(&bin, &args);
        assert_eq!(output.status.code(), Some(1));
        let mut value = parse_stdout(&output);
        assert_eq!(value["error"]["kind"], "usage");
        value["error"]["message"] = "<message>".into();
        assert_eq!(&value, golden);
    }
}

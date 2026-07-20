use std::path::{Path, PathBuf}; use std::process::{Command, Output}; use ast_sgrep_testkit::CliSession; use serde_json::Value; use tempfile::TempDir;

fn bin() -> PathBuf { PathBuf::from(env!("CARGO_BIN_EXE_asgrep")) } fn run(bin: &Path, args: &[&str]) -> Output { Command::new(bin).args(args).env("NO_COLOR", "1").output().expect("run asgrep") } fn parse_stdout(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| panic!(
        "stdout is not JSON: {e}\nstdout: {}\nstderr: {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)))
} fn assert_success(output: &Output, command: &str) -> Value {
    assert_eq!(output.status.code(), Some(0), "{}", String::from_utf8_lossy(&output.stderr)); assert!(output.stderr.is_empty(), "unexpected stderr: {}", String::from_utf8_lossy(&output.stderr));
    let v = parse_stdout(output); assert_eq!(v["schema_version"], "1.0.0"); assert_eq!(v["tool"], "asgrep"); assert_eq!(v["command"], command); assert_eq!(v["ok"], true); v
} fn fixture(name: &str) -> Value {
    let raw = match name {
        "capabilities" => include_str!("fixtures/capabilities.json"), "shapes" => include_str!("fixtures/machine_shapes.json"), "envelopes" => include_str!("fixtures/envelopes.json"), _ => panic!("unknown fixture {name}"),
    }; serde_json::from_str(raw).expect("valid JSON fixture")
} fn assert_shape(value: &Value, shape: &Value) {
    let mut actual: Vec<_> = value.as_object().expect("object").keys().cloned().collect(); actual.sort(); let expected: Vec<_> = shape.as_array().expect("key array").iter()
        .map(|k| k.as_str().expect("string").to_owned()).collect();
    assert_eq!(actual, expected);
}

#[test] fn capabilities_and_version_match_goldens() {
    let b = bin(); let mut caps = assert_success(&run(&b, &["capabilities", "--json"]), "capabilities"); caps["version"] = "<version>".into(); assert_eq!(caps, fixture("capabilities"));
    let mut ver = assert_success(&run(&b, &["version", "--json"]), "version"); ver["version"] = "<version>".into(); assert_eq!(ver, fixture("envelopes")["version"]);
}

#[test] fn index_reindex_status_and_doctor_have_stable_shapes() {
    let session = CliSession::sample(bin()); let index = session.index_path.to_str().unwrap(); let root = session.root.to_str().unwrap(); let shapes = fixture("shapes"); for command in ["index", "reindex"] {
        let v = assert_success(&run(&session.bin, &["--json", "--no-embed", "--index-path", index, command, root]), command); assert_shape(&v, &shapes["index"]);
    } let status = assert_success(&run(&session.bin, &["--json", "--no-embed", "--index-path", index, "status", root]), "status");
    assert_shape(&status, &shapes["status"]); let blocked = TempDir::new().unwrap(); let blocked_index = blocked.path().join("blocked.db");
    std::fs::create_dir(&blocked_index).unwrap(); let bi = blocked_index.to_str().unwrap(); let doctor = assert_success(&run(&session.bin, &["--json", "--index-path", bi, "doctor", root]), "doctor");
    assert_shape(&doctor, &shapes["doctor"]); assert_eq!(doctor["healthy"], false); assert_eq!(doctor["status"], Value::Null); assert!(!doctor["issues"].as_array().unwrap().is_empty());
}

#[test] fn agent_search_modes_are_stable_and_bounded() {
    let session = CliSession::sample(bin()); let shapes = fixture("shapes"); let agent = session.search_json("process_request", &["--no-embed", "--limit", "2", "--format", "agent"]);
    assert_shape(&agent, &shapes["agent"]); assert_eq!(agent["command"], "search"); assert_eq!(agent["ok"], true); assert!(agent["hits"].as_array().unwrap().len() <= 2);
    let capsule = session.search_json("process_request", &["--no-embed", "--limit", "2", "--format", "agent-capsule", "--excerpt-lines", "2"]); assert_shape(&capsule, &shapes["agent-capsule"]);
    for hit in capsule["hits"].as_array().unwrap() {
        assert!(hit["preview"].as_str().unwrap().chars().count() <= 121); assert!(hit["excerpt"].as_str().unwrap().lines().count() <= 2);
    }
}

#[test] fn chain_eval_and_bench_successes_use_machine_envelope() {
    let session = CliSession::sample(bin()); let index = session.index_path.to_str().unwrap(); let root = session.root.to_str().unwrap();
    let chain = assert_success(&run(&session.bin, &["--json", "--no-embed", "--index-path", index, "chain", "process_request", root]), "chain"); assert!(chain["nodes"].is_array());
    let bench = assert_success(&run(&session.bin, &["--json", "--no-embed", "--index-path", index, "bench", root, "--query", "process_request", "--iterations", "1", "--skip-index"]), "bench"); assert_eq!(bench["iterations"], 1);
}

#[test] fn operational_failures_are_json_and_exit_two() {
    let b = bin(); let temp = TempDir::new().unwrap(); let blocked = temp.path().join("blocked.db");
    std::fs::create_dir(&blocked).unwrap(); let bi = blocked.to_str().unwrap(); let root = temp.path().to_str().unwrap(); let golden = &fixture("envelopes")["operational"]; for (command, args) in [
        ("index", vec!["--json", "--index-path", bi, "index", root]), ("reindex", vec!["--json", "--index-path", bi, "reindex", root]),
        ("status", vec!["--json", "--index-path", bi, "status", root]), ("search", vec!["--json", "--index-path", bi, "query", root]),
    ] {
        let output = run(&b, &args); assert_eq!(output.status.code(), Some(2), "{command}: {}", String::from_utf8_lossy(&output.stderr));
        let mut value = parse_stdout(&output); assert_eq!(value["command"], command); assert_eq!(value["error"]["kind"], "operational"); assert!(value["error"]["message"].as_str().unwrap().chars().count() <= 4_097);
        value["command"] = "<command>".into(); value["error"]["message"] = "<message>".into(); assert_eq!(&value, golden);
    }
}

#[test] fn bounded_arguments_are_json_usage_errors() {
    let b = bin(); let golden = &fixture("envelopes")["usage"]; for args in [["--json", "--limit", "1001", "query", "."], ["--json", "--excerpt-lines", "101", "query", "."]] {
        let output = run(&b, &args); assert_eq!(output.status.code(), Some(1)); let mut value = parse_stdout(&output); assert_eq!(value["error"]["kind"], "usage"); value["error"]["message"] = "<message>".into(); assert_eq!(&value, golden);
    }
}

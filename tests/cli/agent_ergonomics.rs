//! Agent-ergonomics first-try surfaces (flag/command recovery).
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

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

#[test]
fn jsno_flag_recovers_as_json() {
    let (code, stdout, stderr) = run(&["--jsno", "capabilities"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "capabilities");
    assert!(
        stderr.contains("recovered `--jsno` as `--json`"),
        "stderr should teach the canonical flag: {stderr}"
    );
}

#[test]
fn command_typo_capabilites_recovers() {
    let (code, stdout, stderr) = run(&["capabilites", "--json"]);
    assert_eq!(code, 0, "stderr={stderr} stdout={stdout}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["command"], "capabilities");
    assert!(
        stderr.contains("recovered `capabilites` as `capabilities`"),
        "stderr={stderr}"
    );
}

#[test]
fn colour_flag_is_a_noop_with_a_teaching_note() {
    let (code, stdout, stderr) = run(&["--colour", "capabilities"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["ok"], true);
    assert!(stderr.contains("ignored `--colour`"), "stderr={stderr}");
}

#[test]
fn robot_triage_works_without_doctor_verb() {
    let (code, stdout, stderr) = run(&["--robot-triage"]);
    assert!(
        code == 0 || code == 2,
        "mega-command must run doctor, got {code} stderr={stderr} stdout={stdout}"
    );
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["command"], "doctor");
    assert!(value.get("robot_triage").is_some() || value["ok"].is_boolean());
}

#[test]
fn robot_next_aliases_robot_triage() {
    let (code, stdout, stderr) = run(&["--robot-next"]);
    assert!(code == 0 || code == 2, "stderr={stderr} stdout={stdout}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["command"], "doctor");
}

#[test]
fn json_stdout_is_standalone_for_jq() {
    let (code, stdout, stderr) = run(&["--json", "capabilities"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(
        stdout.trim_start().starts_with("{"),
        "stdout must be JSON without a log prefix: {stdout}"
    );
    let _: Value = serde_json::from_str(&stdout).expect("stdout is one JSON value");
    assert!(
        !stderr.contains('{'),
        "diagnostics must not mix JSON into stderr: {stderr}"
    );
}


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

#[test]
fn short_j_is_json() {
    let (code, stdout, stderr) = run(&["-j", "capabilities"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["command"], "capabilities");
}

#[test]
fn jason_flag_recovers_as_json() {
    let (code, stdout, stderr) = run(&["--jason", "capabilities"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["command"], "capabilities");
    assert!(
        stderr.contains("recovered `--jason` as `--json`"),
        "stderr={stderr}"
    );
}

#[test]
fn machine_and_format_json_recover_as_json() {
    for flag in ["--machine", "--output-json", "--format=json"] {
        let (code, stdout, stderr) = run(&[flag, "capabilities"]);
        assert_eq!(code, 0, "flag={flag} stderr={stderr}");
        let value: Value = serde_json::from_str(&stdout).expect("json stdout");
        assert_eq!(value["command"], "capabilities");
        assert!(
            stderr.contains("recovered") && stderr.contains("`--json`"),
            "flag={flag} stderr={stderr}"
        );
    }
}

#[test]
fn dryrun_recovers_as_dry_run() {
    let (code, stdout, stderr) = run(&["codemod", "--dryrun"]);
    assert_eq!(code, 1, "stdout={stdout} stderr={stderr}");
    assert!(
        stderr.contains("recovered `--dryrun` as `--dry-run`"),
        "stderr={stderr}"
    );
    assert!(
        stdout.contains("asgrep codemod --dry-run --pattern")
            || stderr.contains("asgrep codemod --dry-run --pattern"),
        "teach the copy-paste command: stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn docs_alias_recovers_as_robot_docs() {
    let (code, stdout, stderr) = run(&["docs"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(
        stdout.contains("asgrep — agent handbook") || stdout.contains("robot-docs"),
        "stdout={stdout}"
    );
    assert!(
        stderr.contains("recovered `docs` as `robot-docs`"),
        "stderr={stderr}"
    );
}

#[test]
fn codemod_missing_args_names_dry_run_command() {
    let (code, stdout, stderr) = run(&["codemod"]);
    assert_eq!(code, 1, "stdout={stdout} stderr={stderr}");
    assert!(
        stderr.contains("asgrep codemod --dry-run --pattern"),
        "error must name the exact dry-run command: {stderr}"
    );
}

#[test]
fn codemod_apply_without_yes_names_recovery_commands() {
    let (code, stdout, stderr) = run(&[
        "codemod",
        "--pattern",
        "legacy($ARG)",
        "--rewrite",
        "modern($ARG)",
        ".",
    ]);
    assert_eq!(code, 1, "stdout={stdout} stderr={stderr}");
    let blob = format!("{stdout}{stderr}");
    assert!(blob.contains("without --yes"), "must refuse apply: {blob}");
    assert!(
        blob.contains("asgrep codemod --dry-run --pattern"),
        "must name dry-run: {blob}"
    );
    assert!(
        blob.contains("asgrep codemod --yes --pattern"),
        "must name apply command: {blob}"
    );
}

#[test]
fn force_alias_is_accepted_as_yes() {
    // --force must parse as --yes so clap does not reject a common agent spelling.
    // The command still fails on missing pattern, which proves the flag was accepted.
    let (code, stdout, stderr) = run(&["codemod", "--force", "--dry-run"]);
    assert_eq!(code, 1, "stdout={stdout} stderr={stderr}");
    let blob = format!("{stdout}{stderr}");
    assert!(
        !blob.contains("unexpected argument") && !blob.contains("unexpected arg"),
        "alias --force must be accepted: {blob}"
    );
}

#[test]
fn capabilities_json_is_byte_stable() {
    let (c1, s1, e1) = run(&["capabilities", "--json"]);
    let (c2, s2, e2) = run(&["capabilities", "--json"]);
    assert_eq!(c1, 0, "stderr={e1}");
    assert_eq!(c2, 0, "stderr={e2}");
    assert_eq!(
        s1, s2,
        "capabilities --json must be byte-identical across runs"
    );
}

#[test]
fn doctor_json_omits_tty_and_clock_fields() {
    let (code, stdout, stderr) = run(&["--json", "doctor", "."]);
    assert!(code == 0 || code == 2, "stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert!(
        value.get("tty").is_none(),
        "tty leaks host interactivity: {value}"
    );
    let blob = stdout.to_ascii_lowercase();
    for needle in ["updated_unix_ms", "generated_at", "timestamp"] {
        assert!(
            !blob.contains(needle),
            "doctor envelope must not include {needle}: {stdout}"
        );
    }
}

#[test]
fn capabilities_documents_yes_and_source_date_epoch() {
    let (code, stdout, stderr) = run(&["capabilities", "--json"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    let flags = value["global_flags"].as_array().expect("global_flags");
    assert!(
        flags.iter().any(|f| f.as_str() == Some("--yes")),
        "capabilities must list --yes: {flags:?}"
    );
    let env = value["environment"].as_array().expect("environment");
    assert!(
        env.iter().any(|e| e.as_str() == Some("SOURCE_DATE_EPOCH")),
        "capabilities must list SOURCE_DATE_EPOCH: {env:?}"
    );
    let codemod = value["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .find(|c| c["name"] == "codemod")
        .expect("codemod command");
    assert_eq!(codemod["safe_mutating"]["kind"], "source_rewrite");
}

#[test]
fn help_after_help_names_yes_and_json_short_flag() {
    let (code, stdout, stderr) = run(&["--help"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let blob = format!("{stdout}{stderr}");
    assert!(
        blob.contains("-j") && blob.contains("--json"),
        "help must name -j/--json: {blob}"
    );
    assert!(blob.contains("--yes"), "help must name --yes: {blob}");
}

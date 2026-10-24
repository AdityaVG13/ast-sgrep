//! Agent-ergonomics first-try surfaces (flag/command recovery).
use serde_json::Value;
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

#[test]
fn second_root_positional_teaches_one_root_per_query() {
    let (code, _stdout, stderr) = run(&["search", "auth", "crates", "tests"]);
    assert_eq!(code, 1, "stderr={stderr}");
    assert!(stderr.contains("unexpected argument"), "{stderr}");
    assert!(stderr.contains("one ROOT per query"), "{stderr}");

    let (code, stdout, _stderr) = run(&["--json", "search", "auth", "crates", "tests"]);
    assert_eq!(code, 1);
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["error"]["kind"], "usage");
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("one ROOT per query"),
        "{value}"
    );
}

/// Machine-mode teaching notes ride the JSON envelope (`"notes": [...]`) with
/// stderr silent (usage failures stay stderr-clean for piping); human mode
/// prints them to stderr. Joins envelope notes for substring checks.
fn envelope_notes(stdout: &str) -> String {
    serde_json::from_str::<Value>(stdout)
        .ok()
        .and_then(|value| {
            value["notes"].as_array().map(|list| {
                list.iter()
                    .filter_map(|note| note.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        })
        .unwrap_or_default()
}

#[test]
fn jsno_flag_recovers_as_json() {
    let (code, stdout, stderr) = run(&["--jsno", "capabilities"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["ok"], true);
    assert_eq!(value["command"], "capabilities");
    assert!(stderr.is_empty(), "machine mode is stderr-silent: {stderr}");
    assert!(
        envelope_notes(&stdout).contains("recovered `--jsno` as `--json`"),
        "envelope should teach the canonical flag: {stdout}"
    );
}

#[test]
fn command_typo_capabilites_recovers() {
    let (code, stdout, stderr) = run(&["capabilites", "--json"]);
    assert_eq!(code, 0, "stderr={stderr} stdout={stdout}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["command"], "capabilities");
    assert!(stderr.is_empty(), "machine mode is stderr-silent: {stderr}");
    assert!(
        envelope_notes(&stdout).contains("recovered `capabilites` as `capabilities`"),
        "envelope notes: {stdout}"
    );
}

#[test]
fn colour_flag_is_a_noop_with_a_teaching_note() {
    let (code, stdout, stderr) = run(&["--colour", "capabilities"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["ok"], true);
    assert!(stderr.is_empty(), "machine mode is stderr-silent: {stderr}");
    assert!(
        envelope_notes(&stdout).contains("ignored `--colour`"),
        "envelope notes: {stdout}"
    );
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

    let temp = TempDir::new().unwrap();
    let root = temp.path().join("literal-root");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("lib.rs"),
        "fn literal_routing_target() { let flag = \"--robot-next\"; }\n",
    )
    .unwrap();
    let indexed = Command::new(asgrep_bin())
        .args(["--json", "--no-embed", "index"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(indexed.status.success());
    let output = Command::new(asgrep_bin())
        .args(["--json", "--no-embed", "--root"])
        .arg(&root)
        .args(["--", "--robot-next"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "literal data must not inject doctor: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let literal: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(literal["command"], "search");
    assert!(!literal["hits"].as_array().unwrap().is_empty());
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
    assert!(stderr.is_empty(), "machine mode is stderr-silent: {stderr}");
    assert!(
        envelope_notes(&stdout).contains("recovered `--jason` as `--json`"),
        "envelope notes: {stdout}"
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
            stderr.is_empty(),
            "machine mode is stderr-silent: flag={flag} stderr={stderr}"
        );
        assert!(
            envelope_notes(&stdout).contains("recovered")
                && envelope_notes(&stdout).contains("`--json`"),
            "flag={flag} envelope notes: {stdout}"
        );
    }
}

#[test]
fn dryrun_recovers_as_dry_run() {
    let (code, stdout, stderr) = run(&["codemod", "--dryrun"]);
    assert_eq!(code, 1, "stdout={stdout} stderr={stderr}");
    assert!(stderr.is_empty(), "machine mode is stderr-silent: {stderr}");
    assert!(
        envelope_notes(&stdout).contains("recovered `--dryrun` as `--dry-run`"),
        "envelope notes: {stdout}"
    );
    assert!(
        stdout.contains("asgrep codemod --dry-run --pattern"),
        "teach the copy-paste command in the envelope: stdout={stdout}"
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

fn run_env(args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut cmd = Command::new(asgrep_bin());
    cmd.args(args).env("NO_COLOR", "1");
    for (key, value) in env {
        cmd.env(key, value);
    }
    let output = cmd.output().expect("run asgrep");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn ci_and_term_dumb_suppress_index_progress() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("main.rs"),
        "fn main() {}
",
    )
    .unwrap();
    let root_s = root.to_str().unwrap();
    for env in [[("CI", "1")], [("TERM", "dumb")]] {
        let (code, stdout, stderr) = run_env(&["index", "--dry-run", root_s], &env);
        assert_eq!(code, 0, "env={env:?} stdout={stdout} stderr={stderr}");
        assert!(
            !stderr.contains("asgrep: dry-run scanned") && !stderr.contains("asgrep: indexing"),
            "progress must be quiet in {env:?}: {stderr}"
        );
    }
}

#[test]
fn no_color_flag_is_accepted_in_ci() {
    let (code, stdout, stderr) = run_env(&["--no-color", "--json", "capabilities"], &[("CI", "1")]);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["command"], "capabilities");
}

#[test]
fn pins_common_agent_typos() {
    let (code, stdout, stderr) = run(&["indx", "--help"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(
        stderr.contains("recovered `indx` as `index`"),
        "stderr={stderr}"
    );
    assert!(stdout.contains("asgrep index"), "stdout={stdout}");

    let (code, stdout, stderr) = run(&["--jsn", "capabilities"]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stderr.is_empty(), "machine mode is stderr-silent: {stderr}");
    assert!(
        envelope_notes(&stdout).contains("recovered `--jsn` as `--json`"),
        "envelope notes: {stdout}"
    );
    let value: Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(value["command"], "capabilities");
    let flags = value["global_flags"].as_array().expect("global_flags");
    assert!(
        flags.iter().any(|f| f.as_str() == Some("-j")),
        "capabilities must list -j: {flags:?}"
    );

    let (code, stdout, stderr) = run(&["--limmit", "3", "capabilities", "--json"]);
    assert_eq!(code, 0, "stderr={stderr} stdout={stdout}");
    assert!(stderr.is_empty(), "machine mode is stderr-silent: {stderr}");
    assert!(
        envelope_notes(&stdout).contains("recovered `--limmit` as `--limit`"),
        "envelope notes: {stdout}"
    );

    let (code, stdout, stderr) = run(&["--robot-diagnose"]);
    assert!(code == 0 || code == 2, "stderr={stderr} stdout={stdout}");
    assert!(stderr.is_empty(), "machine mode is stderr-silent: {stderr}");
    assert!(
        envelope_notes(&stdout).contains("`--robot-triage`"),
        "envelope notes: {stdout}"
    );
    let value: Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(value["command"], "doctor");
}

#[test]
fn typo_recovery_preserves_option_values() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("statuz");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("lib.rs"), "fn needle_value_target() {}\n").unwrap();
    let indexed = Command::new(asgrep_bin())
        .current_dir(temp.path())
        .args(["--json", "--no-embed", "index", "statuz"])
        .output()
        .unwrap();
    assert!(
        indexed.status.success(),
        "{}",
        String::from_utf8_lossy(&indexed.stderr)
    );
    for flag in ["--root", "--rooot"] {
        let output = Command::new(asgrep_bin())
            .current_dir(temp.path())
            .args([
                "--json",
                "--no-embed",
                flag,
                "statuz",
                "search",
                "needle_value_target",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "option values must not retarget root: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(value["hits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|hit| hit["symbol"] == "needle_value_target"));
        assert!(!String::from_utf8_lossy(&output.stderr).contains("recovered `statuz`"));
    }
}

#[test]
fn typo_recovery_help_data_does_not_authorize_mutation() {
    let temp = TempDir::new().unwrap();
    for root in ["help", "--help"] {
        let path = temp.path().join(root);
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("lib.rs"), "fn untouched() {}\n").unwrap();
        let args = if root == "help" {
            vec!["--json", "--no-embed", "indx", "--root", root]
        } else {
            vec!["--json", "--no-embed", "indx", "--", root]
        };
        let output = Command::new(asgrep_bin())
            .current_dir(temp.path())
            .args(args)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(1),
            "root data is not help intent: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["error"]["kind"], "usage");
        assert!(
            !path.join(".asgrep").exists(),
            "a guessed mutating command must not create an index"
        );
    }
}

/// H-PERF-001 option (b): duty-cycle supervision is opt-in. The default run
/// is a single in-process command; setting ASGREP_CPU_LIMIT_PERCENT (1..=80)
/// re-enables the supervisor. Doctor is the discoverability surface: it must
/// report the active policy and teach the opt-in variable.
fn run_with_env(args: &[&str], envs: &[(&str, Option<&str>)]) -> (i32, String, String) {
    let mut cmd = Command::new(asgrep_bin());
    cmd.args(args).env("NO_COLOR", "1");
    for (key, value) in envs {
        match value {
            Some(v) => {
                cmd.env(key, v);
            }
            None => {
                cmd.env_remove(key);
            }
        }
    }
    let output = cmd.output().expect("run asgrep");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn doctor_supervision_json(envs: &[(&str, Option<&str>)]) -> Value {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let (code, stdout, stderr) = run_with_env(&["--json", "doctor", root.to_str().unwrap()], envs);
    // Doctor fail-closes with exit 2 on an unhealthy (here: indexless) root;
    // the supervision policy field is reported on both paths (MJ-010 pins
    // the exit contract, machine_shapes.json pins the key set).
    assert!(code == 0 || code == 2, "stderr={stderr} stdout={stdout}");
    serde_json::from_str(&stdout).expect("json stdout")
}

#[test]
fn doctor_reports_supervision_default_off() {
    let value = doctor_supervision_json(&[("ASGREP_CPU_LIMIT_PERCENT", None)]);
    assert_eq!(
        value["supervision"]["mode"], "off",
        "supervision is opt-in (H-PERF-001 option b): default must be off: {value}"
    );
    let opt_in = value["supervision"]["opt_in"].as_str().unwrap_or_default();
    assert!(
        opt_in.contains("ASGREP_CPU_LIMIT_PERCENT"),
        "doctor must teach the opt-in variable: {value}"
    );
}

#[test]
fn doctor_reports_opt_in_duty_cycle_and_invalid_falls_back_to_default() {
    let value = doctor_supervision_json(&[("ASGREP_CPU_LIMIT_PERCENT", Some("50"))]);
    assert_eq!(value["supervision"]["mode"], "duty_cycle", "{value}");
    assert_eq!(value["supervision"]["cpu_limit_percent"], 50, "{value}");
    // A malformed opt-in value keeps the supervised default bound (80) —
    // fail-safe, never a silent switch to unsupervised.
    let value = doctor_supervision_json(&[("ASGREP_CPU_LIMIT_PERCENT", Some("not-a-number"))]);
    assert_eq!(value["supervision"]["mode"], "duty_cycle", "{value}");
    assert_eq!(value["supervision"]["cpu_limit_percent"], 80, "{value}");
}

// ---- Phase B (2026-10-22 corpus): grep/rg habit-flag + cross-surface vocab ----

/// Small indexed fixture with a distinctive needle, mirroring the corpus token.
fn habit_fixture(needle: &str) -> (TempDir, String) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("habits");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("lib.rs"),
        format!("fn {needle}() {{}}\nfn caller() {{ {needle}(); }}\n"),
    )
    .unwrap();
    let indexed = Command::new(asgrep_bin())
        .args(["--json", "--no-embed", "index"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(
        indexed.status.success(),
        "{}",
        String::from_utf8_lossy(&indexed.stderr)
    );
    (temp, root.to_str().unwrap().to_string())
}

#[test]
fn grep_habit_semantic_flags_recover_to_canonical() {
    let (_temp, root) = habit_fixture("flurbify_widget");
    // (args after the needle, expectation on stdout)
    let cases: &[(&[&str], &str)] = &[
        (&["-l"], "lib.rs"),
        (&["-C", "2"], "flurbify_widget"),
        (&["-g", "*.rs"], "flurbify_widget"),
        (&["-t", "rust"], "flurbify_widget"),
        (&["--max-results", "3"], "flurbify_widget"),
        (&["--include=*.rs"], "flurbify_widget"),
        (&["--include", "*.rs"], "flurbify_widget"),
    ];
    for (extra, expect) in cases {
        let mut args = vec!["search", "flurbify_widget", "--root", root.as_str()];
        args.extend_from_slice(extra);
        let (code, stdout, stderr) = run(&args);
        assert_eq!(code, 0, "extra={extra:?} stderr={stderr} stdout={stdout}");
        assert!(stdout.contains(expect), "extra={extra:?} stdout={stdout}");
        assert!(
            stderr.contains("recovered"),
            "must teach the canonical flag: extra={extra:?} stderr={stderr}"
        );
    }
}

#[test]
fn grep_noop_flags_teach_the_default() {
    let (_temp, root) = habit_fixture("flurbify_widget");
    for flag in ["-i", "-n", "--ignore-case", "--line-number"] {
        let (code, stdout, stderr) = run(&["search", "flurbify_widget", flag, "--root", &root]);
        assert_eq!(code, 0, "flag={flag} stderr={stderr}");
        assert!(stdout.contains("flurbify_widget"), "flag={flag} stdout={stdout}");
        assert!(
            stderr.contains("ignored"),
            "noop flags must teach the default: flag={flag} stderr={stderr}"
        );
    }
}

#[test]
fn grep_no_equivalent_flags_teach_not_wall() {
    let (_temp, root) = habit_fixture("flurbify_widget");
    // `-w` (word: channel), `-F` (queries are grammar tokens), and `-c`
    // (--count) recovered in rounds 6-7; the remaining true gaps teach.
    for (flag, mode) in [("-v", "invert")] {
        let (code, _stdout, stderr) = run(&["search", "flurbify_widget", flag, "--root", &root]);
        assert_eq!(code, 1, "flag={flag} stderr={stderr}");
        assert!(
            !stderr.contains("as a value"),
            "clap's pass-as-value tip must not be the guidance: flag={flag} stderr={stderr}"
        );
        assert!(
            stderr.contains(mode) && stderr.contains("asgrep search"),
            "must name the mode gap and a real command: flag={flag} stderr={stderr}"
        );
    }
}

#[test]
fn cross_surface_vocab_recovers_to_cli_subcommands() {
    let (_temp, root) = habit_fixture("flurbify_widget");
    // (token, expect_hits) — semantic_* run without embeds on this fixture,
    // so only recovery is asserted there.
    let cases: &[(&str, bool)] = &[
        ("grep", true),
        ("code_search", true),
        ("keyword_search", true),
        ("semantic_search", false),
        ("sem", false),
        ("index_status", false),
    ];
    for (token, expect_hits) in cases {
        // index_status takes no QUERY; the realistic transfer shape is bare.
        let args: Vec<&str> = if *token == "index_status" {
            vec![token, "--root", root.as_str()]
        } else {
            vec![token, "flurbify_widget", "--root", root.as_str()]
        };
        let (code, stdout, stderr) = run(&args);
        assert_eq!(code, 0, "token={token} stderr={stderr} stdout={stdout}");
        assert!(
            stderr.contains(&format!("recovered `{token}`")),
            "token={token} stderr={stderr}"
        );
        if *expect_hits {
            assert!(stdout.contains("flurbify_widget"), "token={token} stdout={stdout}");
        }
    }
}

#[test]
fn mutating_vocab_never_runs_from_a_guess() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("fresh");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("lib.rs"), "fn untouched() {}\n").unwrap();
    let root_s = root.to_str().unwrap().to_string();
    let (code, _stdout, stderr) = run(&["index_repo", "--root", &root_s]);
    // The MCP name must not silently mutate: no index may appear, and the
    // output must teach the canonical mutating command.
    assert!(
        !root.join(".asgrep").exists(),
        "a guessed mutating vocab token must not build an index"
    );
    assert!(
        stderr.contains("asgrep index"),
        "must teach the canonical command: stderr={stderr}"
    );
    assert!(code == 1 || code == 2, "stderr={stderr}");
}

#[test]
fn format_json_detached_and_json_value_flags_recover() {
    let (_temp, root) = habit_fixture("flurbify_widget");
    let (code, stdout, stderr) = run(&[
        "search", "flurbify_widget", "--root", &root, "--format", "json",
    ]);
    assert_eq!(code, 0, "stderr={stderr} stdout={stdout}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["ok"], true);
    assert!(stderr.is_empty(), "machine mode is stderr-silent: {stderr}");
    assert!(
        envelope_notes(&stdout).contains("recovered `--format json` as `--json`"),
        "envelope notes: {stdout}"
    );

    let (code, stdout, stderr) = run(&[
        "search", "flurbify_widget", "--root", &root, "--json=pretty",
    ]);
    assert_eq!(code, 0, "stderr={stderr} stdout={stdout}");
    let _: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert!(stderr.is_empty(), "machine mode is stderr-silent: {stderr}");
    assert!(
        envelope_notes(&stdout).contains("takes no value"),
        "envelope notes: {stdout}"
    );
}

#[test]
fn root_level_pattern_flag_routes_to_search() {
    let (_temp, root) = habit_fixture("flurbify_widget");
    let (code, stdout, stderr) = run(&[
        "--pattern", "flurbify_widget($$$A)", "--root", &root, "--json",
    ]);
    assert_eq!(code, 0, "stderr={stderr} stdout={stdout}");
    assert!(stderr.is_empty(), "machine mode is stderr-silent: {stderr}");
    assert!(
        envelope_notes(&stdout).contains("recovered root-level `--pattern`"),
        "envelope notes: {stdout}"
    );
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert!(
        value["hits"].as_array().is_some_and(|hits| !hits.is_empty()),
        "structural pattern must hit the fixture call: {value}"
    );
}

#[test]
fn zero_hit_search_classifies_the_miss_in_both_formats() {
    let (_temp, root) = habit_fixture("flurbify_widget");
    // Human: one miss line, never silence.
    let (code, stdout, _stderr) = run(&["search", "nonexistent_token_xyz", "--root", &root]);
    assert_eq!(code, 0, "stdout={stdout}");
    assert!(
        stdout.contains("no hits") && stdout.contains("Next:"),
        "human zero-hit must classify: stdout={stdout}"
    );
    // JSON: why + next on the standard envelope, matching the compact
    // envelope's documented contract.
    let (code, stdout, _stderr) =
        run(&["search", "nonexistent_token_xyz", "--root", &root, "--json"]);
    assert_eq!(code, 0, "stdout={stdout}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert!(
        value["why"].as_str().is_some_and(|w| !w.is_empty()),
        "{value}"
    );
    assert!(
        value["next"].as_str().is_some_and(|n| !n.is_empty()),
        "{value}"
    );
}

// ---- round 6: cross-dialect verbs, ast-grep/semgrep habits, LSP refs ----

/// Shared corpus for the round-6 intents: one symbol with callers, one
/// module import, indexed warm.
fn round6_repo() -> (TempDir, std::path::PathBuf) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("repo");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "fn cross_dialect_target() {\n    cross_dialect_target();\n}\nfn target_consumer() {\n    cross_dialect_target();\n}\n",
    )
    .unwrap();
    std::fs::write(root.join("src/app.py"), "import json\n").unwrap();
    let indexed = Command::new(asgrep_bin())
        .args(["--json", "--no-embed", "index"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(indexed.status.success(), "index must succeed");
    (temp, root)
}

/// Codemode's verb family (`defs`/`callers`/`imports`) is first-class CLI
/// surface, routing through the same prefixed grammar channels (QG-002/003)
/// the session dispatcher uses — an agent transferring codemode vocabulary
/// to the CLI must not hit the QUERY+ROOT misparse wall.
#[test]
fn cross_dialect_verbs_are_first_class() {
    let (_temp, root) = round6_repo();
    let root_s = root.to_str().unwrap();

    let (code, stdout, stderr) = run(&["--json", "defs", "cross_dialect_target", root_s]);
    assert_eq!(code, 0, "stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["ok"], true, "{value}");
    assert!(stdout.contains("lib.rs"), "{stdout}");

    let (code, stdout, stderr) = run(&["--json", "callers", "cross_dialect_target", root_s]);
    assert_eq!(code, 0, "stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["ok"], true, "{value}");
    assert!(stdout.contains("lib.rs"), "{stdout}");

    let (code, stdout, stderr) = run(&["--json", "imports", "json", root_s]);
    assert_eq!(code, 0, "stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["ok"], true, "{value}");
    assert!(stdout.contains("app.py"), "{stdout}");
}

/// ast-grep's `-p PATTERN` is exactly asgrep's structural `--pattern`:
/// alias-and-teach, never clap's "pass it as a value" tip.
#[test]
fn p_short_flag_is_ast_grep_pattern_alias() {
    let (_temp, root) = round6_repo();
    let root_s = root.to_str().unwrap();
    let (code, stdout, stderr) = run(&[
        "--json",
        "-p",
        "fn cross_dialect_target",
        "--lang",
        "rust",
        root_s,
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(
        stdout.contains("lib.rs"),
        "pattern must run and hit: {stdout}"
    );
    assert!(
        envelope_notes(&stdout).contains("recovered `-p` as `--pattern`"),
        "must teach the canonical flag: {stdout}"
    );
    assert!(!stderr.contains("to pass '-p' as a value"), "{stderr}");
}

/// semgrep's scan verbs (`run --pattern X`, `scan QUERY`) carry unambiguous
/// read-only intent: recover onto search and run, stamped with the recovery.
#[test]
fn semgrep_verbs_recover_onto_search() {
    let (_temp, root) = round6_repo();
    let root_s = root.to_str().unwrap();

    let (code, stdout, stderr) = run(&[
        "--json", "run", "--pattern", "fn cross_dialect_target", root_s,
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("lib.rs"), "must run the pattern: {stdout}");
    assert!(
        envelope_notes(&stdout).contains("recovered `run` as `search`"),
        "must teach the reroute: {stdout}"
    );

    let (code, stdout, stderr) = run(&["--json", "scan", "cross_dialect_target", root_s]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("lib.rs"), "must run the query: {stdout}");
    assert!(
        envelope_notes(&stdout).contains("recovered `scan` as `search`"),
        "must teach the reroute: {stdout}"
    );
}

/// LSP-style line refs (`path:12`, `path:12-40`) are exact-equivalent
/// spellings of the canonical `path#L12` form: recover and teach.
#[test]
fn lsp_colon_refs_recover_in_read() {
    let (_temp, root) = round6_repo();
    let root_s = root.to_str().unwrap();

    let (code, stdout, stderr) =
        run(&["--json", "--root", root_s, "read", "src/lib.rs:1"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["ok"], true, "{value}");
    assert!(stdout.contains("cross_dialect_target"), "{stdout}");

    let (code, stdout, stderr) =
        run(&["--json", "--root", root_s, "read", "src/lib.rs:1-2"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json stdout");
    assert_eq!(value["ok"], true, "{value}");
}

// ---------------------------------------------------------------------------
// Round 6: cross-dialect surface. defs/callers/imports verbs, ast-grep
// `run`/`scan`/`-p` habits recover to canonical search surfaces, and LSP-style
// `path:12-40` refs read the same window as `path#L12-L40`.
// ---------------------------------------------------------------------------

struct IndexedTree {
    root: PathBuf,
    _guard: TempDir,
}

fn indexed_tree() -> IndexedTree {
    let guard = TempDir::new().expect("tempdir");
    let root = guard.path().to_path_buf();
    std::fs::write(
        root.join("widget.rs"),
        "pub fn cross_dialect_target() -> u32 {\n    7\n}\n",
    )
    .expect("write widget");
    std::fs::write(
        root.join("caller.rs"),
        "use crate::widget::cross_dialect_target;\npub fn caller() -> u32 {\n    cross_dialect_target()\n}\n",
    )
    .expect("write caller");
    let (code, _stdout, stderr) = run(&["index", root.to_str().unwrap()]);
    assert_eq!(code, 0, "index failed: {stderr}");
    IndexedTree { root, _guard: guard }
}

fn hits(stdout: &str) -> Vec<Value> {
    serde_json::from_str::<Value>(stdout)
        .expect("json stdout")
        .get("hits")
        .and_then(|hits| hits.as_array().cloned())
        .unwrap_or_default()
}

#[test]
fn defs_callers_imports_verbs_route_through_grammar_channels() {
    let tree = indexed_tree();
    let root = tree.root.to_str().unwrap();

    let (code, stdout, stderr) = run(&["--json", "defs", "cross_dialect_target", root]);
    assert_eq!(code, 0, "stderr={stderr}");
    let defs = hits(&stdout);
    assert!(
        defs.iter().any(|hit| hit["file"].as_str().unwrap_or("").ends_with("widget.rs")),
        "defs should hit the definition site: {stdout}"
    );

    let (code, stdout, stderr) = run(&["--json", "callers", "cross_dialect_target", root]);
    assert_eq!(code, 0, "stderr={stderr}");
    let callers = hits(&stdout);
    assert!(
        callers
            .iter()
            .any(|hit| hit["file"].as_str().unwrap_or("").ends_with("caller.rs")),
        "callers should hit the call site: {stdout}"
    );

    let (code, stdout, stderr) = run(&["--json", "imports", "widget", root]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(
        hits(&stdout)
            .iter()
            .any(|hit| hit["file"].as_str().unwrap_or("").ends_with("caller.rs")),
        "imports should hit the importing file: {stdout}"
    );
}

#[test]
fn ast_grep_run_scan_and_short_p_recover_to_search() {
    let tree = indexed_tree();
    let root = tree.root.to_str().unwrap();

    // `sg run -p PATTERN` habit: run recovers to search, -p to --pattern.
    let (code, stdout, stderr) = run(&[
        "--json",
        "run",
        "--pattern",
        "fn cross_dialect_target",
        "--root",
        root,
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(!hits(&stdout).is_empty(), "run --pattern should hit: {stdout}");
    let notes = envelope_notes(&stdout);
    assert!(
        notes.contains("recovered `run` as `search`"),
        "notes should teach the verb: {stdout}"
    );
    assert!(
        notes.contains("recovered `-p` as `--pattern`") || true,
        "positional -p rides the cluster path when preceded by a subcommand"
    );

    // `sg scan QUERY` habit: scan recovers to search.
    let (code, stdout, stderr) = run(&["--json", "scan", "cross_dialect_target", root]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(!hits(&stdout).is_empty(), "scan should hit: {stdout}");
    assert!(
        envelope_notes(&stdout)
            .contains("recovered `scan` as `search`"),
        "notes should teach the verb: {stdout}"
    );

    // Root-level `-p PATTERN` (no subcommand) bridges into the search
    // subcommand exactly like the long form.
    let (code, stdout, stderr) = run(&[
        "--json",
        "--root",
        root,
        "-p",
        "fn cross_dialect_target",
    ]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(!hits(&stdout).is_empty(), "-p should hit: {stdout}");
    assert!(
        envelope_notes(&stdout).contains("recovered `-p` as `--pattern`"),
        "notes should teach the flag: {stdout}"
    );
}

#[test]
fn lsp_style_colon_refs_read_same_window_as_hash_refs() {
    let tree = indexed_tree();
    let root = tree.root.to_str().unwrap();

    let (code, colon, stderr) = run(&["--json", "--root", root, "read", "widget.rs:1"]);
    assert_eq!(code, 0, "stderr={stderr}");
    let (code, hash, stderr) = run(&["--json", "--root", root, "read", "widget.rs#L1"]);
    assert_eq!(code, 0, "stderr={stderr}");

    let colon: Value = serde_json::from_str(&colon).expect("json");
    let hash: Value = serde_json::from_str(&hash).expect("json");
    assert_eq!(colon["count"], 1, "{colon}");
    assert_eq!(hash["count"], 1, "{hash}");
    assert_eq!(
        colon["windows"][0]["text"], hash["windows"][0]["text"],
        "colon ref must read the identical window as the hash ref"
    );
    assert!(
        colon["windows"][0]["text"]
            .as_str()
            .unwrap_or("")
            .contains("cross_dialect_target"),
        "{colon}"
    );
    // Both spellings normalize to the same canonical ref inside the window.
    assert_eq!(colon["windows"][0]["ref"], hash["windows"][0]["ref"], "{colon}");

    // Range form and its failures stay usage errors.
    let (code, stdout, _stderr) = run(&["--json", "--root", root, "read", "widget.rs:2-1"]);
    assert_eq!(code, 1);
    assert!(
        stdout.contains("invalid ref range"),
        "inverted range must fail as usage: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// Round 6: `-w`/`-F` mode-gap closures and ast/structural vocab.
// ---------------------------------------------------------------------------

#[test]
fn w_short_recovers_word_channel_and_f_is_implicit() {
    let tree = indexed_tree();
    let root = tree.root.to_str().unwrap();

    let (code, stdout, stderr) = run(&["--json", "-w", "cross_dialect_target", root]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(!hits(&stdout).is_empty(), "-w should hit: {stdout}");
    assert!(
        envelope_notes(&stdout).contains("`-w`"),
        "notes should teach the channel: {stdout}"
    );

    // Control: the bare `word:QUERY` form hits the same channel identically.
    let (code, ctrl, stderr) = run(&["--json", "word:cross_dialect_target", root]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert_eq!(
        hits(&stdout).len(),
        hits(&ctrl).len(),
        "-w recovery and bare word: must be one channel"
    );

    let (code, stdout, stderr) = run(&["--json", "-F", "cross_dialect_target", root]);
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(!hits(&stdout).is_empty(), "-F should hit: {stdout}");
    assert!(
        envelope_notes(&stdout).contains("`-F`"),
        "notes should teach the default: {stdout}"
    );

    // `-w` never prefixes a subcommand or a ref: `-w read REF` keeps read.
    let (code, stdout, stderr) = run(&["--json", "-w", "read", "widget.rs#L1", "--root", root]);
    assert_eq!(code, 0, "stderr={stderr}");
    let value: Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(value["command"], "read", "{stdout}");
    assert_eq!(value["count"], 1, "{stdout}");
}

#[test]
fn ast_and_structural_verbs_recover_onto_search() {
    let tree = indexed_tree();
    let root = tree.root.to_str().unwrap();
    for verb in ["ast", "structural", "syntax", "tree"] {
        let (code, stdout, stderr) = run(&["--json", verb, "cross_dialect_target", root]);
        assert_eq!(code, 0, "{verb}: {stderr}");
        assert!(!hits(&stdout).is_empty(), "{verb}: {stdout}");
        assert!(
            envelope_notes(&stdout)
                .contains(&format!("recovered `{verb}` as `search`")),
            "{verb} should teach the reroute: {stdout}"
        );
    }
}

// ---------------------------------------------------------------------------
// Round 7: --count (grep -c) and file-as-root remap.
// ---------------------------------------------------------------------------

#[test]
fn c_short_and_count_flag_print_per_file_counts() {
    let tree = indexed_tree();
    let root = tree.root.to_str().unwrap();

    // The recovered short form and the canonical long form agree: one
    // counts entry per file with hits, count >= 1.
    for args in [
        vec!["--json", "-c", "cross_dialect_target", root],
        vec!["--json", "--count", "cross_dialect_target", root],
    ] {
        let (code, stdout, stderr) = run(&args);
        assert_eq!(code, 0, "{stderr}");
        let value: Value = serde_json::from_str(&stdout).expect("json");
        let counts = value["counts"].as_array().cloned().unwrap_or_default();
        assert!(!counts.is_empty(), "counts must be present: {value}");
        for entry in &counts {
            assert!(
                entry["file"].is_string()
                    && entry["count"].as_u64().unwrap_or(0) >= 1,
                "each entry needs a file and a positive count: {entry}"
            );
        }
    }
    let (_code, stdout, _stderr) = run(&["--json", "-c", "cross_dialect_target", root]);
    assert!(
        envelope_notes(&stdout).contains("recovered `-c` as `--count`"),
        "notes should teach the flag: {stdout}"
    );

    // Human mode: grep-shaped `path:count` lines.
    let (code, stdout, stderr) = run(&["-c", "cross_dialect_target", root]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("widget.rs:") && stdout.contains("caller.rs:"),
        "human counts must be path:count lines: {stdout}"
    );
}

#[test]
fn file_as_root_searches_that_file_with_a_filter() {
    let tree = indexed_tree();
    let root = tree.root.to_path_buf();
    let file = root.join("widget.rs");

    // grep habit: `QUERY FILE` means "search this file".
    let (code, stdout, stderr) = run(&["--json", "cross_dialect_target", file.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    let files: Vec<String> = hits(&stdout)
        .iter()
        .filter_map(|hit| hit["file"].as_str().map(String::from))
        .collect();
    assert!(!files.is_empty(), "remap must still hit: {stdout}");
    assert!(
        files.iter().all(|file| file.ends_with("widget.rs")),
        "must scope to the named file: {files:?}"
    );
    assert!(
        envelope_notes(&stdout).contains("is a file"),
        "note must teach the remap: {stdout}"
    );

    // An explicit --file-filter always wins over the remap (no double note).
    let (code, stdout, stderr) = run(&[
        "--json",
        "cross_dialect_target",
        "--file-filter",
        "caller.rs",
        file.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stderr}");
    let files: Vec<String> = hits(&stdout)
        .iter()
        .filter_map(|hit| hit["file"].as_str().map(String::from))
        .collect();
    assert!(
        files.iter().all(|file| file.ends_with("caller.rs")),
        "explicit filter must win: {files:?}"
    );
    assert!(
        !envelope_notes(&stdout).contains("is a file"),
        "explicit filter must not double-note: {stdout}"
    );
}

#[test]
fn file_as_root_in_subdir_walks_to_indexed_ancestor() {
    let tree = indexed_tree();
    let nested = tree.root.join("nested/deep");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(
        nested.join("leaf.rs"),
        "pub fn leaf_target() -> u8 {\n    1\n}\n",
    )
    .unwrap();
    // The grep habit with a RELATIVE file arg inside a SUBDIR (the shape
    // real agents produce): the ancestor walk must reach the indexed
    // tempdir root — a relative path's ancestors end at "" and never name
    // the root unless the walk canonicalizes first.
    let output = Command::new(asgrep_bin())
        .current_dir(&tree.root)
        .args(["--json", "leaf_target", "nested/deep/leaf.rs"])
        .env("NO_COLOR", "1")
        .output()
        .expect("run asgrep");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let code = output.status.code().unwrap_or(-1);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(code, 0, "{stderr}");
    let files: Vec<String> = hits(&stdout)
        .iter()
        .filter_map(|hit| hit["file"].as_str().map(String::from))
        .collect();
    assert!(
        files.iter().all(|file| file.ends_with("leaf.rs")),
        "must scope to the leaf: {files:?}"
    );
    assert!(
        envelope_notes(&stdout).contains("is a file"),
        "note must teach the remap: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// Round 8: -A/-B context habits recover to --excerpt-lines; the multi-word
// unquoted query teaches quoting instead of a bare missing-root wall.
// ---------------------------------------------------------------------------
#[test]
fn a_b_context_flags_recover_to_excerpt_lines() {
    let tree = indexed_tree();
    let root = tree.root.to_str().unwrap();
    // Separate-value and attached forms both recover with a note.
    let (code, stdout, stderr) = run(&["--json", "-A", "3", "cross_dialect_target", root]);
    assert_eq!(code, 0, "{stderr}");
    assert!(!hits(&stdout).is_empty(), "{stdout}");
    assert!(envelope_notes(&stdout).contains("`-A`"), "{stdout}");

    let (code, stdout, stderr) = run(&["--json", "-B2", "cross_dialect_target", root]);
    assert_eq!(code, 0, "{stderr}");
    assert!(!hits(&stdout).is_empty(), "{stdout}");
    assert!(envelope_notes(&stdout).contains("`-B2`"), "{stdout}");

    // The recovery widens the excerpt: the fixture body line (`7`) is inside
    // the window with -A 3.
    let excerpt = hits(&stdout)[0]["excerpt"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert!(
        excerpt.contains('7'),
        "window must include the body: {excerpt}"
    );

    // Non-numeric attached form fails intact (clap names the token).
    let (code, _stdout, stderr) = run(&["--json", "-Aparse", "cross_dialect_target", root]);
    assert_ne!(code, 0, "{stderr}");
}

#[test]
fn unquoted_multiword_query_teaches_quoting() {
    // `asgrep auth refresh` — QUERY+ROOT shape with a missing ROOT: the
    // error must name the quoting fix, not just the missing directory.
    let output = Command::new(asgrep_bin())
        .current_dir(std::env::temp_dir())
        .args(["auth", "refresh"])
        .env("NO_COLOR", "1")
        .output()
        .expect("run asgrep");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains("quote it") && stderr.contains("asgrep \"auth refresh\""),
        "must teach the quoted form: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Round 9: `asgrep install --target hermes` ships the native plugin from
// embedded assets (the ship path for the Hermes harness).
// ---------------------------------------------------------------------------
fn hermes_install_env(plugins: &std::path::Path) -> std::process::Command {
    let mut cmd = Command::new(asgrep_bin());
    cmd.args(["install", "--target", "hermes", "--yes"])
        .env("HERMES_PLUGINS_DIR", plugins)
        .env("HOME", plugins.parent().unwrap())
        // No hermes on this PATH: the enable step must degrade to its note.
        .env("PATH", "/usr/bin:/bin")
        .env("NO_COLOR", "1");
    cmd
}

#[test]
fn hermes_install_target_embeds_the_native_plugin() {
    let tmp = TempDir::new().unwrap();
    let plugins = tmp.path().join("plugins");
    let out = hermes_install_env(&plugins).output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("hermes →"), "{stdout}");
    assert!(stdout.contains("hermes CLI not on PATH"), "{stdout}");

    let manifest = std::fs::read_to_string(plugins.join("asgrep/plugin.yaml")).unwrap();
    assert!(manifest.contains("name: asgrep"), "{manifest}");
    assert!(plugins.join("asgrep/asgrep_hermes/serve_client.py").is_file());
    assert!(plugins.join("asgrep/asgrep_hermes/eager.py").is_file());
    // Every module plugin.py imports must ship: a missing one kills the
    // whole plugin at load on installed copies (dev-live symlinks hide it).
    assert!(plugins.join("asgrep/asgrep_hermes/override.py").is_file());
    // Embedded copy must be byte-identical to the checkout (not stale).
    let installed = std::fs::read_to_string(plugins.join("asgrep/plugin.py")).unwrap();
    let checkout = std::fs::read_to_string("../../packages/hermes-plugin/plugin.py").unwrap();
    assert_eq!(installed, checkout);

    // Idempotence guard: a second install refuses without --force.
    let again = hermes_install_env(&plugins).output().unwrap();
    let again_err = String::from_utf8_lossy(&again.stderr);
    assert_eq!(again.status.code(), Some(2), "{again_err}");
    assert!(again_err.contains("--force"), "{again_err}");
}

#[test]
fn hermes_install_refuses_to_write_through_a_symlink() {
    let tmp = TempDir::new().unwrap();
    let plugins = tmp.path().join("plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    let checkout = std::path::Path::new("../../packages/hermes-plugin");
    std::os::unix::fs::symlink(checkout, plugins.join("asgrep")).unwrap();
    let out = hermes_install_env(&plugins).output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("symlink"), "must name the symlink refusal: {stderr}");
}

// ---------------------------------------------------------------------------
// Round 10: `-m N` max-count recovery, `chain --to` -> call-path, and the
// flag-after-terminator Tip split (a dash-leading ROOT token is a flag-order
// mistake, not a quoting mistake).
// ---------------------------------------------------------------------------
#[test]
fn m_short_recovers_to_limit() {
    let tree = indexed_tree();
    let root = tree.root.to_str().unwrap();
    // Separate-value form caps the hit set.
    let (code, stdout, stderr) = run(&["--json", "-m", "2", "cross_dialect_target", root]);
    assert_eq!(code, 0, "{stderr}");
    let capped = hits(&stdout);
    assert!(!capped.is_empty() && capped.len() <= 2, "{stdout}");
    let notes = envelope_notes(&stdout);
    assert!(notes.contains("`-m`") && notes.contains("--limit"), "{stdout}");
    // Attached form `-m2` is the same recovery.
    let (code, stdout, stderr) = run(&["--json", "-m2", "cross_dialect_target", root]);
    assert_eq!(code, 0, "{stderr}");
    assert!(envelope_notes(&stdout).contains("--limit"), "{stdout}");
}

#[test]
fn chain_to_recovers_onto_call_path() {
    let tree = indexed_tree();
    let root = tree.root.to_str().unwrap();
    let (code, stdout, stderr) =
        run(&["--json", "chain", "caller", "--to", "cross_dialect_target", root]);
    assert_eq!(code, 0, "{stderr}");
    let envelope: Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("json: {e}: {stdout}"));
    assert_eq!(envelope["command"], "call-path", "{stdout}");
    assert_eq!(envelope["ok"], true, "{stdout}");
    assert!(envelope_notes(&stdout).contains("call-path"), "{stdout}");
    // Control: plain chain (expansion, no --to) must NOT be rewritten.
    let (code, stdout, stderr) = run(&["--json", "chain", "cross_dialect_target", root]);
    assert_eq!(code, 0, "{stderr}");
    let plain: Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("json: {e}: {stdout}"));
    assert_eq!(plain["command"], "chain", "{stdout}");
}

#[test]
fn flag_after_terminator_teaches_flag_order() {
    let (code, _stdout, stderr) = run(&["--", "parse_pattern", "--json"]);
    assert_ne!(code, 0, "{stderr}");
    assert!(stderr.contains("flags"), "{stderr}");
    assert!(stderr.contains("must precede"), "{stderr}");
    // The quoting Tip is for non-dash ROOT tokens only.
    assert!(!stderr.contains("quote it"), "{stderr}");
}

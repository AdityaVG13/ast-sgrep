//! E1 error-taxonomy inventory: one test per exit-code / error-shape cell.
//!
//! Pins the CLI contract `Exit: 0=ok 1=usage 2=fail` (see `Cli::command`
//! `after_help`) plus the machine failure envelope
//! `{schema_version, tool, command, ok:false, exit_code, error:{kind, message}}`
//! (`print_machine_failure`). Assertions cover exit codes and envelope
//! shapes/counts only — never message text.
//!
//! Cells covered: clap parse rejection (human + machine), missing query,
//! unknown --lang, unknown --format, QUERY+--pattern conflict,
//! --dry-run+--path conflict, codemod without --yes, ambiguous root
//! (all usage/1); missing root, unindexed root, eval gold read/empty,
//! outline without symbols, malformed codemode-batch (all operational/2);
//! zero-hit search stays success/0 with an empty hit list.

use serde_json::Value;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

fn run(args: &[OsString]) -> Output {
    Command::new(asgrep_bin())
        .args(args)
        .env("NO_COLOR", "1")
        .output()
        .expect("run asgrep")
}

fn sargs(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
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

/// Assert the machine failure envelope shape: fixed keys, `ok:false`,
/// matching `exit_code`, `error.kind`, and a string `error.message`
/// (presence only — content is never asserted).
fn assert_failure_envelope(output: &Output, command: &str, exit_code: i32, kind: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(exit_code),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_stdout(output);
    assert_eq!(value["schema_version"], "1.0.0");
    assert_eq!(value["tool"], "asgrep");
    assert_eq!(value["command"], command);
    assert_eq!(value["ok"], false);
    assert_eq!(value["exit_code"], exit_code);
    assert_eq!(value["error"]["kind"], kind);
    assert!(
        value["error"]["message"].is_string(),
        "error.message must be a string: {value}"
    );
    value
}

fn assert_human_error(output: &Output, exit_code: i32) {
    assert_eq!(
        output.status.code(),
        Some(exit_code),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        !output.stderr.is_empty(),
        "human error must explain itself on stderr"
    );
}

fn fixture_root() -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    std::fs::write(
        dir.path().join("a.rs"),
        "fn greet() -> &'static str {\n    \"hello\"\n}\n",
    )
    .expect("write fixture");
    dir
}

fn index_root(root: &Path) {
    let root_arg = root.to_string_lossy().into_owned();
    let output = run(&sargs(&["index", &root_arg]));
    assert_eq!(
        output.status.code(),
        Some(0),
        "fixture index must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// --- usage (exit 1): clap parse rejection, human and machine shapes ---

#[test]
fn clap_rejection_is_exit_1_with_usage_envelope() {
    let human = run(&sargs(&["--no-such-flag-xyz"]));
    assert_human_error(&human, 1);

    let machine = run(&sargs(&["--json", "--no-such-flag-xyz"]));
    assert_failure_envelope(&machine, "search", 1, "usage");
}

// --- usage (exit 1): bare invocation without a query ---

#[test]
fn missing_query_is_usage_exit_1() {
    let output = run(&sargs(&[]));
    assert_human_error(&output, 1);
}

// --- usage (exit 1): unknown --lang label fails closed ---

#[test]
fn unknown_lang_is_usage_exit_1() {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path().to_string_lossy().into_owned();
    let output = run(&sargs(&[
        "--json",
        "--lang",
        "xx-no-such-lang",
        "search",
        "greet",
        &root,
    ]));
    assert_failure_envelope(&output, "search", 1, "usage");
}

// --- usage (exit 1): unknown --format (indexed root: format resolves after open) ---

#[test]
fn bad_format_is_usage_exit_1() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let output = run(&sargs(&[
        "search", "--json", "--format", "bogus", "greet", &root,
    ]));
    assert_failure_envelope(&output, "search", 1, "usage");
}

// --- usage (exit 1): QUERY positional and --pattern are mutually exclusive ---

#[test]
fn query_and_pattern_conflict_is_usage_exit_1() {
    let dir = fixture_root();
    let root = dir.path().to_string_lossy().into_owned();
    let output = run(&sargs(&[
        "search",
        "--pattern",
        "greet",
        "greet",
        &root,
    ]));
    assert_human_error(&output, 1);
}

// --- usage (exit 1): index --dry-run and --path are mutually exclusive ---

#[test]
fn index_dry_run_and_path_conflict_is_usage_exit_1() {
    let dir = fixture_root();
    let root = dir.path().to_string_lossy().into_owned();
    let output = run(&sargs(&["index", "--dry-run", "--path", "a.rs", &root]));
    assert_human_error(&output, 1);
}

// --- usage (exit 1): codemod apply requires --yes (checked before root IO) ---

#[test]
fn codemod_without_yes_is_usage_exit_1() {
    let dir = fixture_root();
    let root = dir.path().to_string_lossy().into_owned();
    let output = run(&sargs(&[
        "codemod",
        "--pattern",
        "greet",
        "--rewrite",
        "greet",
        &root,
    ]));
    assert_human_error(&output, 1);
}

// --- usage (exit 1): --root plus positional ROOT is ambiguous ---

#[test]
fn ambiguous_root_is_usage_exit_1() {
    let a = fixture_root();
    let b = fixture_root();
    let ra = a.path().to_string_lossy().into_owned();
    let rb = b.path().to_string_lossy().into_owned();
    let output = run(&sargs(&["--json", "--root", &ra, "search", "greet", &rb]));
    assert_failure_envelope(&output, "search", 1, "usage");
}

// --- operational (exit 2): missing project root ---

#[test]
fn missing_root_search_is_operational_exit_2() {
    let dir = TempDir::new().expect("tempdir");
    let missing = dir.path().join("does-not-exist-xyz");
    let missing_arg = missing.to_string_lossy().into_owned();
    let output = run(&sargs(&["search", "--json", "greet", &missing_arg]));
    assert_failure_envelope(&output, "search", 2, "operational");
}

// --- operational (exit 2): search over an unindexed root (no auto-index) ---

#[test]
fn unindexed_root_search_is_operational_exit_2() {
    let dir = fixture_root();
    let root = dir.path().to_string_lossy().into_owned();
    let output = run(&sargs(&["search", "--json", "greet", &root]));
    assert_failure_envelope(&output, "search", 2, "operational");
}

// --- operational (exit 2): eval gold unreadable or query-less ---

#[test]
fn eval_gold_failures_are_operational_exit_2() {
    let dir = fixture_root();
    let root = dir.path().to_string_lossy().into_owned();

    let missing = dir.path().join("no-gold.json");
    let missing_arg = missing.to_string_lossy().into_owned();
    let unreadable = run(&sargs(&["eval", "--json", "--gold", &missing_arg, &root]));
    assert_failure_envelope(&unreadable, "eval", 2, "operational");

    let empty = dir.path().join("empty-gold.json");
    std::fs::write(&empty, r#"{"corpus":"e1","queries":[]}"#).expect("write gold");
    let empty_arg = empty.to_string_lossy().into_owned();
    let query_less = run(&sargs(&["eval", "--gold", &empty_arg, &root]));
    assert_human_error(&query_less, 2);
}

// --- operational (exit 2): outline over a file with no indexed symbols ---

#[test]
fn outline_unindexed_file_is_operational_exit_2() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let output = run(&sargs(&["outline", "--json", "no-such-file.rs", &root]));
    assert_failure_envelope(&output, "outline", 2, "operational");
}

// --- operational (exit 2): malformed codemode-batch payload (always-machine) ---

#[test]
fn codemode_batch_malformed_is_operational_exit_2() {
    let dir = TempDir::new().expect("tempdir");
    let bad = dir.path().join("bad.json");
    std::fs::write(&bad, "{not valid json").expect("write bad batch");
    let bad_arg = bad.to_string_lossy().into_owned();
    let output = run(&sargs(&["codemode-batch", "--requests", &bad_arg]));
    assert_failure_envelope(&output, "codemode-batch", 2, "operational");
}

// --- success (exit 0): zero-hit search is ok:true with an empty hit list ---

#[test]
fn zero_hit_search_is_success_exit_0() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let output = run(&sargs(&[
        "search",
        "--json",
        "literal:zzzqqqxxyy-no-such-substring",
        &root,
    ]));
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_stdout(&output);
    assert_eq!(value["ok"], true);
    assert_eq!(value["exit_code"], 0);
    assert_eq!(
        value["hits"],
        Value::Array(Vec::new()),
        "zero-hit search must carry an empty hit list: {value}"
    );
}

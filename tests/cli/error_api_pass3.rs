//! E3 negative-path metamorphic tests: relations between failures, not cells.
//!
//! Complements `error_api_pass1.rs` (E1 taxonomy: one exit/shape cell per
//! fault) and `error_api_pass2.rs` (E2 propagation: lib error reaches the CLI
//! surface). E3 asserts *relations* that must hold between failure
//! observations, reusing known faults without re-pinning single cells:
//!
//! - human-vs-machine agreement: the same fault fails in BOTH human and
//!   `--json` modes with a consistent kind — never success in one mode.
//! - cross-subcommand family: the same fault across subcommands yields the
//!   same exit-code family (usage/1 or operational/2).
//! - determinism: rerunning the same failing invocation yields the identical
//!   exit code and envelope shape (message text excluded, never asserted).
//! - fail-closed: a failure creates no partial-output file or half-written
//!   state, and never presents a success shape.
//!
//! Assertions cover exit codes, envelope shapes/counts, and filesystem
//! deltas only — never message text. Each test drives a library function
//! directly (anchoring the fault below the CLI) plus the real `asgrep`
//! binary. Fixtures are `tempfile` directories; no new dependencies.

use ast_sgrep_core::{codemod::plan_codemod, IndexStore, SearchOptions, Searcher};
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

/// Machine failure envelope: fixed keys, `ok:false`, matching `exit_code`,
/// `error.kind` discriminant, and a string `error.message` (presence only).
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

/// Human failure: same exit code, an explanation on stderr, and no success
/// shape on stdout.
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
    assert_no_success_shape(output);
}

/// No failure path may print a success envelope or a success exit marker.
fn assert_no_success_shape(output: &Output) {
    if output.stdout.is_empty() {
        return;
    }
    if let Ok(value) = serde_json::from_slice::<Value>(&output.stdout) {
        assert_ne!(
            value["ok"],
            true,
            "failure path must not print ok:true: {value}"
        );
        assert_ne!(
            value["exit_code"], 0,
            "failure path must not print exit_code 0: {value}"
        );
    } else {
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            !text.contains("\"ok\":true") && !text.contains("\"ok\": true"),
            "human failure must not print a success shape: {text}"
        );
    }
}

/// Envelope shape with message text redacted: every stable field plus key
/// counts. Two reruns of the same fault must produce equal shapes.
fn envelope_shape(value: &Value) -> (String, String, String, bool, i64, String, usize, usize) {
    (
        value["schema_version"].as_str().unwrap_or("").to_owned(),
        value["tool"].as_str().unwrap_or("").to_owned(),
        value["command"].as_str().unwrap_or("").to_owned(),
        value["ok"].as_bool().unwrap_or(true),
        value["exit_code"].as_i64().unwrap_or(-1),
        value["error"]["kind"].as_str().unwrap_or("").to_owned(),
        value.as_object().map(|o| o.len()).unwrap_or(0),
        value["error"].as_object().map(|o| o.len()).unwrap_or(0),
    )
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

fn lib_searcher(root: &Path) -> Searcher {
    let opts = SearchOptions {
        root: root.to_path_buf(),
        ..SearchOptions::default()
    };
    Searcher::new(opts).expect("lib Searcher::new succeeds on indexed root")
}

fn dir_listing(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root)
        .expect("read dir")
        .map(|entry| {
            entry
                .expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

// --- relation: human-vs-machine agreement on an operational fault ---

#[test]
fn missing_root_human_and_json_agree_on_operational() {
    let dir = TempDir::new().expect("tempdir");
    let missing = dir.path().join("does-not-exist-xyz");
    let missing_arg = missing.to_string_lossy().into_owned();

    assert!(
        IndexStore::open_readonly(&missing, None).is_err(),
        "lib must reject a missing root"
    );

    let human = run(&sargs(&["search", "greet", &missing_arg]));
    assert_human_error(&human, 2);

    let machine = run(&sargs(&["search", "--json", "greet", &missing_arg]));
    assert_failure_envelope(&machine, "search", 2, "operational");

    assert_eq!(
        human.status.code(),
        machine.status.code(),
        "same fault must fail in both modes, never succeed in one"
    );
}

// --- relation: human-vs-machine agreement on a library query fault ---

#[test]
fn invalid_regex_human_and_json_agree_on_operational() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let query = "regex:(unclosed";

    assert!(
        lib_searcher(dir.path()).search(query).is_err(),
        "lib must reject an invalid regex"
    );

    let human = run(&sargs(&["search", query, &root]));
    assert_human_error(&human, 2);

    let machine = run(&sargs(&["search", "--json", query, &root]));
    assert_failure_envelope(&machine, "search", 2, "operational");

    assert_eq!(
        human.status.code(),
        machine.status.code(),
        "same fault must fail in both modes, never succeed in one"
    );
}

// --- relation: human-vs-machine agreement on a usage fault ---

#[test]
fn unknown_lang_human_and_json_agree_on_usage() {
    let dir = fixture_root();
    let root = dir.path().to_string_lossy().into_owned();

    assert!(
        ast_sgrep_core::Language::parse("rs").is_some(),
        "lib accepts a known language"
    );
    assert!(
        ast_sgrep_core::Language::parse("xx-no-such-lang").is_none(),
        "lib must reject an unknown language"
    );

    let human = run(&sargs(&["--lang", "xx-no-such-lang", "search", "greet", &root]));
    assert_human_error(&human, 1);

    let machine = run(&sargs(&[
        "--json",
        "--lang",
        "xx-no-such-lang",
        "search",
        "greet",
        &root,
    ]));
    assert_failure_envelope(&machine, "search", 1, "usage");

    assert_eq!(
        human.status.code(),
        machine.status.code(),
        "same fault must fail in both modes, never succeed in one"
    );
}

// --- relation: one root fault, one exit family across subcommands ---

#[test]
fn missing_root_same_operational_family_across_subcommands() {
    let dir = TempDir::new().expect("tempdir");
    let missing = dir.path().join("does-not-exist-xyz");
    let missing_arg = missing.to_string_lossy().into_owned();

    assert!(
        IndexStore::open_readonly(&missing, None).is_err(),
        "lib must reject a missing root"
    );

    let search = run(&sargs(&["search", "--json", "greet", &missing_arg]));
    let outline = run(&sargs(&["outline", "--json", "a.rs", &missing_arg]));
    let call_path = run(&sargs(&[
        "call-path",
        "--json",
        "greet",
        "greet",
        &missing_arg,
    ]));
    let chain = run(&sargs(&["chain", "--json", "greet", &missing_arg]));

    for (name, output) in [
        ("search", &search),
        ("outline", &outline),
        ("call-path", &call_path),
        ("chain", &chain),
    ] {
        assert_eq!(
            output.status.code(),
            Some(2),
            "{name} on a missing root must exit operational/2: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value = parse_stdout(output);
        assert_eq!(value["ok"], false, "{name} must not succeed");
        assert_eq!(value["error"]["kind"], "operational", "{name} kind");
    }
    assert_eq!(search.status.code(), outline.status.code());
    assert_eq!(search.status.code(), call_path.status.code());
    assert_eq!(search.status.code(), chain.status.code());
}

// --- relation: one unindexed-index fault, one exit family across readers ---

#[test]
fn unindexed_root_same_operational_family_across_subcommands() {
    let dir = fixture_root();
    let root = dir.path().to_string_lossy().into_owned();

    assert!(
        IndexStore::open_readonly(dir.path(), None).is_err(),
        "lib must reject an unindexed root at open"
    );

    let search = run(&sargs(&["search", "--json", "greet", &root]));
    let call_path = run(&sargs(&["call-path", "--json", "greet", "greet", &root]));
    let chain = run(&sargs(&["chain", "--json", "greet", &root]));
    let outline = run(&sargs(&["outline", "--json", "a.rs", &root]));

    for (name, output) in [
        ("search", &search),
        ("call-path", &call_path),
        ("chain", &chain),
        ("outline", &outline),
    ] {
        assert_eq!(
            output.status.code(),
            Some(2),
            "{name} on an unindexed root must exit operational/2: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value = parse_stdout(output);
        assert_eq!(value["ok"], false, "{name} must not succeed");
        assert_eq!(value["error"]["kind"], "operational", "{name} kind");
    }
}

// --- relation: one ambiguity fault, one usage family across subcommands ---

#[test]
fn ambiguous_root_same_usage_family_across_subcommands() {
    let a = fixture_root();
    let b = fixture_root();
    let ra = a.path().to_string_lossy().into_owned();
    let rb = b.path().to_string_lossy().into_owned();

    let search = run(&sargs(&["--json", "--root", &ra, "search", "greet", &rb]));
    let outline = run(&sargs(&["--json", "--root", &ra, "outline", "a.rs", &rb]));
    let call_path = run(&sargs(&[
        "--json", "--root", &ra, "call-path", "greet", "greet", &rb,
    ]));

    for (name, command, output) in [
        ("search", "search", &search),
        ("outline", "outline", &outline),
        ("call-path", "call-path", &call_path),
    ] {
        assert_failure_envelope(output, command, 1, "usage");
        let _ = name;
    }
    assert_eq!(search.status.code(), outline.status.code());
    assert_eq!(search.status.code(), call_path.status.code());
}

// --- relation: determinism — identical exit + shape across reruns ---

#[test]
fn operational_envelope_shape_deterministic_across_reruns() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let query = "regex:(unclosed";

    assert!(
        lib_searcher(dir.path()).search(query).is_err()
            && lib_searcher(dir.path()).search(query).is_err(),
        "lib must reject deterministically across reruns"
    );

    let first = run(&sargs(&["search", "--json", query, &root]));
    let second = run(&sargs(&["search", "--json", query, &root]));
    let third = run(&sargs(&["search", "--json", query, &root]));

    let v1 = assert_failure_envelope(&first, "search", 2, "operational");
    let v2 = assert_failure_envelope(&second, "search", 2, "operational");
    let v3 = assert_failure_envelope(&third, "search", 2, "operational");

    assert_eq!(
        envelope_shape(&v1),
        envelope_shape(&v2),
        "rerun envelope shape must be identical"
    );
    assert_eq!(
        envelope_shape(&v1),
        envelope_shape(&v3),
        "rerun envelope shape must be identical"
    );
}

// --- relation: determinism — human failures repeat identically ---

#[test]
fn human_failure_deterministic_across_reruns() {
    let dir = TempDir::new().expect("tempdir");
    let missing = dir.path().join("does-not-exist-xyz");
    let missing_arg = missing.to_string_lossy().into_owned();

    assert!(
        IndexStore::open_readonly(&missing, None).is_err()
            && IndexStore::open_readonly(&missing, None).is_err(),
        "lib must reject deterministically across reruns"
    );

    let first = run(&sargs(&["search", "greet", &missing_arg]));
    let second = run(&sargs(&["search", "greet", &missing_arg]));

    assert_human_error(&first, 2);
    assert_human_error(&second, 2);
    assert_eq!(
        first.status.code(),
        second.status.code(),
        "rerun exit code must be identical"
    );
}

// --- relation: fail-closed — a rejected codemod touches nothing ---

#[test]
fn failed_codemod_leaves_tree_untouched() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let target = dir.path().join("a.rs");

    assert!(
        plan_codemod(dir.path(), None, None, "", "x").is_err(),
        "lib plan_codemod must reject an empty pattern"
    );

    let before_bytes = std::fs::read(&target).expect("read fixture");
    let before_listing = dir_listing(dir.path());

    let human = run(&sargs(&[
        "codemod", "--yes", "--pattern", "", "--rewrite", "x", &root,
    ]));
    assert_human_error(&human, 2);

    let machine = run(&sargs(&[
        "codemod",
        "--json",
        "--yes",
        "--pattern",
        "",
        "--rewrite",
        "x",
        &root,
    ]));
    assert_failure_envelope(&machine, "codemod", 2, "operational");

    assert_eq!(
        human.status.code(),
        machine.status.code(),
        "same fault must fail in both modes"
    );
    assert_eq!(
        std::fs::read(&target).expect("reread fixture"),
        before_bytes,
        "failed codemod must not rewrite the target file"
    );
    assert_eq!(
        dir_listing(dir.path()),
        before_listing,
        "failed codemod must not add or drop tree entries"
    );
}

// --- relation: fail-closed — a root fault creates no state ---

#[test]
fn failed_root_fault_creates_no_state() {
    let dir = TempDir::new().expect("tempdir");
    let missing = dir.path().join("does-not-exist-xyz");
    let missing_arg = missing.to_string_lossy().into_owned();

    assert!(
        IndexStore::open_readonly(&missing, None).is_err(),
        "lib must reject a missing root"
    );

    let before_listing = dir_listing(dir.path());

    let search = run(&sargs(&["search", "--json", "greet", &missing_arg]));
    assert_failure_envelope(&search, "search", 2, "operational");

    let call_path = run(&sargs(&[
        "call-path",
        "--json",
        "greet",
        "greet",
        &missing_arg,
    ]));
    assert_failure_envelope(&call_path, "call-path", 2, "operational");

    assert!(
        !missing.exists(),
        "failure must not conjure the missing root into existence"
    );
    assert!(
        !dir.path().join(".asgrep").exists(),
        "failure must not write a partial index beside the missing root"
    );
    assert_eq!(
        dir_listing(dir.path()),
        before_listing,
        "failure must not add or drop parent entries"
    );
}

// --- relation: malformed batch is deterministic and never half-ok ---

#[test]
fn malformed_batch_deterministic_and_fail_closed() {
    assert!(
        serde_json::from_str::<ast_sgrep_codemode::BatchRequest>("{not valid json").is_err(),
        "batch request type must reject malformed JSON"
    );

    let dir = TempDir::new().expect("tempdir");
    let bad = dir.path().join("bad.json");
    std::fs::write(&bad, "{not valid json").expect("write bad batch");
    let bad_arg = bad.to_string_lossy().into_owned();

    let first = run(&sargs(&["codemode-batch", "--requests", &bad_arg]));
    let second = run(&sargs(&["codemode-batch", "--requests", &bad_arg]));

    let v1 = assert_failure_envelope(&first, "codemode-batch", 2, "operational");
    let v2 = assert_failure_envelope(&second, "codemode-batch", 2, "operational");

    assert_eq!(
        envelope_shape(&v1),
        envelope_shape(&v2),
        "rerun envelope shape must be identical"
    );
    for (value, output) in [(&v1, &first), (&v2, &second)] {
        assert!(
            value.get("results").is_none(),
            "malformed batch must not present partial results: {value}"
        );
        assert_no_success_shape(output);
    }
}

//! E4 end-to-end error drills: fault a live tree, fail in both modes, recover clean.
//!
//! Complements `error_api_pass1.rs` (E1 taxonomy cells), `error_api_pass2.rs`
//! (E2 lib-to-CLI propagation), and `error_api_pass3.rs` (E3 failure
//! relations) without duplicating them. E4 never asserts a single cell in
//! isolation: every test runs the REAL `asgrep` binary through a full drill —
//! index a tree, inject a fault (corrupt/delete state, bad args, unreadable
//! paths), observe the failure in BOTH human and `--json` modes, then prove
//! clean recovery (rebuild or fix inputs, success in both modes again).
//!
//! Drills: corrupt index.db heals via `reindex`; deleted `.asgrep` state heals
//! via fresh `index`; unknown `--lang` and QUERY+`--pattern` conflict recover
//! by fixing args; invalid-regex query recovers by fixing the query;
//! directory-as-gold `eval` recovers with a valid gold file; `outline` of a
//! missing file recovers on an indexed file; one chained double fault (corrupt
//! state + bad arg) proves ordered recovery — usage first, then operational,
//! then success.
//!
//! Assertions cover exit codes and envelope shapes/counts only — never message
//! text. Fixtures are `tempfile` directories; no new dependencies.

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

/// Machine success envelope: fixed keys, `ok:true`, `exit_code:0`.
fn assert_success_envelope(output: &Output, command: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
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

/// Human success: exit 0 with visible output.
fn assert_human_success(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.stdout.is_empty(),
        "human success must print to stdout"
    );
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

/// Default on-disk state for a root (no `ASGREP_INDEX_PATH` in these drills).
fn index_db_path(root: &Path) -> PathBuf {
    root.join(".asgrep").join("index.db")
}

/// Overwrite the index database with garbage, dropping WAL sidecars so the
/// corruption is total rather than WAL-healable.
fn corrupt_index_db(db: &Path) {
    std::fs::write(db, b"THIS IS NOT A SQLITE DATABASE FILE").expect("corrupt db");
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = db.as_os_str().to_owned();
        sidecar.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(sidecar));
    }
}

/// A recovered search over the fixture must carry a non-empty hit list.
fn assert_fixture_hits(value: &Value) {
    let hits = value["hits"]
        .as_array()
        .expect("search success must carry a hits array");
    assert!(
        !hits.is_empty(),
        "recovered search must hit the fixture: {value}"
    );
}

// --- drill: corrupt state -> operational/2 both modes -> reindex heals ---

#[test]
fn corrupt_index_db_search_drill_reindex_heals() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let db = index_db_path(dir.path());
    assert!(db.is_file(), "index must create {}", db.display());

    corrupt_index_db(&db);

    let human = run(&sargs(&["search", "greet", &root]));
    assert_human_error(&human, 2);
    let machine = run(&sargs(&["search", "--json", "greet", &root]));
    assert_failure_envelope(&machine, "search", 2, "operational");

    let healed = run(&sargs(&["--json", "reindex", &root]));
    let value = assert_success_envelope(&healed, "reindex");
    assert!(
        value["files_indexed"].as_u64().unwrap_or(0) >= 1,
        "reindex must rebuild rows: {value}"
    );
    assert!(db.is_file(), "healed state must exist at {}", db.display());

    let recovered = run(&sargs(&["search", "--json", "greet", &root]));
    let value = assert_success_envelope(&recovered, "search");
    assert_fixture_hits(&value);
    assert_human_success(&run(&sargs(&["search", "greet", &root])));
}

// --- drill: deleted state -> operational/2 both modes -> fresh index heals ---

#[test]
fn deleted_index_state_search_drill_index_heals() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let db = index_db_path(dir.path());
    assert!(db.is_file(), "index must create {}", db.display());

    std::fs::remove_dir_all(dir.path().join(".asgrep")).expect("delete state");
    assert!(!db.exists(), "fault must remove {}", db.display());

    let human = run(&sargs(&["search", "greet", &root]));
    assert_human_error(&human, 2);
    let machine = run(&sargs(&["search", "--json", "greet", &root]));
    assert_failure_envelope(&machine, "search", 2, "operational");

    let rebuilt = run(&sargs(&["--json", "index", &root]));
    let value = assert_success_envelope(&rebuilt, "index");
    assert!(
        value["files_indexed"].as_u64().unwrap_or(0) >= 1,
        "fresh index must rebuild rows: {value}"
    );
    assert!(db.is_file(), "rebuilt state must exist at {}", db.display());

    let recovered = run(&sargs(&["search", "--json", "greet", &root]));
    let value = assert_success_envelope(&recovered, "search");
    assert_fixture_hits(&value);
    assert_human_success(&run(&sargs(&["search", "greet", &root])));
}

// --- drill: unknown --lang -> usage/1 both modes -> valid args recover ---

#[test]
fn bad_lang_arg_search_drill_fix_arg_recovers() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();

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

    let recovered = run(&sargs(&["search", "--json", "greet", &root]));
    let value = assert_success_envelope(&recovered, "search");
    assert_fixture_hits(&value);
    assert_human_success(&run(&sargs(&["search", "greet", &root])));
}

// --- drill: QUERY+--pattern conflict -> usage/1 both modes -> one query recovers ---

#[test]
fn query_pattern_conflict_drill_single_query_recovers() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();

    let human = run(&sargs(&["search", "--pattern", "greet", "greet", &root]));
    assert_human_error(&human, 1);
    let machine = run(&sargs(&[
        "search", "--json", "--pattern", "greet", "greet", &root,
    ]));
    assert_failure_envelope(&machine, "search", 1, "usage");

    let recovered = run(&sargs(&["search", "--json", "greet", &root]));
    let value = assert_success_envelope(&recovered, "search");
    assert_fixture_hits(&value);
    assert_human_success(&run(&sargs(&["search", "greet", &root])));
}

// --- drill: invalid regex -> operational/2 both modes -> valid query recovers ---

#[test]
fn invalid_regex_query_drill_fix_query_recovers() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let query = "regex:(unclosed";

    let human = run(&sargs(&["search", query, &root]));
    assert_human_error(&human, 2);
    let machine = run(&sargs(&["search", "--json", query, &root]));
    assert_failure_envelope(&machine, "search", 2, "operational");

    let recovered = run(&sargs(&["search", "--json", "greet", &root]));
    let value = assert_success_envelope(&recovered, "search");
    assert_fixture_hits(&value);
    assert_human_success(&run(&sargs(&["search", "greet", &root])));
}

// --- drill: directory-as-gold eval -> operational/2 both modes -> valid gold recovers ---

#[test]
fn unreadable_gold_eval_drill_valid_gold_recovers() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();

    let gold_path = dir.path().join("gold.json");
    std::fs::create_dir(&gold_path).expect("gold path starts as a directory");
    let gold_arg = gold_path.to_string_lossy().into_owned();

    let human = run(&sargs(&["eval", "--gold", &gold_arg, &root]));
    assert_human_error(&human, 2);
    let machine = run(&sargs(&["eval", "--json", "--gold", &gold_arg, &root]));
    assert_failure_envelope(&machine, "eval", 2, "operational");

    std::fs::remove_dir(&gold_path).expect("remove gold directory");
    std::fs::write(
        &gold_path,
        r#"{"corpus":"e4","queries":[{"name":"g1","query":"greet","k":5,"relevant":[{"file":"a.rs","symbol":"greet"}]}]}"#,
    )
    .expect("write valid gold");

    let recovered = run(&sargs(&["eval", "--json", "--gold", &gold_arg, &root]));
    let value = assert_success_envelope(&recovered, "eval");
    assert_eq!(
        value["queries"].as_array().map(Vec::len),
        Some(1),
        "recovered eval must score one query: {value}"
    );
    assert_eq!(value["aggregate"]["n_queries"], 1);
    assert_human_success(&run(&sargs(&["eval", "--gold", &gold_arg, &root])));
}

// --- drill: outline of a missing file -> operational/2 both modes -> indexed file recovers ---

#[test]
fn outline_missing_file_drill_indexed_file_recovers() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();

    let human = run(&sargs(&["outline", "no-such-file.rs", &root]));
    assert_human_error(&human, 2);
    let machine = run(&sargs(&["outline", "--json", "no-such-file.rs", &root]));
    assert_failure_envelope(&machine, "outline", 2, "operational");

    let recovered = run(&sargs(&["outline", "--json", "a.rs", &root]));
    let value = assert_success_envelope(&recovered, "outline");
    let symbols = value["symbols"]
        .as_array()
        .expect("outline success must carry a symbols array");
    assert!(!symbols.is_empty(), "indexed file must outline: {value}");
    assert_eq!(
        value["count"].as_u64(),
        Some(symbols.len() as u64),
        "outline count must match symbols length: {value}"
    );
    assert_human_success(&run(&sargs(&["outline", "a.rs", &root])));
}

// --- chained double fault: corrupt state + bad arg -> usage, then operational, then success ---

#[test]
fn chained_corrupt_plus_bad_arg_ordered_recovery() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let db = index_db_path(dir.path());
    assert!(db.is_file(), "index must create {}", db.display());

    corrupt_index_db(&db);

    // Both faults present: arg validation runs before state open, so usage/1 wins.
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

    // Fix the arg: the corrupt-state fault surfaces as operational/2.
    let human = run(&sargs(&["search", "greet", &root]));
    assert_human_error(&human, 2);
    let machine = run(&sargs(&["search", "--json", "greet", &root]));
    assert_failure_envelope(&machine, "search", 2, "operational");

    // Heal the state: reindex, then clean search in both modes.
    let healed = run(&sargs(&["--json", "reindex", &root]));
    assert_success_envelope(&healed, "reindex");
    assert!(db.is_file(), "healed state must exist at {}", db.display());

    let recovered = run(&sargs(&["search", "--json", "greet", &root]));
    let value = assert_success_envelope(&recovered, "search");
    assert_fixture_hits(&value);
    assert_human_success(&run(&sargs(&["search", "greet", &root])));
}

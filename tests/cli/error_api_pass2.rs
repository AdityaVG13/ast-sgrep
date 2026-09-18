//! E2 error-propagation oracles: library errors reach the CLI surface faithfully.
//!
//! Complements `error_api_pass1.rs` (E1 taxonomy inventory) without duplicating
//! its cells. E1 pins the CLI's own exit contract (`0=ok 1=usage 2=fail`) and
//! envelope shapes for CLI-level rejections. E2 proves *propagation*: a
//! hand-built trigger that makes a library function fail must make the real
//! `asgrep` binary fail the same way — same documented exit code in human and
//! `--json` modes, machine envelopes carrying the error discriminant
//! (`error.kind`), and no failure path printing a success envelope (`ok:true`,
//! `exit_code:0`) or exiting 0.
//!
//! Assertions cover exit codes and envelope shapes/counts only — never message
//! text. Each test drives a lib function directly (proving the library
//! rejects) plus the real binary in human and machine modes (proving the CLI
//! propagates). Fixtures are `tempfile` directories; no new dependencies.

use ast_sgrep_core::{
    call_path::{find_call_path, CallPathConfig},
    codemod::plan_codemod,
    Durability, IndexStore, SearchOptions, Searcher, MAX_FILE_FILTER_CHARS, MAX_QUERY_CHARS,
};
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
/// shape on stdout (neither a parsed `ok:true` envelope nor compact markers).
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

// --- StoreError::Other(query len) -> search exit 2 ---

#[test]
fn query_too_long_search_propagates_operational() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let long = "x".repeat(MAX_QUERY_CHARS + 500);

    assert!(
        ast_sgrep_core::validate_query_len(&long).is_err(),
        "lib must reject an oversize query"
    );
    assert!(
        lib_searcher(dir.path()).search(&long).is_err(),
        "lib Searcher::search must reject an oversize query"
    );

    let json_args = vec![
        OsString::from("search"),
        OsString::from("--json"),
        OsString::from(&long),
        OsString::from(&root),
    ];
    assert_failure_envelope(&run(&json_args), "search", 2, "operational");

    let human_args = vec![
        OsString::from("search"),
        OsString::from(&long),
        OsString::from(&root),
    ];
    assert_human_error(&run(&human_args), 2);
}

// --- StoreError::Other(invalid regex) -> search exit 2 ---

#[test]
fn invalid_regex_search_propagates_operational() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let query = "regex:(unclosed";

    assert!(
        lib_searcher(dir.path()).search(query).is_err(),
        "lib must reject an invalid regex"
    );

    let machine = run(&sargs(&["search", "--json", query, &root]));
    assert_failure_envelope(&machine, "search", 2, "operational");

    let human = run(&sargs(&["search", query, &root]));
    assert_human_error(&human, 2);
}

// --- StoreError::Other(file_filter len) at Searcher::new -> search exit 2 ---

#[test]
fn file_filter_too_long_propagates_operational() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let long_filter = "a".repeat(MAX_FILE_FILTER_CHARS + 100);

    let opts = SearchOptions {
        root: dir.path().to_path_buf(),
        file_filter: Some(long_filter.clone()),
        ..SearchOptions::default()
    };
    assert!(
        Searcher::new(opts).is_err(),
        "lib Searcher::new must reject an oversize file_filter"
    );

    let json_args = vec![
        OsString::from("search"),
        OsString::from("--json"),
        OsString::from("--file-filter"),
        OsString::from(&long_filter),
        OsString::from("greet"),
        OsString::from(&root),
    ];
    assert_failure_envelope(&run(&json_args), "search", 2, "operational");

    let human_args = vec![
        OsString::from("search"),
        OsString::from("--file-filter"),
        OsString::from(&long_filter),
        OsString::from("greet"),
        OsString::from(&root),
    ];
    assert_human_error(&run(&human_args), 2);
}

// --- StoreError::Other(file_filter control chars) at finish -> search exit 2 ---

#[test]
fn file_filter_control_chars_propagate_operational() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let bad_filter = "a\x01b";

    let opts = SearchOptions {
        root: dir.path().to_path_buf(),
        file_filter: Some(bad_filter.to_string()),
        ..SearchOptions::default()
    };
    let searcher = Searcher::new(opts).expect("lib accepts filter length at open");
    assert!(
        searcher.search("greet").is_err(),
        "lib search must reject a control-char file_filter at finish"
    );

    let machine = run(&sargs(&[
        "search",
        "--json",
        "--file-filter",
        bad_filter,
        "greet",
        &root,
    ]));
    assert_failure_envelope(&machine, "search", 2, "operational");

    let human = run(&sargs(&["search", "--file-filter", bad_filter, "greet", &root]));
    assert_human_error(&human, 2);
}

// --- StoreError::Other(in: scope missing) -> search exit 2 ---

#[test]
fn in_scope_missing_propagates_operational() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let query = "greet in:no-such-dir-xyz";

    assert!(
        lib_searcher(dir.path()).search(query).is_err(),
        "lib must reject an in: scope matching nothing"
    );

    let machine = run(&sargs(&["search", "--json", query, &root]));
    assert_failure_envelope(&machine, "search", 2, "operational");

    let human = run(&sargs(&["search", query, &root]));
    assert_human_error(&human, 2);
}

// --- StoreError::Other(in: scope error) -> search exit 2 ---

#[test]
fn in_scope_escape_propagates_operational() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let query = "greet in:../escape";

    assert!(
        lib_searcher(dir.path()).search(query).is_err(),
        "lib must reject an escaping in: scope"
    );

    let machine = run(&sargs(&["search", "--json", query, &root]));
    assert_failure_envelope(&machine, "search", 2, "operational");

    let human = run(&sargs(&["search", query, &root]));
    assert_human_error(&human, 2);
}

// --- Feature-gated lib errors -> search exit 2 (agreement in both builds) ---
//
// When the binary lacks `neural-embed`/`rerank`, the library rejects and the
// CLI must propagate operational/2. When built with the features, both agree
// on success instead — either way lib and CLI stay in agreement.

#[test]
fn feature_gates_propagate_between_lib_and_cli() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();

    let neural_opts = SearchOptions {
        root: dir.path().to_path_buf(),
        use_embed: true,
        use_neural_embed: true,
        ..SearchOptions::default()
    };
    let neural_lib_err = Searcher::new(neural_opts).is_err();
    let neural_machine = run(&sargs(&[
        "search",
        "--json",
        "--neural-embed",
        "greet",
        &root,
    ]));
    let neural_human = run(&sargs(&["search", "--neural-embed", "greet", &root]));
    if neural_lib_err {
        assert_failure_envelope(&neural_machine, "search", 2, "operational");
        assert_human_error(&neural_human, 2);
    } else {
        assert_eq!(neural_machine.status.code(), Some(0));
        assert_eq!(neural_human.status.code(), Some(0));
    }

    let rerank_opts = SearchOptions {
        root: dir.path().to_path_buf(),
        use_rerank: true,
        ..SearchOptions::default()
    };
    let rerank_lib_err = Searcher::new(rerank_opts).is_err();
    let rerank_machine = run(&sargs(&["search", "--json", "--rerank", "greet", &root]));
    let rerank_human = run(&sargs(&["search", "--rerank", "greet", &root]));
    if rerank_lib_err {
        assert_failure_envelope(&rerank_machine, "search", 2, "operational");
        assert_human_error(&rerank_human, 2);
    } else {
        assert_eq!(rerank_machine.status.code(), Some(0));
        assert_eq!(rerank_human.status.code(), Some(0));
    }
}

// --- StoreError::Database (non-DB index path) -> search exit 2 ---

#[test]
fn corrupt_index_path_propagates_operational() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let bad = dir.path().join("bad-index.db");
    std::fs::write(&bad, "not a sqlite db").expect("write bad index");
    let bad_arg = bad.to_string_lossy().into_owned();

    assert!(
        IndexStore::open_readonly(dir.path(), Some(&bad)).is_err(),
        "lib must reject a non-database index path"
    );

    let machine = run(&sargs(&[
        "search",
        "--json",
        "--index-path",
        &bad_arg,
        "greet",
        &root,
    ]));
    assert_failure_envelope(&machine, "search", 2, "operational");

    let human = run(&sargs(&["search", "--index-path", &bad_arg, "greet", &root]));
    assert_human_error(&human, 2);
}

// --- plan_codemod(empty pattern) -> codemod exit 2 (preview + apply agree) ---

#[test]
fn codemod_empty_pattern_propagates_operational() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();

    assert!(
        plan_codemod(dir.path(), None, None, "", "x").is_err(),
        "lib plan_codemod must reject an empty pattern"
    );

    let dry = run(&sargs(&[
        "codemod",
        "--dry-run",
        "--pattern",
        "",
        "--rewrite",
        "x",
        &root,
    ]));
    assert_failure_envelope(&dry, "codemod", 2, "operational");

    let human = run(&sargs(&[
        "codemod", "--yes", "--pattern", "", "--rewrite", "x", &root,
    ]));
    assert_human_error(&human, 2);

    let apply_json = run(&sargs(&[
        "codemod",
        "--json",
        "--yes",
        "--pattern",
        "",
        "--rewrite",
        "x",
        &root,
    ]));
    assert_failure_envelope(&apply_json, "codemod", 2, "operational");
}

// --- StoreError::Other(query len) via call-path -> exit 2 ---

#[test]
fn call_path_oversize_propagates_operational() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let long = "y".repeat(MAX_QUERY_CHARS + 500);

    let store = IndexStore::open_readonly(dir.path(), None).expect("lib opens store");
    assert!(
        find_call_path(&store, &long, "greet", &CallPathConfig::default()).is_err(),
        "lib find_call_path must reject an oversize source"
    );

    let json_args = vec![
        OsString::from("call-path"),
        OsString::from("--json"),
        OsString::from(&long),
        OsString::from("greet"),
        OsString::from(&root),
    ];
    assert_failure_envelope(&run(&json_args), "call-path", 2, "operational");

    let human_args = vec![
        OsString::from("call-path"),
        OsString::from(&long),
        OsString::from("greet"),
        OsString::from(&root),
    ];
    assert_human_error(&run(&human_args), 2);
}

// --- bench_suite unknown suite (lib None) -> bench exit 2 ---

#[test]
fn bench_unknown_suite_propagates_operational() {
    let dir = fixture_root();
    let root = dir.path().to_string_lossy().into_owned();

    assert!(
        ast_sgrep_core::bench_suite::fixture_by_name("sample").is_some(),
        "lib knows the default bench fixture"
    );
    assert!(
        ast_sgrep_core::bench_suite::suite_by_name("no-such-suite-xyz").is_none(),
        "lib must reject an unknown bench suite"
    );

    let machine = run(&sargs(&[
        "bench",
        "--json",
        "--suite",
        "no-such-suite-xyz",
        &root,
    ]));
    assert_failure_envelope(&machine, "bench", 2, "operational");

    let human = run(&sargs(&["bench", "--suite", "no-such-suite-xyz", &root]));
    assert_human_error(&human, 2);
}

// --- Durability::parse failure (lib None) -> usage exit 1 ---

#[test]
fn durability_unknown_propagates_usage() {
    let dir = fixture_root();
    index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();

    assert!(
        Durability::parse("balanced").is_some(),
        "lib accepts a known durability"
    );
    assert!(
        Durability::parse("bogus-no-such-durability").is_none(),
        "lib must reject an unknown durability"
    );

    let machine = run(&sargs(&[
        "--json",
        "--durability",
        "bogus-no-such-durability",
        "search",
        "greet",
        &root,
    ]));
    assert_failure_envelope(&machine, "search", 1, "usage");

    let human = run(&sargs(&[
        "--durability",
        "bogus-no-such-durability",
        "search",
        "greet",
        &root,
    ]));
    assert_human_error(&human, 1);
}

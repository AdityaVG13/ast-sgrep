//! N2 numerical totality, pass 2: degenerate numeric inputs never panic,
//! never emit success-with-garbage, and always land on the documented
//! outcome (remap-to-default / clamp / usage(1) / operational(2)).
//!
//! Contract under test (all expectations probed against the real binary):
//!
//! - `--limit 0` (flag or `ASGREP_LIMIT=0`) remaps to the default 16
//!   (`clamp_output_limit`); the envelope reports `"limit": 16`.
//! - Negative (`--limit=-1`), non-numeric, and over-`u64` values are clap
//!   parse failures -> exit 1 `usage` envelope on stdout, stderr empty.
//!   (`--limit -1` with a space never reaches the parser: clap treats it
//!   as an unexpected argument. That path is already pinned by
//!   `machine_contracts::bounded_arguments_are_json_usage_errors`; here we
//!   pin the equals form, which DOES reach `parse_bounded_usize`.)
//! - Zero-size windows (`--excerpt-lines 0`, `--snippet-tokens 0`,
//!   `--budget-tokens 0`, `--ann-probes 0`, `--rerank-top-k 0`,
//!   `--ann-threshold 0`) are accepted: exit 0, `ok:true`.
//!   (`--rerank-top-k 0` is clamped to >= 1 downstream; `--ann-probes 0`
//!   is documented as adaptive.)
//! - Unbounded `usize` flags (`--ann-threshold`, `--rerank-top-k`) reject
//!   `u64`-overflowing input with exit 1 `usage`.
//! - `call-path` ranges (`--max-depth 1..=64`, `--max-nodes 1..=100000`,
//!   `--max-edges 1..=500000`) reject 0 and over-max with exit 1 `usage`.
//! - `eval` with `k: 0` or `"relevant": []` succeeds with EXACT zeros
//!   (0/0 guarded: `recall_of` returns 0 on empty relevant, `idcg(0)==0`
//!   forces nDCG 0); every float must be a JSON number, never null
//!   (serde_json renders NaN as null, so null == NaN leak).
//! - `eval` with zero queries, or a non-`usize` k (e.g. `1.5`), or an
//!   empty corpus directory fails closed: exit 2 `operational`.
//! - `eval` with `k == usize::MAX` succeeds: the searcher clamps the
//!   derived limit and `take(cutoff)` bounds the scan; metrics stay in
//!   `[0,1]`.
//!
//! Deliberately NOT covered here (already pinned): `--limit 1001` /
//! `--limit -1` (space form) / `--excerpt-lines 101` envelopes
//! (`machine_contracts`), budget-cap messages + max acceptance and eval
//! metric arithmetic (`numerical_pass1`).

use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

fn run(bin: &PathBuf, args: &[&str]) -> Output {
    Command::new(bin)
        .args(args)
        .output()
        .expect("run asgrep")
}

fn parse_stdout(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout is JSON")
}

fn write_fixture(root: &std::path::Path, name: &str, body: &str) {
    std::fs::write(root.join(name), body).expect("write fixture file");
}

/// Build a one-file indexed project; returns (tempdir, root).
fn indexed_project(term: &str) -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, "a.rs", &format!("fn {term}() {{}}\n"));
    let bin = asgrep_bin();
    let output = run(&bin, &["--json", "--no-embed", "index", root.to_str().unwrap()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "index must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (temp, root)
}

/// Assert an exit-1 usage envelope: shape only, never message text.
fn assert_usage_envelope(output: &Output) -> Value {
    assert_eq!(
        output.status.code(),
        Some(1),
        "degenerate input must be a usage error: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "--json usage errors stay on stdout: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_stdout(output);
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["exit_code"], 1, "{value}");
    assert_eq!(value["error"]["kind"], "usage", "{value}");
    assert!(value["error"]["message"].is_string(), "{value}");
    value
}

/// Assert an exit-2 operational envelope: shape only, never message text.
fn assert_operational_envelope(output: &Output) -> Value {
    assert_eq!(
        output.status.code(),
        Some(2),
        "degenerate input must fail closed as operational: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "--json operational errors stay on stdout: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_stdout(output);
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["exit_code"], 2, "{value}");
    assert_eq!(value["error"]["kind"], "operational", "{value}");
    assert!(value["error"]["message"].is_string(), "{value}");
    value
}

fn assert_finite_unit(value: &Value, pointer: &str) -> f64 {
    let v = value
        .pointer(pointer)
        .unwrap_or_else(|| panic!("missing {pointer} in {value}"))
        .as_f64()
        .unwrap_or_else(|| panic!("{pointer} is not a JSON number in {value}"));
    assert!(
        v.is_finite() && (0.0..=1.0).contains(&v),
        "{pointer} must be a finite unit value, got {v}"
    );
    v
}

// --- --limit 0 remaps to the default ----------------------------------------

#[test]
fn limit_zero_remaps_to_default() {
    let (_temp, root) = indexed_project("wobblebuild_lim0");
    let bin = asgrep_bin();
    let root_arg = root.to_str().unwrap();
    // Flag form.
    let output = run(&bin, &["--json", "--no-embed", "--limit", "0", "qqqxq_missing_zzz", root_arg]);
    assert_eq!(output.status.code(), Some(0));
    let value = parse_stdout(&output);
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["limit"], 16, "limit 0 must remap to default 16: {value}");
    // Env form.
    let output = Command::new(&bin)
        .args(["--json", "--no-embed", "qqqxq_missing_zzz", root_arg])
        .env("ASGREP_LIMIT", "0")
        .output()
        .expect("run asgrep");
    assert_eq!(output.status.code(), Some(0));
    let value = parse_stdout(&output);
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["limit"], 16, "ASGREP_LIMIT=0 must remap to default 16: {value}");
}

// --- negative / non-numeric / overflowing --limit is usage ------------------

#[test]
fn limit_negative_equals_form_is_usage() {
    // Space form (`--limit -1`) never reaches the parser (clap unexpected-
    // argument path, pinned in machine_contracts). The equals form reaches
    // parse_bounded_usize, which must reject the negative.
    let (_temp, root) = indexed_project("wobblebuild_limneg");
    let bin = asgrep_bin();
    let output = run(&bin, &["--json", "--no-embed", "--limit=-1", "q", root.to_str().unwrap()]);
    assert_usage_envelope(&output);
}

#[test]
fn limit_nonnumeric_and_overflow_are_usage() {
    let (_temp, root) = indexed_project("wobblebuild_limof");
    let bin = asgrep_bin();
    let root_arg = root.to_str().unwrap().to_owned();
    // "abc" is not an integer; 2^64 overflows u64/usize parsing.
    for raw in ["abc", "18446744073709551616"] {
        let output = run(&bin, &["--json", "--no-embed", "--limit", raw, "q", &root_arg]);
        assert_usage_envelope(&output);
    }
}

#[test]
fn env_limit_garbage_is_usage() {
    // ASGREP_LIMIT feeds the same clap value parser as --limit: garbage is
    // a usage error, never a silent fallback.
    let (_temp, root) = indexed_project("wobblebuild_limenv");
    let bin = asgrep_bin();
    let output = Command::new(&bin)
        .args(["--json", "--no-embed", "qqqxq_missing_zzz", root.to_str().unwrap()])
        .env("ASGREP_LIMIT", "abc")
        .output()
        .expect("run asgrep");
    assert_usage_envelope(&output);
}

// --- zero-size windows are accepted ------------------------------------------

#[test]
fn zero_size_windows_are_accepted() {
    let (_temp, root) = indexed_project("wobblebuild_zero");
    let bin = asgrep_bin();
    let root_arg = root.to_str().unwrap().to_owned();
    for flag in ["--excerpt-lines", "--snippet-tokens", "--budget-tokens"] {
        let output = run(&bin, &["--json", "--no-embed", flag, "0", "qqqxq_missing_zzz", &root_arg]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{flag} 0 must be accepted: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value = parse_stdout(&output);
        assert_eq!(value["ok"], true, "{flag} 0: {value}");
    }
    // All zero at once, against a query that HITS: still ok:true with a
    // well-shaped hits array (never success-with-garbage).
    let output = run(
        &bin,
        &[
            "--json",
            "--no-embed",
            "--excerpt-lines",
            "0",
            "--snippet-tokens",
            "0",
            "--budget-tokens",
            "0",
            "wobblebuild_zero",
            &root_arg,
        ],
    );
    assert_eq!(output.status.code(), Some(0));
    let value = parse_stdout(&output);
    assert_eq!(value["ok"], true, "{value}");
    assert!(value["hits"].is_array(), "{value}");
    assert!(!value["hits"].as_array().unwrap().is_empty(), "{value}");
}

// --- unbounded usize flags: zero accepted, overflow rejected ----------------

#[test]
fn unclamped_usize_flags_accept_zero_reject_overflow() {
    let (_temp, root) = indexed_project("wobblebuild_unclamped");
    let bin = asgrep_bin();
    let root_arg = root.to_str().unwrap().to_owned();
    // Zero is accepted (--ann-probes 0 is documented adaptive;
    // --rerank-top-k 0 is clamped downstream; --ann-threshold 0 disables).
    for flag in ["--ann-probes", "--rerank-top-k", "--ann-threshold"] {
        let output = run(&bin, &["--json", "--no-embed", flag, "0", "qqqxq_missing_zzz", &root_arg]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{flag} 0 must be accepted: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(parse_stdout(&output)["ok"], true);
    }
    // Values overflowing usize are usage errors, never wraps or panics.
    for flag in ["--ann-threshold", "--rerank-top-k", "--ann-probes"] {
        let output = run(
            &bin,
            &["--json", "--no-embed", flag, "99999999999999999999", "qqqxq_missing_zzz", &root_arg],
        );
        assert_usage_envelope(&output);
    }
}

// --- call-path ranges reject 0 and over-max ---------------------------------

#[test]
fn call_path_bounds_reject_zero_and_over_max() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, "a.rs", "fn wobblebuild_cp() {}\n");
    let bin = asgrep_bin();
    let root_arg = root.to_str().unwrap().to_owned();
    // Parse rejection happens before any index is opened, so no indexing.
    for (flag, raw) in [
        ("--max-depth", "0"),
        ("--max-depth", "65"),
        ("--max-nodes", "0"),
        ("--max-nodes", "100001"),
        ("--max-edges", "0"),
        ("--max-edges", "500001"),
    ] {
        let output = run(&bin, &["--json", "--no-embed", "call-path", "foo", "bar", &root_arg, flag, raw]);
        let value = assert_usage_envelope(&output);
        assert_eq!(value["command"], "call-path", "{value}");
    }
}

// --- eval totality ------------------------------------------------------------

fn write_gold(dir: &std::path::Path, name: &str, body: &Value) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body.to_string()).expect("write gold");
    path
}

fn eval_project(term: &str) -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    write_fixture(&root, "a.rs", &format!("fn {term}() {{}}\n"));
    (temp, root)
}

#[test]
fn eval_k_zero_yields_exact_zeros() {
    // k=0: scan takes no hits (found=0, first_rank=null) and idcg(min(1,0))
    // is 0, forcing nDCG 0. Every float must be a JSON number: a NaN leak
    // would render as null and fail as_f64.
    let (temp, root) = eval_project("wobblebuild_k0");
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "n2", "queries": [
            {"name": "k0", "query": "wobblebuild_k0", "k": 0,
             "relevant": [{"file": "a.rs"}]}
        ]}),
    );
    let bin = asgrep_bin();
    let output = run(
        &bin,
        &["--json", "--no-embed", "eval", "--gold", gold.to_str().unwrap(), root.to_str().unwrap()],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "eval k=0 must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_stdout(&output);
    let q = &value["queries"][0];
    assert!(q["first_rank"].is_null(), "{value}");
    assert_eq!(q["found"], 0, "{value}");
    assert_eq!(q["relevant"], 1, "{value}");
    assert_eq!(assert_finite_unit(&value, "/queries/0/rr"), 0.0);
    assert_eq!(assert_finite_unit(&value, "/queries/0/ndcg"), 0.0);
    assert_eq!(assert_finite_unit(&value, "/queries/0/recall_at/1"), 0.0);
    assert_eq!(assert_finite_unit(&value, "/queries/0/recall_at/5"), 0.0);
    assert_eq!(assert_finite_unit(&value, "/queries/0/recall_at/20"), 0.0);
    assert_eq!(assert_finite_unit(&value, "/aggregate/mrr"), 0.0);
    assert_eq!(assert_finite_unit(&value, "/aggregate/ndcg"), 0.0);
    assert_eq!(assert_finite_unit(&value, "/aggregate/recall_at_k"), 0.0);
    assert_eq!(value["aggregate"]["n_queries"], 1, "{value}");
}

#[test]
fn eval_empty_relevant_yields_exact_zeros() {
    // relevant=[]: recall_of guards 0/0 -> 0, idcg(0)==0 forces nDCG 0.
    let (temp, root) = eval_project("wobblebuild_norel");
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "n2", "queries": [
            {"name": "norel", "query": "wobblebuild_norel", "k": 5, "relevant": []}
        ]}),
    );
    let bin = asgrep_bin();
    let output = run(
        &bin,
        &["--json", "--no-embed", "eval", "--gold", gold.to_str().unwrap(), root.to_str().unwrap()],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "eval relevant=[] must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_stdout(&output);
    let q = &value["queries"][0];
    assert!(q["first_rank"].is_null(), "{value}");
    assert_eq!(q["found"], 0, "{value}");
    assert_eq!(q["relevant"], 0, "{value}");
    assert_eq!(assert_finite_unit(&value, "/queries/0/rr"), 0.0);
    assert_eq!(assert_finite_unit(&value, "/queries/0/ndcg"), 0.0);
    assert_eq!(assert_finite_unit(&value, "/queries/0/recall_at/5"), 0.0);
    assert_eq!(assert_finite_unit(&value, "/aggregate/mrr"), 0.0);
    assert_eq!(assert_finite_unit(&value, "/aggregate/ndcg"), 0.0);
    assert_eq!(assert_finite_unit(&value, "/aggregate/recall_at_k"), 0.0);
}

#[test]
fn eval_degenerate_gold_fails_closed() {
    // Zero queries, or a non-usize k (1.5), is an operational failure (exit
    // 2), never ok:true with fabricated zeros.
    let (temp, root) = eval_project("wobblebuild_badgold");
    let bin = asgrep_bin();
    let root_arg = root.to_str().unwrap().to_owned();
    let empty = write_gold(temp.path(), "empty.json", &serde_json::json!({"corpus": "n2", "queries": []}));
    let output = run(
        &bin,
        &["--json", "--no-embed", "eval", "--gold", empty.to_str().unwrap(), &root_arg],
    );
    let value = assert_operational_envelope(&output);
    assert_eq!(value["command"], "eval", "{value}");
    let float_k = write_gold(
        temp.path(),
        "floatk.json",
        &serde_json::json!({"corpus": "n2", "queries": [
            {"name": "f", "query": "x", "k": 1.5, "relevant": []}
        ]}),
    );
    let output = run(
        &bin,
        &["--json", "--no-embed", "eval", "--gold", float_k.to_str().unwrap(), &root_arg],
    );
    let value = assert_operational_envelope(&output);
    assert_eq!(value["command"], "eval", "{value}");
}

#[test]
fn eval_empty_corpus_fails_closed() {
    // Indexing an empty directory then reporting all-zero MRR/recall would
    // fabricate a quality measurement: fail closed (exit 2).
    let temp = TempDir::new().unwrap();
    let empty = temp.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "n2", "queries": [
            {"name": "solo", "query": "wobblebuild_probe", "k": 5,
             "relevant": [{"file": "a.rs"}]}
        ]}),
    );
    let bin = asgrep_bin();
    let output = run(
        &bin,
        &["--json", "--no-embed", "eval", "--gold", gold.to_str().unwrap(), empty.to_str().unwrap()],
    );
    let value = assert_operational_envelope(&output);
    assert_eq!(value["command"], "eval", "{value}");
}

#[test]
fn eval_usize_max_k_stays_finite() {
    // k == usize::MAX: the derived searcher limit is clamped downstream
    // and the scan is bounded by take(cutoff); metrics stay finite in
    // [0,1] and the rank-1 hit is still found.
    let (temp, root) = eval_project("wobblebuild_bigk");
    let gold = write_gold(
        temp.path(),
        "gold.json",
        &serde_json::json!({"corpus": "n2", "queries": [
            {"name": "big", "query": "wobblebuild_bigk", "k": 18446744073709551615u64,
             "relevant": [{"file": "a.rs"}]}
        ]}),
    );
    let bin = asgrep_bin();
    let output = run(
        &bin,
        &["--json", "--no-embed", "eval", "--gold", gold.to_str().unwrap(), root.to_str().unwrap()],
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "eval huge k must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_stdout(&output);
    assert_eq!(value["queries"][0]["first_rank"], 1, "{value}");
    assert_eq!(assert_finite_unit(&value, "/queries/0/rr"), 1.0);
    assert_eq!(assert_finite_unit(&value, "/queries/0/ndcg"), 1.0);
    assert_eq!(assert_finite_unit(&value, "/queries/0/recall_at/20"), 1.0);
    assert_eq!(assert_finite_unit(&value, "/aggregate/mrr"), 1.0);
    assert_eq!(assert_finite_unit(&value, "/aggregate/ndcg"), 1.0);
    assert_eq!(assert_finite_unit(&value, "/aggregate/recall_at_k"), 1.0);
}

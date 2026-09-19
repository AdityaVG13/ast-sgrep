//! CLI numerical: limit/cap/window surface contract suite.
//!
//! Canonical per-surface successor of the flag-parsing arms of
//! `numerical_pass{1,2}.rs` per `tests/catalog/numerical-cli.md`: --limit
//! remap/rejection, token-budget caps, zero-size windows, unclamped usize
//! flags, and call-path ranges. Eval-arithmetic arms live in
//! `numerical_eval.rs`; search-limit arms in `numerical_search.rs`.
//!
//! Absorption map (catalog VERDICTs): 5 KEEP anchors stay standalone;
//! `budget_cap_contract` absorbs 3, `limit_parse_contract` absorbs 2,
//! `zero_window_contract` absorbs 1. 8 tests.

use ast_sgrep_testkit::{
    asgrep_bin, assert_usage_envelope, indexed_project, parse_stdout, run, run_env,
    run_index_json_noembed, usage_message, write_fixture,
};
use tempfile::TempDir;

/// INTENT: Over-cap --budget-tokens usage message names the exact max 65536.
/// KILLS: cap/message-regression (wrong max in code or text).
/// ABSORBS: none (KEEP anchor).
#[test]
fn budget_tokens_cap_names_65536() {
    let value = usage_message(&["--json", "--budget-tokens", "65537", "query", "."]);
    let msg = value["error"]["message"].as_str().expect("message");
    assert!(
        msg.contains("--budget-tokens must not exceed 65536"),
        "message must name the exact cap: {msg}"
    );
}

/// INTENT: --limit 0 and ASGREP_LIMIT=0 remap to default 16 in envelope.
/// KILLS: remap-omission (0 passes through).
/// ABSORBS: none (KEEP anchor).
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
    // Env form (testkit run_env: hermetic scrub, then ASGREP_LIMIT wins).
    let output = run_env(
        &bin,
        &["--json", "--no-embed", "qqqxq_missing_zzz", root_arg],
        &[("ASGREP_LIMIT", "0")],
    );
    assert_eq!(output.status.code(), Some(0));
    let value = parse_stdout(&output);
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(value["limit"], 16, "ASGREP_LIMIT=0 must remap to default 16: {value}");
}

/// INTENT: Non-numeric and 2^64 --limit values are exit-1 usage.
/// KILLS: parse-fail-open (silent fallback or panic).
/// ABSORBS: none (KEEP anchor).
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

/// INTENT: Zero excerpt/snippet/budget windows accepted exit 0, incl.
/// all-zero hit run.
/// KILLS: zero-rejection (0 treated as invalid).
/// ABSORBS: none (KEEP anchor).
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

/// INTENT: call-path max-depth/nodes/edges reject 0 and over-max as usage.
/// KILLS: range-check-omission.
/// ABSORBS: none (KEEP anchor).
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

/// INTENT: One contract for token-budget caps: --snippet-tokens names max
/// 4096, --response-snippet-tokens names max 65536, and cap-boundary 65536
/// parses with search staying exit 0 / ok:true.
/// KILLS: cap/message-regression, boundary-off-by-one (>= vs >).
/// ABSORBS: snippet_tokens_cap_names_4096,
/// response_snippet_tokens_cap_names_65536, budget_tokens_max_is_accepted.
#[test]
fn budget_cap_contract() {
    // Arm 1 (absorbed: snippet_tokens_cap_names_4096).
    {
        let value = usage_message(&["--json", "--snippet-tokens", "4097", "query", "."]);
        let msg = value["error"]["message"].as_str().expect("message");
        assert!(
            msg.contains("--snippet-tokens must not exceed 4096"),
            "message must name the exact cap: {msg}"
        );
    }
    // Arm 2 (absorbed: response_snippet_tokens_cap_names_65536).
    {
        let value = usage_message(&[
            "--json",
            "--response-snippet-tokens",
            "65537",
            "query",
            ".",
        ]);
        let msg = value["error"]["message"].as_str().expect("message");
        assert!(
            msg.contains("--response-snippet-tokens must not exceed 65536"),
            "message must name the exact cap: {msg}"
        );
    }
    // Arm 3 (absorbed: budget_tokens_max_is_accepted). 65536 is the cap
    // itself (parse rejects only value > maximum), so a search carrying it
    // must proceed past argument parsing: index the root, then zero hits on
    // a gibberish query stays exit 0 / ok:true per the exit contract.
    {
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        write_fixture(&root, "a.rs", "fn wobblebuild_cap() {}\n");
        run_index_json_noembed(&root);
        let bin = asgrep_bin();
        let output = run(
            &bin,
            &[
                "--json",
                "--no-embed",
                "--budget-tokens",
                "65536",
                "qqqxq_missing_zzz",
                root.to_str().expect("root utf8"),
            ],
        );
        assert_eq!(
            output.status.code(),
            Some(0),
            "max budget must parse and search must stay exit 0: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value = parse_stdout(&output);
        assert_eq!(value["ok"], true, "{value}");
    }
}

/// INTENT: One contract for --limit parse totality: --limit=-1 equals form
/// is exit-1 usage, and ASGREP_LIMIT=abc is exit-1 usage (never silent
/// fallback).
/// KILLS: parse-accept-garbage (negative accepted), env-parser-bypass (env
/// skips clap parser).
/// ABSORBS: limit_negative_equals_form_is_usage, env_limit_garbage_is_usage.
#[test]
fn limit_parse_contract() {
    // Arm 1 (absorbed: limit_negative_equals_form_is_usage). Space form
    // (`--limit -1`) never reaches the parser (clap unexpected-argument
    // path, pinned in machine_contracts). The equals form reaches
    // parse_bounded_usize, which must reject the negative.
    {
        let (_temp, root) = indexed_project("wobblebuild_limneg");
        let bin = asgrep_bin();
        let output = run(&bin, &["--json", "--no-embed", "--limit=-1", "q", root.to_str().unwrap()]);
        assert_usage_envelope(&output);
    }
    // Arm 2 (absorbed: env_limit_garbage_is_usage). ASGREP_LIMIT feeds the
    // same clap value parser as --limit: garbage is a usage error, never a
    // silent fallback.
    {
        let (_temp, root) = indexed_project("wobblebuild_limenv");
        let bin = asgrep_bin();
        let output = run_env(
            &bin,
            &["--json", "--no-embed", "qqqxq_missing_zzz", root.to_str().unwrap()],
            &[("ASGREP_LIMIT", "abc")],
        );
        assert_usage_envelope(&output);
    }
}

/// INTENT: Unbounded usize flags (--ann-probes, --rerank-top-k,
/// --ann-threshold) accept 0 and reject usize-overflow as usage.
/// KILLS: overflow-wrap-panic.
/// ABSORBS: unclamped_usize_flags_accept_zero_reject_overflow.
#[test]
fn zero_window_contract() {
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

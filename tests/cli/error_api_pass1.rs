//! E1 error-taxonomy inventory: one test per exit-code family, not per cell.
//!
//! Pins the CLI contract `Exit: 0=ok 1=usage 2=fail` (see `Cli::command`
//! `after_help`) plus the machine failure envelope
//! `{schema_version, tool, command, ok:false, exit_code, error:{kind, message}}`
//! (`print_machine_failure`). The 12 taxonomy cells fold into 2 family anchors
//! (usage/1, operational/2) with one leg per cell, plus a success-shape
//! control row. Assertions cover exit codes and envelope shapes/counts only —
//! never message text.

#[path = "error_testkit.rs"]
mod kit;

use serde_json::Value;
use tempfile::TempDir;

/// INTENT: usage family — every CLI-side arg/shape rejection exits 1 with a
/// usage envelope (machine) or a stderr explanation (human).
/// KILLS: exit-code-swap(1↔2), envelope-kind-swap, missing-arg-default-Ok,
/// lang/format-fallback-swallow, conflict-takes-first-silently,
/// guard-drop (implicit --yes), first-wins-silently on ambiguous roots.
/// ABSORBS: missing_query_is_usage_exit_1, unknown_lang_is_usage_exit_1,
/// bad_format_is_usage_exit_1, query_and_pattern_conflict_is_usage_exit_1,
/// index_dry_run_and_path_conflict_is_usage_exit_1,
/// codemod_without_yes_is_usage_exit_1, ambiguous_root_is_usage_exit_1 — one
/// leg each; the clap leg is the original anchor.
#[test]
fn clap_rejection_is_exit_1_with_usage_envelope() {
    // Leg 1 (anchor): clap parse rejection, human and machine shapes.
    let human = kit::run(&["--no-such-flag-xyz"]);
    kit::assert_human_error(&human, 1);
    let machine = kit::run(&["--json", "--no-such-flag-xyz"]);
    kit::assert_failure_envelope(&machine, "search", 1, "usage");

    // Leg 2 (was missing_query_is_usage_exit_1): bare invocation.
    let output = kit::run(&[]);
    kit::assert_human_error(&output, 1);

    // Leg 3 (was unknown_lang_is_usage_exit_1): unknown --lang fails closed.
    {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path().to_string_lossy().into_owned();
        let output = kit::run(&[
            "--json",
            "--lang",
            "xx-no-such-lang",
            "search",
            "greet",
            root.as_str(),
        ]);
        kit::assert_failure_envelope(&output, "search", 1, "usage");
    }

    // Leg 4 (was bad_format_is_usage_exit_1): unknown --format on an indexed root.
    {
        let dir = kit::fixture_root();
        kit::index_root(dir.path());
        let root = dir.path().to_string_lossy().into_owned();
        let output = kit::run(&[
            "search",
            "--json",
            "--format",
            "bogus",
            "greet",
            root.as_str(),
        ]);
        kit::assert_failure_envelope(&output, "search", 1, "usage");
    }

    // Legs 5-7 (were query_and_pattern_conflict / index_dry_run_and_path_conflict
    // / codemod_without_yes): mutual-exclusion and guard rejections.
    {
        let dir = kit::fixture_root();
        let root = dir.path().to_string_lossy().into_owned();
        let conflict = kit::run(&["search", "--pattern", "greet", "greet", root.as_str()]);
        kit::assert_human_error(&conflict, 1);
        let dry_run = kit::run(&["index", "--dry-run", "--path", "a.rs", root.as_str()]);
        kit::assert_human_error(&dry_run, 1);
        let codemod = kit::run(&[
            "codemod",
            "--pattern",
            "greet",
            "--rewrite",
            "greet",
            root.as_str(),
        ]);
        kit::assert_human_error(&codemod, 1);
    }

    // Leg 8 (was ambiguous_root_is_usage_exit_1): --root + positional ROOT.
    {
        let a = kit::fixture_root();
        let b = kit::fixture_root();
        let ra = a.path().to_string_lossy().into_owned();
        let rb = b.path().to_string_lossy().into_owned();
        let output = kit::run(&["--json", "--root", ra.as_str(), "search", "greet", rb.as_str()]);
        kit::assert_failure_envelope(&output, "search", 1, "usage");
    }
}

/// INTENT: operational family — every runtime/state fault exits 2 with an
/// operational envelope (machine) or a stderr explanation (human).
/// KILLS: exit-code-swap(2→1), envelope-kind-swap, auto-index-swallow-to-Ok,
/// gold-default-empty-swallow, empty-outline-Ok-swallow, json-error-to-usage-swap.
/// ABSORBS: unindexed_root_search_is_operational_exit_2,
/// eval_gold_failures_are_operational_exit_2,
/// outline_unindexed_file_is_operational_exit_2,
/// codemode_batch_malformed_is_operational_exit_2 — one leg each; the
/// missing-root leg is the original anchor.
#[test]
fn missing_root_search_is_operational_exit_2() {
    // Leg 1 (anchor): missing project root.
    {
        let dir = TempDir::new().expect("tempdir");
        let missing = dir.path().join("does-not-exist-xyz");
        let missing_arg = missing.to_string_lossy().into_owned();
        let output = kit::run(&["search", "--json", "greet", missing_arg.as_str()]);
        kit::assert_failure_envelope(&output, "search", 2, "operational");
    }

    // Leg 2 (was unindexed_root_search_is_operational_exit_2): no auto-index.
    // Leg 3 (was eval_gold_failures_are_operational_exit_2): unreadable gold
    // (machine) and query-less gold (human).
    {
        let dir = kit::fixture_root();
        let root = dir.path().to_string_lossy().into_owned();
        let output = kit::run(&["search", "--json", "greet", root.as_str()]);
        kit::assert_failure_envelope(&output, "search", 2, "operational");

        let missing = dir.path().join("no-gold.json");
        let missing_arg = missing.to_string_lossy().into_owned();
        let unreadable = kit::run(&["eval", "--json", "--gold", missing_arg.as_str(), root.as_str()]);
        kit::assert_failure_envelope(&unreadable, "eval", 2, "operational");

        let empty = dir.path().join("empty-gold.json");
        std::fs::write(&empty, r#"{"corpus":"e1","queries":[]}"#).expect("write gold");
        let empty_arg = empty.to_string_lossy().into_owned();
        let query_less = kit::run(&["eval", "--gold", empty_arg.as_str(), root.as_str()]);
        kit::assert_human_error(&query_less, 2);
    }

    // Leg 4 (was outline_unindexed_file_is_operational_exit_2).
    {
        let dir = kit::fixture_root();
        kit::index_root(dir.path());
        let root = dir.path().to_string_lossy().into_owned();
        let output = kit::run(&["outline", "--json", "no-such-file.rs", root.as_str()]);
        kit::assert_failure_envelope(&output, "outline", 2, "operational");
    }

    // Leg 5 (was codemode_batch_malformed_is_operational_exit_2): malformed
    // batch payload (always-machine).
    {
        let dir = TempDir::new().expect("tempdir");
        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, "{not valid json").expect("write bad batch");
        let bad_arg = bad.to_string_lossy().into_owned();
        let output = kit::run(&["codemode-batch", "--requests", bad_arg.as_str()]);
        kit::assert_failure_envelope(&output, "codemode-batch", 2, "operational");
    }
}

/// INTENT: success-shape control — a zero-hit search is ok:true with an empty
/// hit list, proving empty results are not errors.
/// KILLS: empty-to-error-swap.
/// ABSORBS: none (stands alone as the control row).
#[test]
fn zero_hit_search_is_success_exit_0() {
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let output = kit::run(&[
        "search",
        "--json",
        "literal:zzzqqqxxyy-no-such-substring",
        root.as_str(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = kit::parse_stdout(&output);
    assert_eq!(value["ok"], true);
    assert_eq!(value["exit_code"], 0);
    assert_eq!(
        value["hits"],
        Value::Array(Vec::new()),
        "zero-hit search must carry an empty hit list: {value}"
    );
}

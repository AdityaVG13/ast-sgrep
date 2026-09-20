//! E3 negative-path metamorphic tests: one test per relation, not per cell.
//!
//! Complements `error_api_pass1.rs` (E1 taxonomy families) and
//! `error_api_pass2.rs` (E2 propagation). E3 asserts *relations* between
//! failure observations, reusing known faults without re-pinning single cells:
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

#[path = "error_testkit.rs"]
mod kit;

use ast_sgrep_core::{codemod::plan_codemod, IndexStore};
use tempfile::TempDir;

/// INTENT: human/machine agreement — the same fault fails in both modes with
/// the same exit, never success in one (operational root fault, operational
/// query fault, usage fault).
/// KILLS: mode-divergence (success-in-one-mode).
/// ABSORBS: invalid_regex_human_and_json_agree_on_operational,
/// unknown_lang_human_and_json_agree_on_usage — one leg each; the missing-root
/// leg is the original anchor.
#[test]
fn missing_root_human_and_json_agree_on_operational() {
    // Leg 1 (anchor): operational root fault.
    {
        let dir = TempDir::new().expect("tempdir");
        let missing = dir.path().join("does-not-exist-xyz");
        let missing_arg = missing.to_string_lossy().into_owned();
        assert!(
            IndexStore::open_readonly(&missing, None).is_err(),
            "lib must reject a missing root"
        );
        let human = kit::run(&["search", "greet", missing_arg.as_str()]);
        kit::assert_human_error(&human, 2);
        let machine = kit::run(&["search", "--json", "greet", missing_arg.as_str()]);
        kit::assert_failure_envelope(&machine, "search", 2, "operational");
        assert_eq!(
            human.status.code(),
            machine.status.code(),
            "same fault must fail in both modes, never succeed in one"
        );
    }

    // Leg 2 (was invalid_regex_human_and_json_agree_on_operational): library
    // query fault.
    {
        let dir = kit::fixture_root();
        kit::index_root(dir.path());
        let root = dir.path().to_string_lossy().into_owned();
        let query = "regex:(unclosed";
        assert!(
            kit::lib_searcher(dir.path()).search(query).is_err(),
            "lib must reject an invalid regex"
        );
        let human = kit::run(&["search", query, root.as_str()]);
        kit::assert_human_error(&human, 2);
        let machine = kit::run(&["search", "--json", query, root.as_str()]);
        kit::assert_failure_envelope(&machine, "search", 2, "operational");
        assert_eq!(
            human.status.code(),
            machine.status.code(),
            "same fault must fail in both modes, never succeed in one"
        );
    }

    // Leg 3 (was unknown_lang_human_and_json_agree_on_usage): usage fault.
    {
        let dir = kit::fixture_root();
        let root = dir.path().to_string_lossy().into_owned();
        assert!(
            ast_sgrep_core::Language::parse("rs").is_some(),
            "lib accepts a known language"
        );
        assert!(
            ast_sgrep_core::Language::parse("xx-no-such-lang").is_none(),
            "lib must reject an unknown language"
        );
        let human = kit::run(&[
            "--lang",
            "xx-no-such-lang",
            "search",
            "greet",
            root.as_str(),
        ]);
        kit::assert_human_error(&human, 1);
        let machine = kit::run(&[
            "--json",
            "--lang",
            "xx-no-such-lang",
            "search",
            "greet",
            root.as_str(),
        ]);
        kit::assert_failure_envelope(&machine, "search", 1, "usage");
        assert_eq!(
            human.status.code(),
            machine.status.code(),
            "same fault must fail in both modes, never succeed in one"
        );
    }
}

/// INTENT: one operational fault yields exit 2 + operational kind on every
/// reader subcommand, for both the missing-root and unindexed-root faults.
/// KILLS: subcommand-exit-divergence, kind-divergence.
/// ABSORBS: unindexed_root_same_operational_family_across_subcommands (second
/// fault leg; parameterized fault × subcommand).
#[test]
fn missing_root_same_operational_family_across_subcommands() {
    let scratch = TempDir::new().expect("tempdir");
    let missing = scratch.path().join("does-not-exist-xyz");
    let missing_arg = missing.to_string_lossy().into_owned();
    assert!(
        IndexStore::open_readonly(&missing, None).is_err(),
        "lib must reject a missing root"
    );

    let unindexed = kit::fixture_root();
    let unindexed_arg = unindexed.path().to_string_lossy().into_owned();
    assert!(
        IndexStore::open_readonly(unindexed.path(), None).is_err(),
        "lib must reject an unindexed root at open"
    );

    let mut codes = Vec::new();
    for (fault, root_arg) in [("missing", &missing_arg), ("unindexed", &unindexed_arg)] {
        for sub in ["search", "outline", "call-path", "chain"] {
            let args: Vec<&str> = match sub {
                "search" => vec![sub, "--json", "greet", root_arg.as_str()],
                "outline" => vec![sub, "--json", "a.rs", root_arg.as_str()],
                "call-path" => vec![sub, "--json", "greet", "greet", root_arg.as_str()],
                _ => vec![sub, "--json", "greet", root_arg.as_str()],
            };
            let output = kit::run(&args);
            assert_eq!(
                output.status.code(),
                Some(2),
                "{sub} on a {fault} root must exit operational/2: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let value = kit::parse_stdout(&output);
            assert_eq!(value["ok"], false, "{sub} on {fault} must not succeed");
            assert_eq!(
                value["error"]["kind"], "operational",
                "{sub} on {fault} kind"
            );
            codes.push(output.status.code());
        }
    }
    assert!(
        codes.iter().all(|code| *code == Some(2)),
        "one fault must yield one exit family across subcommands: {codes:?}"
    );
}

/// INTENT: one ambiguity fault yields exit 1 + usage kind on every affected
/// subcommand.
/// KILLS: subcommand-exit-divergence.
/// ABSORBS: none (usage family; distinct from the operational relation).
#[test]
fn ambiguous_root_same_usage_family_across_subcommands() {
    let a = kit::fixture_root();
    let b = kit::fixture_root();
    let ra = a.path().to_string_lossy().into_owned();
    let rb = b.path().to_string_lossy().into_owned();

    let search = kit::run(&[
        "--json",
        "--root",
        ra.as_str(),
        "search",
        "greet",
        rb.as_str(),
    ]);
    let outline = kit::run(&[
        "--json",
        "--root",
        ra.as_str(),
        "outline",
        "a.rs",
        rb.as_str(),
    ]);
    let call_path = kit::run(&[
        "--json",
        "--root",
        ra.as_str(),
        "call-path",
        "greet",
        "greet",
        rb.as_str(),
    ]);

    for (name, command, output) in [
        ("search", "search", &search),
        ("outline", "outline", &outline),
        ("call-path", "call-path", &call_path),
    ] {
        kit::assert_failure_envelope(output, command, 1, "usage");
        let _ = name;
    }
    assert_eq!(search.status.code(), outline.status.code());
    assert_eq!(search.status.code(), call_path.status.code());
}

/// INTENT: failure determinism — rerunning the same fault yields the identical
/// machine envelope shape (3×) and the identical human exit (2×).
/// KILLS: nondeterministic-envelope, flaky-exit.
/// ABSORBS: human_failure_deterministic_across_reruns (human leg folded into
/// the envelope-determinism anchor).
#[test]
fn operational_envelope_shape_deterministic_across_reruns() {
    // Leg 1 (anchor): machine envelope identical across 3 reruns.
    {
        let dir = kit::fixture_root();
        kit::index_root(dir.path());
        let root = dir.path().to_string_lossy().into_owned();
        let query = "regex:(unclosed";
        assert!(
            kit::lib_searcher(dir.path()).search(query).is_err()
                && kit::lib_searcher(dir.path()).search(query).is_err(),
            "lib must reject deterministically across reruns"
        );
        let first = kit::run(&["search", "--json", query, root.as_str()]);
        let second = kit::run(&["search", "--json", query, root.as_str()]);
        let third = kit::run(&["search", "--json", query, root.as_str()]);
        let v1 = kit::assert_failure_envelope(&first, "search", 2, "operational");
        let v2 = kit::assert_failure_envelope(&second, "search", 2, "operational");
        let v3 = kit::assert_failure_envelope(&third, "search", 2, "operational");
        assert_eq!(
            kit::envelope_shape(&v1),
            kit::envelope_shape(&v2),
            "rerun envelope shape must be identical"
        );
        assert_eq!(
            kit::envelope_shape(&v1),
            kit::envelope_shape(&v3),
            "rerun envelope shape must be identical"
        );
    }

    // Leg 2 (was human_failure_deterministic_across_reruns): human exit
    // identical across 2 reruns.
    {
        let dir = TempDir::new().expect("tempdir");
        let missing = dir.path().join("does-not-exist-xyz");
        let missing_arg = missing.to_string_lossy().into_owned();
        assert!(
            IndexStore::open_readonly(&missing, None).is_err()
                && IndexStore::open_readonly(&missing, None).is_err(),
            "lib must reject deterministically across reruns"
        );
        let first = kit::run(&["search", "greet", missing_arg.as_str()]);
        let second = kit::run(&["search", "greet", missing_arg.as_str()]);
        kit::assert_human_error(&first, 2);
        kit::assert_human_error(&second, 2);
        assert_eq!(
            first.status.code(),
            second.status.code(),
            "rerun exit code must be identical"
        );
    }
}

/// INTENT: a rejected codemod rewrites nothing — same failure both modes, file
/// bytes and tree listing unchanged.
/// KILLS: partial-write-on-failure.
/// ABSORBS: none (write-path fail-closed).
#[test]
fn failed_codemod_leaves_tree_untouched() {
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let target = dir.path().join("a.rs");

    assert!(
        plan_codemod(dir.path(), None, None, "", "x").is_err(),
        "lib plan_codemod must reject an empty pattern"
    );

    let before_bytes = std::fs::read(&target).expect("read fixture");
    let before_listing = kit::dir_listing(dir.path());

    let human = kit::run(&[
        "codemod",
        "--yes",
        "--pattern",
        "",
        "--rewrite",
        "x",
        root.as_str(),
    ]);
    kit::assert_human_error(&human, 2);
    let machine = kit::run(&[
        "codemod",
        "--json",
        "--yes",
        "--pattern",
        "",
        "--rewrite",
        "x",
        root.as_str(),
    ]);
    kit::assert_failure_envelope(&machine, "codemod", 2, "operational");

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
        kit::dir_listing(dir.path()),
        before_listing,
        "failed codemod must not add or drop tree entries"
    );
}

/// INTENT: a root fault creates no state — no conjured root, no partial index,
/// no parent entries.
/// KILLS: state-creation-on-failure.
/// ABSORBS: none (create-path fail-closed; distinct mechanism).
#[test]
fn failed_root_fault_creates_no_state() {
    let dir = TempDir::new().expect("tempdir");
    let missing = dir.path().join("does-not-exist-xyz");
    let missing_arg = missing.to_string_lossy().into_owned();

    assert!(
        IndexStore::open_readonly(&missing, None).is_err(),
        "lib must reject a missing root"
    );

    let before_listing = kit::dir_listing(dir.path());

    let search = kit::run(&["search", "--json", "greet", missing_arg.as_str()]);
    kit::assert_failure_envelope(&search, "search", 2, "operational");

    let call_path = kit::run(&[
        "call-path",
        "--json",
        "greet",
        "greet",
        missing_arg.as_str(),
    ]);
    kit::assert_failure_envelope(&call_path, "call-path", 2, "operational");

    assert!(
        !missing.exists(),
        "failure must not conjure the missing root into existence"
    );
    assert!(
        !dir.path().join(".asgrep").exists(),
        "failure must not write a partial index beside the missing root"
    );
    assert_eq!(
        kit::dir_listing(dir.path()),
        before_listing,
        "failure must not add or drop parent entries"
    );
}

/// INTENT: a malformed batch is deterministic across reruns with no partial
/// results and no success shape.
/// KILLS: half-ok-batch, nondeterministic-envelope.
/// ABSORBS: none.
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

    let first = kit::run(&["codemode-batch", "--requests", bad_arg.as_str()]);
    let second = kit::run(&["codemode-batch", "--requests", bad_arg.as_str()]);

    let v1 = kit::assert_failure_envelope(&first, "codemode-batch", 2, "operational");
    let v2 = kit::assert_failure_envelope(&second, "codemode-batch", 2, "operational");

    assert_eq!(
        kit::envelope_shape(&v1),
        kit::envelope_shape(&v2),
        "rerun envelope shape must be identical"
    );
    for (value, output) in [(&v1, &first), (&v2, &second)] {
        assert!(
            value.get("results").is_none(),
            "malformed batch must not present partial results: {value}"
        );
        kit::assert_no_success_shape(output);
    }
}

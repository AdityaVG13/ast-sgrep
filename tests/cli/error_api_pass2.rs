//! E2 error-propagation oracles: library errors reach the CLI surface faithfully.
//!
//! Complements `error_api_pass1.rs` (E1 taxonomy families). E2 proves
//! *propagation*: a hand-built trigger that makes a library function fail must
//! make the real `asgrep` binary fail the same way — same documented exit code
//! in human and `--json` modes, machine envelopes carrying the error
//! discriminant (`error.kind`), and no failure path printing a success shape.
//! Same-contract second triggers fold into their anchor (file-filter,
//! in-scope); every other contract pins its own test.
//!
//! Assertions cover exit codes and envelope shapes/counts only — never message
//! text. Each test drives a lib function directly (proving the library
//! rejects) plus the real binary in human and machine modes (proving the CLI
//! propagates). Fixtures are `tempfile` directories; no new dependencies.

#[path = "error_testkit.rs"]
mod kit;

use ast_sgrep_core::{
    call_path::{find_call_path, CallPathConfig},
    codemod::plan_codemod,
    Durability, IndexStore, SearchOptions, Searcher, MAX_FILE_FILTER_CHARS, MAX_QUERY_CHARS,
};

/// INTENT: oversize query rejected by lib reaches CLI as exit 2 both modes.
/// KILLS: swallow-to-Ok, exit-swap.
/// ABSORBS: none.
#[test]
fn query_too_long_search_propagates_operational() {
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let long = "x".repeat(MAX_QUERY_CHARS + 500);

    assert!(
        ast_sgrep_core::validate_query_len(&long).is_err(),
        "lib must reject an oversize query"
    );
    assert!(
        kit::lib_searcher(dir.path()).search(&long).is_err(),
        "lib Searcher::search must reject an oversize query"
    );

    let machine = kit::run(&["search", "--json", long.as_str(), root.as_str()]);
    kit::assert_failure_envelope(&machine, "search", 2, "operational");
    let human = kit::run(&["search", long.as_str(), root.as_str()]);
    kit::assert_human_error(&human, 2);
}

/// INTENT: invalid regex rejected by lib reaches CLI as exit 2 both modes.
/// KILLS: regex-fallback-to-literal, exit-swap.
/// ABSORBS: none.
#[test]
fn invalid_regex_search_propagates_operational() {
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let query = "regex:(unclosed";

    assert!(
        kit::lib_searcher(dir.path()).search(query).is_err(),
        "lib must reject an invalid regex"
    );

    let machine = kit::run(&["search", "--json", query, root.as_str()]);
    kit::assert_failure_envelope(&machine, "search", 2, "operational");
    let human = kit::run(&["search", query, root.as_str()]);
    kit::assert_human_error(&human, 2);
}

/// INTENT: file-filter rejections (oversize at `Searcher::new`, control chars
/// at finish) reach CLI as exit 2 both modes.
/// KILLS: filter-length-gate-drop, filter-content-gate-drop, exit-swap.
/// ABSORBS: file_filter_control_chars_propagate_operational (second leg; the
/// oversize leg is the original anchor).
#[test]
fn file_filter_too_long_propagates_operational() {
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();

    // Leg 1 (anchor): oversize filter rejected at Searcher::new.
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
    let machine = kit::run(&[
        "search",
        "--json",
        "--file-filter",
        long_filter.as_str(),
        "greet",
        root.as_str(),
    ]);
    kit::assert_failure_envelope(&machine, "search", 2, "operational");
    let human = kit::run(&[
        "search",
        "--file-filter",
        long_filter.as_str(),
        "greet",
        root.as_str(),
    ]);
    kit::assert_human_error(&human, 2);

    // Leg 2 (was file_filter_control_chars_propagate_operational): control-char
    // filter rejected at finish.
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
    let machine = kit::run(&[
        "search",
        "--json",
        "--file-filter",
        bad_filter,
        "greet",
        root.as_str(),
    ]);
    kit::assert_failure_envelope(&machine, "search", 2, "operational");
    let human = kit::run(&[
        "search",
        "--file-filter",
        bad_filter,
        "greet",
        root.as_str(),
    ]);
    kit::assert_human_error(&human, 2);
}

/// INTENT: `in:` scope rejections (missing scope, escaping scope) reach CLI as
/// exit 2 both modes.
/// KILLS: scope-miss-to-empty-Ok, scope-jail-drop, exit-swap.
/// ABSORBS: in_scope_escape_propagates_operational (second leg; the missing
/// leg is the original anchor).
#[test]
fn in_scope_missing_propagates_operational() {
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();

    for (leg, query) in [
        ("missing", "greet in:no-such-dir-xyz"),
        ("escaping", "greet in:../escape"),
    ] {
        assert!(
            kit::lib_searcher(dir.path()).search(query).is_err(),
            "lib must reject an {leg} in: scope"
        );
        let machine = kit::run(&["search", "--json", query, root.as_str()]);
        kit::assert_failure_envelope(&machine, "search", 2, "operational");
        let human = kit::run(&["search", query, root.as_str()]);
        kit::assert_human_error(&human, 2);
    }
}

/// INTENT: neural-embed/rerank lib-vs-CLI agree (both err or both ok per build).
/// KILLS: feature-gate-drop, gate-only-on-one-side.
/// ABSORBS: none (only cross-build agreement pin; success arm is BEHAVIOR-ONLY).
#[test]
fn feature_gates_propagate_between_lib_and_cli() {
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();

    let neural_opts = SearchOptions {
        root: dir.path().to_path_buf(),
        use_embed: true,
        use_neural_embed: true,
        ..SearchOptions::default()
    };
    let neural_lib_err = Searcher::new(neural_opts).is_err();
    let neural_machine = kit::run(&["search", "--json", "--neural-embed", "greet", root.as_str()]);
    let neural_human = kit::run(&["search", "--neural-embed", "greet", root.as_str()]);
    if neural_lib_err {
        kit::assert_failure_envelope(&neural_machine, "search", 2, "operational");
        kit::assert_human_error(&neural_human, 2);
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
    let rerank_machine = kit::run(&["search", "--json", "--rerank", "greet", root.as_str()]);
    let rerank_human = kit::run(&["search", "--rerank", "greet", root.as_str()]);
    if rerank_lib_err {
        kit::assert_failure_envelope(&rerank_machine, "search", 2, "operational");
        kit::assert_human_error(&rerank_human, 2);
    } else {
        assert_eq!(rerank_machine.status.code(), Some(0));
        assert_eq!(rerank_human.status.code(), Some(0));
    }
}

/// INTENT: non-DB --index-path rejected by lib reaches CLI as exit 2.
/// KILLS: Database-kind-drop, exit-swap.
/// ABSORBS: none. OVERLAP: recovery corrupt pins (adds lib+CLI agreement).
#[test]
fn corrupt_index_path_propagates_operational() {
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let bad = dir.path().join("bad-index.db");
    std::fs::write(&bad, "not a sqlite db").expect("write bad index");
    let bad_arg = bad.to_string_lossy().into_owned();

    assert!(
        IndexStore::open_readonly(dir.path(), Some(&bad)).is_err(),
        "lib must reject a non-database index path"
    );

    let machine = kit::run(&[
        "search",
        "--json",
        "--index-path",
        bad_arg.as_str(),
        "greet",
        root.as_str(),
    ]);
    kit::assert_failure_envelope(&machine, "search", 2, "operational");
    let human = kit::run(&[
        "search",
        "--index-path",
        bad_arg.as_str(),
        "greet",
        root.as_str(),
    ]);
    kit::assert_human_error(&human, 2);
}

/// INTENT: empty-pattern plan_codemod rejection reaches preview+apply as exit 2.
/// KILLS: preview-apply-divergence, exit-swap.
/// ABSORBS: none.
#[test]
fn codemod_empty_pattern_propagates_operational() {
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();

    assert!(
        plan_codemod(dir.path(), None, None, "", "x").is_err(),
        "lib plan_codemod must reject an empty pattern"
    );

    let dry = kit::run(&[
        "codemod",
        "--dry-run",
        "--pattern",
        "",
        "--rewrite",
        "x",
        root.as_str(),
    ]);
    kit::assert_failure_envelope(&dry, "codemod", 2, "operational");

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

    let apply_json = kit::run(&[
        "codemod",
        "--json",
        "--yes",
        "--pattern",
        "",
        "--rewrite",
        "x",
        root.as_str(),
    ]);
    kit::assert_failure_envelope(&apply_json, "codemod", 2, "operational");
}

/// INTENT: oversize call-path endpoint rejected by lib reaches CLI as exit 2.
/// KILLS: endpoint-length-gate-drop, exit-swap.
/// ABSORBS: none.
#[test]
fn call_path_oversize_propagates_operational() {
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let long = "y".repeat(MAX_QUERY_CHARS + 500);

    let store = IndexStore::open_readonly(dir.path(), None).expect("lib opens store");
    assert!(
        find_call_path(&store, &long, "greet", &CallPathConfig::default()).is_err(),
        "lib find_call_path must reject an oversize source"
    );

    let machine = kit::run(&["call-path", "--json", long.as_str(), "greet", root.as_str()]);
    kit::assert_failure_envelope(&machine, "call-path", 2, "operational");
    let human = kit::run(&["call-path", long.as_str(), "greet", root.as_str()]);
    kit::assert_human_error(&human, 2);
}

/// INTENT: unknown bench suite (lib None) reaches CLI as exit 2.
/// KILLS: suite-fallback-to-default, exit-swap.
/// ABSORBS: none.
#[test]
fn bench_unknown_suite_propagates_operational() {
    let dir = kit::fixture_root();
    let root = dir.path().to_string_lossy().into_owned();

    assert!(
        ast_sgrep_core::bench_suite::fixture_by_name("sample").is_some(),
        "lib knows the default bench fixture"
    );
    assert!(
        ast_sgrep_core::bench_suite::suite_by_name("no-such-suite-xyz").is_none(),
        "lib must reject an unknown bench suite"
    );

    let machine = kit::run(&[
        "bench",
        "--json",
        "--suite",
        "no-such-suite-xyz",
        root.as_str(),
    ]);
    kit::assert_failure_envelope(&machine, "bench", 2, "operational");
    let human = kit::run(&["bench", "--suite", "no-such-suite-xyz", root.as_str()]);
    kit::assert_human_error(&human, 2);
}

/// INTENT: unknown durability (lib None) reaches CLI as exit 1.
/// KILLS: durability-fallback-to-default, exit-swap.
/// ABSORBS: none (only usage-propagation row).
#[test]
fn durability_unknown_propagates_usage() {
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();

    assert!(
        Durability::parse("balanced").is_some(),
        "lib accepts a known durability"
    );
    assert!(
        Durability::parse("bogus-no-such-durability").is_none(),
        "lib must reject an unknown durability"
    );

    let machine = kit::run(&[
        "--json",
        "--durability",
        "bogus-no-such-durability",
        "search",
        "greet",
        root.as_str(),
    ]);
    kit::assert_failure_envelope(&machine, "search", 1, "usage");
    let human = kit::run(&[
        "--durability",
        "bogus-no-such-durability",
        "search",
        "greet",
        root.as_str(),
    ]);
    kit::assert_human_error(&human, 1);
}

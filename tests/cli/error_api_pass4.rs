//! E4 end-to-end error drills: fault a live tree, fail in both modes, recover clean.
//!
//! Complements `error_api_pass1.rs` (E1 taxonomy families),
//! `error_api_pass2.rs` (E2 lib-to-CLI propagation), and `error_api_pass3.rs`
//! (E3 failure relations) without duplicating them. E4 never asserts a single
//! cell in isolation: every test runs the REAL `asgrep` binary through a full
//! drill — index a tree, inject a fault (corrupt/delete state, bad args,
//! unreadable paths), observe the failure in BOTH human and `--json` modes,
//! then prove clean recovery (rebuild or fix inputs, success in both modes
//! again). Drills stay per-fault-class; the bad-lang arg drill folds into the
//! conflict-drill anchor as a second leg.
//!
//! Assertions cover exit codes and envelope shapes/counts only — never message
//! text. Fixtures are `tempfile` directories; no new dependencies.

#[path = "error_testkit.rs"]
mod kit;

/// Recovered search over the fixture: machine success with a non-empty hit
/// list plus human success.
fn recover_search_both_modes(root: &str) {
    let recovered = kit::run(&["search", "--json", "greet", root]);
    let value = kit::assert_success(&recovered, "search");
    kit::assert_fixture_hits(&value);
    kit::assert_human_success(&kit::run(&["search", "greet", root]));
}

/// INTENT: corrupt db fails 2 both modes, reindex heals, hits return.
/// KILLS: heal-path-regression, stale-hits-after-corrupt.
/// ABSORBS: none. OVERLAP: recovery (adds both-modes + hit-list proof).
#[test]
fn corrupt_index_db_search_drill_reindex_heals() {
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let db = kit::index_db_path(dir.path());
    assert!(db.is_file(), "index must create {}", db.display());

    kit::corrupt_index_db(&db);

    let human = kit::run(&["search", "greet", root.as_str()]);
    kit::assert_human_error(&human, 2);
    let machine = kit::run(&["search", "--json", "greet", root.as_str()]);
    kit::assert_failure_envelope(&machine, "search", 2, "operational");

    let healed = kit::run(&["--json", "reindex", root.as_str()]);
    let value = kit::assert_success(&healed, "reindex");
    assert!(
        value["files_indexed"].as_u64().unwrap_or(0) >= 1,
        "reindex must rebuild rows: {value}"
    );
    assert!(db.is_file(), "healed state must exist at {}", db.display());

    recover_search_both_modes(&root);
}

/// INTENT: deleted .asgrep fails 2 both modes, fresh index heals.
/// KILLS: heal-path-regression.
/// ABSORBS: none (distinct heal path from reindex).
#[test]
fn deleted_index_state_search_drill_index_heals() {
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let db = kit::index_db_path(dir.path());
    assert!(db.is_file(), "index must create {}", db.display());

    std::fs::remove_dir_all(dir.path().join(".asgrep")).expect("delete state");
    assert!(!db.exists(), "fault must remove {}", db.display());

    let human = kit::run(&["search", "greet", root.as_str()]);
    kit::assert_human_error(&human, 2);
    let machine = kit::run(&["search", "--json", "greet", root.as_str()]);
    kit::assert_failure_envelope(&machine, "search", 2, "operational");

    let rebuilt = kit::run(&["--json", "index", root.as_str()]);
    let value = kit::assert_success(&rebuilt, "index");
    assert!(
        value["files_indexed"].as_u64().unwrap_or(0) >= 1,
        "fresh index must rebuild rows: {value}"
    );
    assert!(db.is_file(), "rebuilt state must exist at {}", db.display());

    recover_search_both_modes(&root);
}

/// INTENT: arg-fault recovery drill — an arg fault fails 1 in both modes and
/// fixing the args recovers clean, for both the bad-`--lang` and the
/// QUERY+--pattern-conflict faults.
/// KILLS: arg-fault-sticks-after-fix.
/// ABSORBS: bad_lang_arg_search_drill_fix_arg_recovers (first leg; the
/// conflict leg is the original anchor).
#[test]
fn query_pattern_conflict_drill_single_query_recovers() {
    // Leg 1 (was bad_lang_arg_search_drill_fix_arg_recovers): unknown --lang
    // fails 1 both modes, valid args recover.
    {
        let dir = kit::fixture_root();
        kit::index_root(dir.path());
        let root = dir.path().to_string_lossy().into_owned();
        let human = kit::run(&["--lang", "xx-no-such-lang", "search", "greet", root.as_str()]);
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
        recover_search_both_modes(&root);
    }

    // Leg 2 (anchor): QUERY+--pattern conflict fails 1 both modes, a single
    // query recovers.
    {
        let dir = kit::fixture_root();
        kit::index_root(dir.path());
        let root = dir.path().to_string_lossy().into_owned();
        let human = kit::run(&["search", "--pattern", "greet", "greet", root.as_str()]);
        kit::assert_human_error(&human, 1);
        let machine = kit::run(&["search", "--json", "--pattern", "greet", "greet", root.as_str()]);
        kit::assert_failure_envelope(&machine, "search", 1, "usage");
        recover_search_both_modes(&root);
    }
}

/// INTENT: invalid regex fails 2 both modes, valid query recovers.
/// KILLS: query-fault-poisons-handle.
/// ABSORBS: none (operational query-fix drill).
#[test]
fn invalid_regex_query_drill_fix_query_recovers() {
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let query = "regex:(unclosed";

    let human = kit::run(&["search", query, root.as_str()]);
    kit::assert_human_error(&human, 2);
    let machine = kit::run(&["search", "--json", query, root.as_str()]);
    kit::assert_failure_envelope(&machine, "search", 2, "operational");

    recover_search_both_modes(&root);
}

/// INTENT: directory-as-gold fails 2 both modes, valid gold recovers with a
/// scored query.
/// KILLS: gold-fault-sticks-after-fix.
/// ABSORBS: none (only eval drill).
#[test]
fn unreadable_gold_eval_drill_valid_gold_recovers() {
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();

    let gold_path = dir.path().join("gold.json");
    std::fs::create_dir(&gold_path).expect("gold path starts as a directory");
    let gold_arg = gold_path.to_string_lossy().into_owned();

    let human = kit::run(&["eval", "--gold", gold_arg.as_str(), root.as_str()]);
    kit::assert_human_error(&human, 2);
    let machine = kit::run(&["eval", "--json", "--gold", gold_arg.as_str(), root.as_str()]);
    kit::assert_failure_envelope(&machine, "eval", 2, "operational");

    std::fs::remove_dir(&gold_path).expect("remove gold directory");
    std::fs::write(
        &gold_path,
        r#"{"corpus":"e4","queries":[{"name":"g1","query":"greet","k":5,"relevant":[{"file":"a.rs","symbol":"greet"}]}]}"#,
    )
    .expect("write valid gold");

    let recovered = kit::run(&["eval", "--json", "--gold", gold_arg.as_str(), root.as_str()]);
    let value = kit::assert_success(&recovered, "eval");
    assert_eq!(
        value["queries"].as_array().map(Vec::len),
        Some(1),
        "recovered eval must score one query: {value}"
    );
    assert_eq!(value["aggregate"]["n_queries"], 1);
    kit::assert_human_success(&kit::run(&["eval", "--gold", gold_arg.as_str(), root.as_str()]));
}

/// INTENT: outline missing file fails 2 both modes, indexed file recovers with
/// count.
/// KILLS: outline-fault-sticks-after-fix.
/// ABSORBS: none (only outline drill).
#[test]
fn outline_missing_file_drill_indexed_file_recovers() {
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();

    let human = kit::run(&["outline", "no-such-file.rs", root.as_str()]);
    kit::assert_human_error(&human, 2);
    let machine = kit::run(&["outline", "--json", "no-such-file.rs", root.as_str()]);
    kit::assert_failure_envelope(&machine, "outline", 2, "operational");

    let recovered = kit::run(&["outline", "--json", "a.rs", root.as_str()]);
    let value = kit::assert_success(&recovered, "outline");
    let symbols = value["symbols"]
        .as_array()
        .expect("outline success must carry a symbols array");
    assert!(!symbols.is_empty(), "indexed file must outline: {value}");
    assert_eq!(
        value["count"].as_u64(),
        Some(symbols.len() as u64),
        "outline count must match symbols length: {value}"
    );
    kit::assert_human_success(&kit::run(&["outline", "a.rs", root.as_str()]));
}

/// INTENT: usage fault wins over corrupt state, then operational, then success.
/// KILLS: validation-order-swap (arg-after-state).
/// ABSORBS: none (only precedence + ordered-recovery pin).
#[test]
fn chained_corrupt_plus_bad_arg_ordered_recovery() {
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let db = kit::index_db_path(dir.path());
    assert!(db.is_file(), "index must create {}", db.display());

    kit::corrupt_index_db(&db);

    // Both faults present: arg validation runs before state open, so usage/1 wins.
    let human = kit::run(&["--lang", "xx-no-such-lang", "search", "greet", root.as_str()]);
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

    // Fix the arg: the corrupt-state fault surfaces as operational/2.
    let human = kit::run(&["search", "greet", root.as_str()]);
    kit::assert_human_error(&human, 2);
    let machine = kit::run(&["search", "--json", "greet", root.as_str()]);
    kit::assert_failure_envelope(&machine, "search", 2, "operational");

    // Heal the state: reindex, then clean search in both modes.
    let healed = kit::run(&["--json", "reindex", root.as_str()]);
    kit::assert_success(&healed, "reindex");
    assert!(db.is_file(), "healed state must exist at {}", db.display());

    recover_search_both_modes(&root);
}

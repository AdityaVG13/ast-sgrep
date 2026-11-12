//! Cross-surface fault-corpus agreement: core and CLI answer the same on the
//! six duplicated fault behaviors in `ast_sgrep_testkit::FAULT_CORPUS`.
//!
//! Each test drives the fault below the library (asserting the owning core
//! discriminant from its corpus row) and through the real `asgrep` binary in
//! both human and `--json` modes (asserting the witness exit + envelope kind
//! from the same row). Expectations come from
//! `ast_sgrep_testkit::agreement`, never from literals restated here — a
//! contract change edits the corpus row, and every test below follows.
//!
//! Complements, never replaces, the thin witnesses: `tests/core`
//! (`error_api_matrix`) owns the lib-discriminant contract and `tests/cli`
//! (`error_api_pass{1,2,3,4}`) owns the exit-code/envelope contract. This
//! target only proves the two surfaces agree row by row.

#[path = "error_testkit.rs"]
mod kit;

use ast_sgrep_core::{
    IndexOptions, IndexStore, Indexer, SearchOptions, Searcher, StoreError, MAX_FILE_FILTER_CHARS,
    MAX_QUERY_CHARS,
};
use ast_sgrep_testkit::{
    agreement, err_of, is_corrupt_kind, store_error_discriminant, FaultBehavior,
};
use tempfile::TempDir;

/// Recovered search over the fixture: machine success with a non-empty hit
/// list plus human success (same bar as the E4 drills).
fn recover_search_both_modes(root: &str) {
    let recovered = kit::run(&["search", "--json", "greet", root]);
    let value = kit::assert_success(&recovered, "search");
    kit::assert_fixture_hits(&value);
    kit::assert_human_success(&kit::run(&["search", "greet", root]));
}

/// INTENT: corpus row 1 — garbage bytes at the index path fail as Database
/// in core and operational/2 on the CLI; reindex heals both.
/// KILLS: discriminant-swap(Database→Other), exit-swap, heal-path-regression.
/// WITNESSES: core `error_api_matrix` E1/E2 garbage rows; CLI
/// `error_api_pass2::corrupt_index_path_propagates_operational`,
/// `error_api_pass4::corrupt_index_db_search_drill_reindex_heals`.
#[test]
fn garbage_index_db_core_and_cli_agree() {
    let row = agreement(FaultBehavior::GarbageIndexDb);

    // Owner leg: the library rejects garbage as the row's discriminant and
    // the kind stays corrupt-detectable.
    {
        let temp = TempDir::new().unwrap();
        let db = temp.path().join("index.db");
        ast_sgrep_testkit::write_garbage(&db);
        let err = err_of(IndexStore::open(temp.path(), Some(&db)));
        assert_eq!(
            store_error_discriminant(&err),
            row.core_discriminant,
            "garbage index must fail with the corpus discriminant, got {err:?}"
        );
        assert!(
            is_corrupt_kind(&err),
            "garbage index must stay corrupt-detectable, got {err:?}"
        );
    }

    // Witness legs: the binary fails in both modes, then reindex heals.
    {
        let dir = kit::fixture_root();
        kit::index_root(dir.path());
        let root = dir.path().to_string_lossy().into_owned();
        let db = kit::index_db_path(dir.path());
        assert!(db.is_file(), "index must create {}", db.display());
        kit::corrupt_index_db(&db);

        kit::assert_human_error(&kit::run(&["search", "greet", root.as_str()]), row.cli_exit);
        let machine = kit::run(&["search", "--json", "greet", root.as_str()]);
        kit::assert_failure_envelope(&machine, "search", row.cli_exit, row.cli_kind);

        let healed = kit::run(&["--json", "reindex", root.as_str()]);
        let value = kit::assert_success(&healed, "reindex");
        assert!(
            value["files_indexed"].as_u64().unwrap_or(0) >= 1,
            "reindex must rebuild rows: {value}"
        );
        recover_search_both_modes(&root);
    }
}

/// INTENT: corpus row 2 — a missing root fails closed in core (Other at the
/// read layer the CLI exercises; Io at the write layer) and
/// operational/2 on the CLI in both modes.
/// KILLS: conversion-drop(raw-Io-leak), exit-swap, mode-divergence.
/// WITNESSES: core `error_api_matrix`
/// `indexer_new_missing_root_surfaces_io` /
/// `searcher_new_missing_root_converts_to_other`; CLI
/// `error_api_pass3::missing_root_human_and_json_agree_on_operational`.
#[test]
fn missing_root_core_and_cli_agree() {
    let row = agreement(FaultBehavior::MissingRoot);

    // Owner legs: read layer converts to the row's discriminant; write layer
    // keeps the raw Io (documented layering, same as the matrix).
    {
        let temp = TempDir::new().unwrap();
        let missing = temp.path().join("does-not-exist-xyz");
        let err = err_of(Searcher::new(SearchOptions {
            root: missing.clone(),
            ..SearchOptions::default()
        }));
        assert_eq!(
            store_error_discriminant(&err),
            row.core_discriminant,
            "Searcher::new on a missing root must fail with the corpus discriminant, got {err:?}"
        );
        let db = temp.path().join("index.db");
        let err = err_of(Indexer::new(IndexOptions {
            root: missing,
            index_path: Some(db),
            ..IndexOptions::default()
        }));
        assert!(
            matches!(err, StoreError::Io(_)),
            "Indexer::new on a missing root must surface Io, got {err:?}"
        );
    }

    // Witness legs: the binary fails in both modes with the same exit.
    {
        let scratch = TempDir::new().expect("tempdir");
        let missing = scratch.path().join("does-not-exist-xyz");
        let missing_arg = missing.to_string_lossy().into_owned();
        let human = kit::run(&["search", "greet", missing_arg.as_str()]);
        kit::assert_human_error(&human, row.cli_exit);
        let machine = kit::run(&["search", "--json", "greet", missing_arg.as_str()]);
        kit::assert_failure_envelope(&machine, "search", row.cli_exit, row.cli_kind);
        assert_eq!(
            human.status.code(),
            machine.status.code(),
            "same fault must fail in both modes, never succeed in one"
        );
    }
}

/// INTENT: corpus row 3 — an oversize query is refused as Other in core and
/// operational/2 on the CLI in both modes.
/// KILLS: ingress-wiring-drop, swallow-to-Ok, exit-swap.
/// WITNESSES: core `error_api_matrix::search_rejects_oversize_query`
/// (anchor leg); CLI
/// `error_api_pass2::query_too_long_search_propagates_operational`.
#[test]
fn oversize_query_core_and_cli_agree() {
    let row = agreement(FaultBehavior::OversizeQuery);
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let long = "x".repeat(MAX_QUERY_CHARS + 500);

    // Owner leg: the library rejects before touching store state.
    {
        let err = err_of(kit::lib_searcher(dir.path()).search(&long));
        assert_eq!(
            store_error_discriminant(&err),
            row.core_discriminant,
            "oversize query must fail with the corpus discriminant, got {err:?}"
        );
    }

    // Witness legs: the binary fails in both modes.
    {
        let machine = kit::run(&["search", "--json", long.as_str(), root.as_str()]);
        kit::assert_failure_envelope(&machine, "search", row.cli_exit, row.cli_kind);
        let human = kit::run(&["search", long.as_str(), root.as_str()]);
        kit::assert_human_error(&human, row.cli_exit);
    }
}

/// INTENT: corpus row 4 — an invalid regex is refused as Other in core and
/// operational/2 on the CLI in both modes; a valid query recovers.
/// KILLS: regex-fallback-to-literal, query-fault-poisons-handle, exit-swap.
/// WITNESSES: core `error_api_matrix::search_rejects_oversize_query`
/// (invalid-regex leg); CLI
/// `error_api_pass2::invalid_regex_search_propagates_operational`,
/// `error_api_pass4::invalid_regex_query_drill_fix_query_recovers`.
#[test]
fn invalid_regex_core_and_cli_agree() {
    let row = agreement(FaultBehavior::InvalidRegex);
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let query = "regex:(unclosed";

    // Owner leg: the library rejects the invalid regex.
    {
        let err = err_of(kit::lib_searcher(dir.path()).search(query));
        assert_eq!(
            store_error_discriminant(&err),
            row.core_discriminant,
            "invalid regex must fail with the corpus discriminant, got {err:?}"
        );
    }

    // Witness legs: the binary fails in both modes, then a valid query
    // recovers (the fault poisons nothing).
    {
        let human = kit::run(&["search", query, root.as_str()]);
        kit::assert_human_error(&human, row.cli_exit);
        let machine = kit::run(&["search", "--json", query, root.as_str()]);
        kit::assert_failure_envelope(&machine, "search", row.cli_exit, row.cli_kind);
        recover_search_both_modes(&root);
    }
}

/// INTENT: corpus row 5 — an oversize file-filter is refused as Other in core
/// and operational/2 on the CLI in both modes.
/// KILLS: filter-length-gate-drop, exit-swap.
/// WITNESSES: core `error_api_matrix::search_rejects_oversize_query`
/// (oversize-filter leg); CLI
/// `error_api_pass2::file_filter_too_long_propagates_operational` (anchor leg).
#[test]
fn oversize_file_filter_core_and_cli_agree() {
    let row = agreement(FaultBehavior::OversizeFileFilter);
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let long_filter = "a".repeat(MAX_FILE_FILTER_CHARS + 100);

    // Owner leg: the library rejects the oversize filter at construction.
    {
        let opts = SearchOptions {
            root: dir.path().to_path_buf(),
            file_filter: Some(long_filter.clone()),
            ..SearchOptions::default()
        };
        let err = err_of(Searcher::new(opts));
        assert_eq!(
            store_error_discriminant(&err),
            row.core_discriminant,
            "oversize file_filter must fail with the corpus discriminant, got {err:?}"
        );
    }

    // Witness legs: the binary fails in both modes.
    {
        let machine = kit::run(&[
            "search",
            "--json",
            "--file-filter",
            long_filter.as_str(),
            "greet",
            root.as_str(),
        ]);
        kit::assert_failure_envelope(&machine, "search", row.cli_exit, row.cli_kind);
        let human = kit::run(&[
            "search",
            "--file-filter",
            long_filter.as_str(),
            "greet",
            root.as_str(),
        ]);
        kit::assert_human_error(&human, row.cli_exit);
    }
}

/// INTENT: corpus row 6 — a deleted index fails closed as Other in core and
/// operational/2 on the CLI in both modes; a fresh index heals both.
/// KILLS: silent-Ok-on-missing-state, heal-path-regression.
/// WITNESSES: core `error_api_matrix`
/// `drill_deleted_index_search_fails_closed_then_reindex_recovers`; CLI
/// `error_api_pass4::deleted_index_state_search_drill_index_heals`.
#[test]
fn deleted_index_state_core_and_cli_agree() {
    let row = agreement(FaultBehavior::DeletedIndexState);
    let dir = kit::fixture_root();
    kit::index_root(dir.path());
    let root = dir.path().to_string_lossy().into_owned();
    let db = kit::index_db_path(dir.path());
    assert!(db.is_file(), "index must create {}", db.display());

    std::fs::remove_dir_all(dir.path().join(".asgrep")).expect("delete state");
    assert!(!db.exists(), "fault must remove {}", db.display());

    // Owner leg: the read layer fails closed with the corpus discriminant
    // (same trigger shape as the core drill: explicit db path, no recovery).
    {
        let err = err_of(Searcher::new(SearchOptions {
            root: dir.path().to_path_buf(),
            index_path: Some(db.clone()),
            ..SearchOptions::default()
        }));
        assert_eq!(
            store_error_discriminant(&err),
            row.core_discriminant,
            "deleted index must fail with the corpus discriminant, got {err:?}"
        );
    }

    // Witness legs: the binary fails in both modes, then a fresh index
    // heals and search recovers with hits.
    {
        let human = kit::run(&["search", "greet", root.as_str()]);
        kit::assert_human_error(&human, row.cli_exit);
        let machine = kit::run(&["search", "--json", "greet", root.as_str()]);
        kit::assert_failure_envelope(&machine, "search", row.cli_exit, row.cli_kind);

        let rebuilt = kit::run(&["--json", "index", root.as_str()]);
        let value = kit::assert_success(&rebuilt, "index");
        assert!(
            value["files_indexed"].as_u64().unwrap_or(0) >= 1,
            "fresh index must rebuild rows: {value}"
        );
        assert!(db.is_file(), "rebuilt state must exist at {}", db.display());
        recover_search_both_modes(&root);
    }
}

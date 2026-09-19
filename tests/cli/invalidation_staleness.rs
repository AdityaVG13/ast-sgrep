//! CLI invalidation: staleness contracts (counts, selection, discriminants).
//!
//! Canonical successor of `invalidation_pass1.rs` (I1) per
//! `tests/catalog/invalidation-cli.md`. Prior-art non-duplication notes from
//! the I1 header still hold (watch_incremental, watch_daemon_e2e,
//! machine_contracts targeted-count/dry-run beats, cli_smoke refusal beats,
//! durable_recovery reindex-idempotence relation).
//!
//! Absorption map: the 3 MERGE→matrix/staleness-counts tests fuse into
//! `matrix_staleness_counts`; the I1 targeted-noop MERGE moved to
//! `matrix_targeted_refresh` in `invalidation_parity.rs`; the other 6 I1
//! verdicts are KEEPs (the 2 watch-gate KEEPs live in
//! `invalidation_drills.rs` next to the watch-mode drill).

#[path = "invalidation_common.rs"]
mod common;

use ast_sgrep_testkit::{asgrep_bin, assert_success, parse_stdout, run};
use common::*;
use serde_json::Value;
use std::fs;

/// INTENT: one freshness-count matrix over a single evolving fixture —
/// no-change reuse row (0 indexed / 2 skipped), stale-rebuild row (1/1),
/// prune row (removed=1, status file_count drops to 1).
/// KILLS: always-rebuild (freshness-check deletion) | stale-row-reuse |
/// rebuild-all | prune-omission.
/// ABSORBS: `index_second_run_reuses_fresh_rows` + `index_rebuilds_only_stale_file_after_edit`
/// + `index_refresh_prunes_deleted_file` (I1 MERGE→matrix/staleness-counts ×3).
#[test]
fn matrix_staleness_counts() {
    let bin = asgrep_bin();
    let (_dir, root, _index, root_s, index_s) = fixture_two_files();

    let first = run_index(&bin, &index_s, &root_s);
    assert_eq!(first["files_indexed"], 2, "{first}");
    assert_eq!(first["exit_code"], 0, "{first}");

    // Row 1 — reuse: a no-change second index reuses all rows.
    let second = run_index(&bin, &index_s, &root_s);
    assert_eq!(second["files_indexed"], 0, "fresh rows must be reused: {second}");
    assert_eq!(second["files_skipped"], 2, "{second}");
    assert_eq!(second["files_removed"], 0, "{second}");
    assert_eq!(second["files_failed"], 0, "{second}");

    // Row 2 — rebuild-stale: post-edit refresh rebuilds only the stale file.
    rewrite_with_mtime_bump(
        &root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\npub fn alpha_two() -> u32 { 2 }\n",
    );
    let refreshed = run_index(&bin, &index_s, &root_s);
    assert_eq!(refreshed["files_indexed"], 1, "{refreshed}");
    assert_eq!(refreshed["files_skipped"], 1, "{refreshed}");
    assert_eq!(refreshed["files_removed"], 0, "{refreshed}");

    // Row 3 — prune: refresh prunes the deleted file and status drops to 1.
    fs::remove_file(root.join("beta.rs")).expect("delete beta");
    let pruned = run_index(&bin, &index_s, &root_s);
    assert_eq!(pruned["files_removed"], 1, "{pruned}");
    assert_eq!(pruned["files_indexed"], 0, "{pruned}");
    let status = run_status(&bin, &index_s, &root_s);
    assert_eq!(status["file_count"], 1, "{status}");
}

/// INTENT: reindex ignores freshness and rewrites all rows with zero edits.
/// KILLS: reindex-degrades-to-index (freshness-reuse-in-reindex).
/// ABSORBS: none — KEEP of `reindex_rewrites_all_rows_without_edits` (I1).
#[test]
fn reindex_rewrites_all_rows_without_edits() {
    let bin = asgrep_bin();
    let (_dir, _root, _index, root_s, index_s) = fixture_two_files();
    run_index(&bin, &index_s, &root_s);

    let rebuilt = run_reindex(&bin, &index_s, &root_s);
    assert_eq!(
        rebuilt["files_indexed"], 2,
        "reindex must ignore freshness and rewrite all rows: {rebuilt}"
    );
    assert_eq!(rebuilt["files_skipped"], 0, "{rebuilt}");
}

/// INTENT: reindex --dry-run reports counts and creates no DB file.
/// KILLS: write-despite-dry-run.
/// ABSORBS: none — KEEP of `reindex_dry_run_reports_without_writing` (I1).
#[test]
fn reindex_dry_run_reports_without_writing() {
    let bin = asgrep_bin();
    let (_dir, _root, index, root_s, index_s) = fixture_two_files();
    let value = assert_success(
        &run(
            &bin,
            &[
                "--json",
                "--no-embed",
                "--index-path",
                &index_s,
                "reindex",
                "--dry-run",
                &root_s,
            ],
        ),
        "reindex",
    );
    assert_eq!(value["dry_run"], true, "{value}");
    assert_eq!(value["mutates_index"], false, "{value}");
    assert_eq!(value["files_would_index"], 2, "{value}");
    assert!(
        !index.exists(),
        "reindex --dry-run must not create the index database"
    );
}

/// INTENT: status refuses a missing index without creating a DB, and types
/// discriminants when fresh.
/// KILLS: refusal-inversion | create-on-status.
/// ABSORBS: none — KEEP of `status_discriminants_fresh_vs_missing_index` (I1).
#[test]
fn status_discriminants_fresh_vs_missing_index() {
    let bin = asgrep_bin();
    let (_dir, _root, index, root_s, index_s) = fixture_two_files();

    // Missing index: status refuses (operational) and creates nothing.
    let missing = run(
        &bin,
        &[
            "--json",
            "--no-embed",
            "--index-path",
            &index_s,
            "status",
            &root_s,
        ],
    );
    assert_eq!(missing.status.code(), Some(2));
    let missing_value = parse_stdout(&missing);
    assert_eq!(missing_value["ok"], false, "{missing_value}");
    assert_eq!(missing_value["error"]["kind"], "operational", "{missing_value}");
    assert!(
        !index.exists(),
        "status over a missing index must not create the database"
    );

    // Fresh index: count and freshness discriminants are present and typed.
    run_index(&bin, &index_s, &root_s);
    let status = run_status(&bin, &index_s, &root_s);
    assert_eq!(status["file_count"], 2, "{status}");
    assert!(status["writer_generation"].is_u64(), "{status}");
    assert!(
        status["durability"].as_str().is_some_and(|d| !d.is_empty()),
        "{status}"
    );
    assert!(status["semantic_ivf_present"].is_boolean(), "{status}");
    assert!(status["symbol_count"].is_u64(), "{status}");
}

/// INTENT: --auto-index refreshes a stale index before serving the new symbol.
/// KILLS: auto-index-no-refresh (serves-stale).
/// ABSORBS: none — KEEP of `search_auto_index_refreshes_stale_index_after_edit` (I1).
#[test]
fn search_auto_index_refreshes_stale_index_after_edit() {
    let bin = asgrep_bin();
    let (_dir, root, _index, root_s, index_s) = fixture_two_files();
    run_index(&bin, &index_s, &root_s);

    rewrite_with_mtime_bump(
        &root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\npub fn refreshed_after_stale_edit() -> u32 { 3 }\n",
    );
    let value = assert_success(
        &run(
            &bin,
            &[
                "--auto-index",
                "--json",
                "--no-embed",
                "--index-path",
                &index_s,
                "search",
                "word:refreshed_after_stale_edit",
                &root_s,
            ],
        ),
        "search",
    );
    let hits = value["hits"].as_array().expect("hits array");
    assert!(
        !hits.is_empty(),
        "--auto-index must refresh a stale index before serving: {value}"
    );
}

/// INTENT: --no-auto-index wins over --auto-index (refuses on empty, silent
/// stderr).
/// KILLS: precedence-inversion.
/// ABSORBS: none — KEEP of `auto_index_plus_no_auto_index_refuses` (I1).
#[test]
fn auto_index_plus_no_auto_index_refuses() {
    let bin = asgrep_bin();
    let (_dir, _root, _index, root_s, index_s) = fixture_two_files();
    // Never indexed: with both flags, --no-auto-index wins (no auto-index).
    let output = run(
        &bin,
        &[
            "--auto-index",
            "--no-auto-index",
            "--json",
            "--no-embed",
            "--index-path",
            &index_s,
            "search",
            "word:alpha_one",
            &root_s,
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stderr.is_empty(), "machine mode must stay silent");
    let value: Value = parse_stdout(&output);
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["error"]["kind"], "operational", "{value}");
}

/// INTENT: IndexStats::mutated true iff indexed|removed nonzero, false on
/// skipped|failed (lib unit behind ensure_fresh_index).
/// KILLS: predicate-arm-deletion (||→&&, dropped-removed-arm).
/// ABSORBS: none — KEEP of `mutated_predicate_truth_table` (I1).
#[test]
fn mutated_predicate_truth_table() {
    use ast_sgrep_core::IndexStats;
    // Reuse: nothing written or deleted -> no reopen needed.
    assert!(!IndexStats::default().mutated());
    assert!(
        !IndexStats {
            files_skipped: 3,
            ..IndexStats::default()
        }
        .mutated()
    );
    // A failed write leaves no new rows behind -> still no mutation.
    assert!(
        !IndexStats {
            files_failed: 1,
            ..IndexStats::default()
        }
        .mutated()
    );
    // Any written or deleted file row counts as a mutation.
    assert!(
        IndexStats {
            files_indexed: 1,
            ..IndexStats::default()
        }
        .mutated()
    );
    assert!(
        IndexStats {
            files_removed: 1,
            ..IndexStats::default()
        }
        .mutated()
    );
    assert!(
        IndexStats {
            files_indexed: 1,
            files_removed: 1,
            files_skipped: 9,
            ..IndexStats::default()
        }
        .mutated()
    );
}

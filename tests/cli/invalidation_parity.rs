//! CLI invalidation: rebuild-parity relations + targeted-refresh matrix.
//!
//! Canonical successor of `invalidation_pass3.rs` (I3) per
//! `tests/catalog/invalidation-cli.md`, plus the cross-file targeted-refresh
//! matrix. Relations between refresh paths over the same final tree, observed
//! via served bytes with `--no-auto-index`:
//!
//! - `matrix_targeted_refresh`: noop counts + edit-served + targeted==full
//!   bytes (I1+I2+I3 fan-in).
//! - `matrix_parity_incremental_vs_clean`: incremental == clean rebuild over
//!   search+outline bytes.
//! - `matrix_parity_order_independence`: delta application order unobservable.
//! - `matrix_parity_reindex`: reindex idempotent, reindex == clean rebuild.
//! - `i3_converged_refresh_changes_no_observable_bytes`: post-convergence
//!   fixpoint (KEEP).
//!
//! DELETE honored: `i3_incremental_refresh_matches_clean_rebuild_counts` is
//! gone — status/hit counts are strictly implied by the sibling search+outline
//! byte-identity relations in `matrix_parity_incremental_vs_clean`.

#[path = "invalidation_common.rs"]
mod common;

use ast_sgrep_testkit::{asgrep_bin, assert_success, parse_stdout, run, set_mtime_secs};
use common::*;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

// why: a second, independent index database over the same root for two-path
// relations. WHY area-local: parity-harness-specific — single-suite helper.
fn second_index_path(dir: &tempfile::TempDir) -> String {
    dir.path()
        .join("idx2")
        .join("index.db")
        .to_str()
        .expect("index2 utf8")
        .to_owned()
}

// why: targeted `--path` refresh beat returning the success envelope.
// WHY area-local: no targeted-refresh runner exists in testkit, and only this
// suite drives `--path` refresh — single-suite helper.
fn run_targeted(bin: &Path, index_s: &str, root_s: &str, rel: &str) -> Value {
    assert_success(
        &run(
            bin,
            &[
                "--json",
                "--no-embed",
                "--index-path",
                index_s,
                "index",
                root_s,
                "--path",
                rel,
            ],
        ),
        "index",
    )
}

// why: raw served search output — exit code + stdout bytes are the relation.
// WHY area-local: byte-relation probe (testkit search runners assert success);
// only this suite compares raw served bytes — single-suite helper.
fn run_search_raw(bin: &Path, index_s: &str, root_s: &str, query: &str) -> Output {
    run(
        bin,
        &[
            "--json",
            "--no-embed",
            "--no-auto-index",
            "--index-path",
            index_s,
            "search",
            query,
            root_s,
        ],
    )
}

// why: raw `status` run — the normalized-bytes relation needs the raw Output.
// WHY area-local: same as `run_search_raw`.
fn run_status_raw(bin: &Path, index_s: &str, root_s: &str) -> Output {
    run(
        bin,
        &[
            "--json",
            "--no-embed",
            "--index-path",
            index_s,
            "status",
            root_s,
        ],
    )
}

// why: queries spanning added, modified, retained, and deleted symbols. Served
// over a converged index, the (exit code, stdout bytes) vector fully
// characterizes the observable search state for this fixture family.
// WHY area-local: parity-probe-specific constant — single-suite helper.
const PROBE_QUERIES: &[&str] = &[
    "word:alpha_one",
    "word:alpha_two",
    "word:beta_one",
    "word:gamma_new",
];

// why: search envelope with the write counter removed. `snapshot.generation`
// counts index writes, so it legitimately differs between an incrementally
// refreshed database and a clean rebuild (or across two reindexes); every
// other field — hits, lines, scores, `worktree_revision` — must agree.
// WHY area-local: parity-normalization-specific — single-suite helper.
fn normalized_search_bytes(output: &Output) -> Vec<u8> {
    let mut value = parse_stdout(output);
    if let Some(map) = value
        .get_mut("snapshot")
        .and_then(|snapshot| snapshot.as_object_mut())
    {
        map.remove("generation");
    }
    serde_json::to_vec(&value).expect("reserialize normalized search")
}

// why: one (exit code, normalized bytes) snapshot per probe query.
// WHY area-local: same as `normalized_search_bytes`.
fn snapshot_search(bin: &Path, index_s: &str, root_s: &str) -> Vec<(Option<i32>, Vec<u8>)> {
    PROBE_QUERIES
        .iter()
        .map(|query| {
            let output = run_search_raw(bin, index_s, root_s, query);
            (output.status.code(), normalized_search_bytes(&output))
        })
        .collect()
}

// why: status envelope with the per-process writer token removed.
// `writer_generation` is random per process open, so it cannot participate
// in any cross-invocation relation; all other fields must agree.
// WHY area-local: same as `normalized_search_bytes`.
fn normalized_status_bytes(output: &Output) -> Vec<u8> {
    let mut value = parse_stdout(output);
    if let Some(map) = value.as_object_mut() {
        map.remove("writer_generation");
    }
    serde_json::to_vec(&value).expect("reserialize normalized status")
}

// why: one (exit code, raw bytes) snapshot per outline path; refusal codes
// are part of the relation. WHY area-local: same as `normalized_search_bytes`.
fn snapshot_outline(
    bin: &Path,
    index_s: &str,
    root_s: &str,
    rels: &[&str],
) -> Vec<(Option<i32>, Vec<u8>)> {
    rels.iter()
        .map(|rel| {
            let output = run_outline(bin, index_s, root_s, rel);
            (output.status.code(), output.stdout.clone())
        })
        .collect()
}

// why: mutate the two-file fixture into its canonical "final" tree: alpha
// gains a symbol, beta is deleted, gamma is added. WHY area-local:
// parity-fixture-specific — single-suite helper.
fn mutate_to_final_tree(root: &Path) {
    rewrite_with_mtime_bump(
        &root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\npub fn alpha_two() -> u32 { 2 }\n",
    );
    fs::remove_file(root.join("beta.rs")).expect("delete beta");
    fs::write(root.join("gamma.rs"), "pub fn gamma_new() -> u32 { 3 }\n").expect("add gamma");
}

/// Fixed per-step mtimes for order-independence convergence.
/// `worktree_revision` is `MAX(mtime_secs)` over stored rows, and a refresh
/// rewrites a row only when its content changed — mtime-only touches never
/// update the recorded value. Both orders must therefore stamp every content
/// write to the same fixed instant: distinct steps get distinct instants so
/// the mtime fast path still fires, and both orders record identical mtimes.
const MTIME_V0: u64 = 1_900_000_000;
const MTIME_ADDED: u64 = 1_900_000_001;
const MTIME_MODIFIED: u64 = 1_900_000_002;

// why: stamp the freshly created v0 fixture so its recorded mtimes are fixed
// (uses testkit `set_mtime_secs` for the stamp itself).
fn stamp_v0(root: &Path) {
    set_mtime_secs(&root.join("alpha.rs"), MTIME_V0);
    set_mtime_secs(&root.join("beta.rs"), MTIME_V0);
}

// why: reset the fixture root to its v0 bytes at the fixed v0 instant, and
// drop the index so the next order starts clean. WHY area-local:
// order-harness-specific (stamp itself reuses testkit `set_mtime_secs`) —
// single-suite helper.
fn reset_to_v0(root: &Path, index: &Path) {
    fs::write(root.join("alpha.rs"), "pub fn alpha_one() -> u32 { 1 }\n").expect("restore alpha");
    set_mtime_secs(&root.join("alpha.rs"), MTIME_V0);
    fs::write(root.join("beta.rs"), "pub fn beta_one() -> u32 { 2 }\n").expect("restore beta");
    set_mtime_secs(&root.join("beta.rs"), MTIME_V0);
    let gamma = root.join("gamma.rs");
    if gamma.exists() {
        fs::remove_file(&gamma).expect("remove gamma");
    }
    if index.exists() {
        fs::remove_file(index).expect("drop index");
    }
}

// why: order-harness ADD step at its fixed instant. WHY area-local: same as
// `reset_to_v0`.
fn add_gamma(root: &Path) {
    fs::write(root.join("gamma.rs"), "pub fn gamma_new() -> u32 { 3 }\n").expect("add gamma");
    set_mtime_secs(&root.join("gamma.rs"), MTIME_ADDED);
}

// why: order-harness MODIFY step at its fixed instant. WHY area-local: same
// as `reset_to_v0`.
fn modify_alpha(root: &Path) {
    fs::write(
        root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\npub fn alpha_two() -> u32 { 2 }\n",
    )
    .expect("modify alpha");
    set_mtime_secs(&root.join("alpha.rs"), MTIME_MODIFIED);
}

/// INTENT: one targeted-driver matrix — (A) targeted --path on an unchanged
/// file is a counted noop, (B) a targeted --path edit lands in served search+
/// outline output, (C) two targeted refreshes == one full refresh over
/// search+outline bytes.
/// KILLS: targeted-always-rewrite | targeted-flag-ignored |
/// targeted/full-divergence.
/// ABSORBS: `index_path_on_unchanged_file_is_noop_reuse` (I1) +
/// `i2_delta_targeted_path_refresh_serves_edit_without_full_rescan` (I2) +
/// `i3_targeted_path_refreshes_match_full_refresh_bytes` (I3)
/// (MERGE→matrix/targeted-refresh ×3).
#[test]
fn matrix_targeted_refresh() {
    let bin = asgrep_bin();

    // Facet A — noop counts on an unchanged file.
    let (_dir, root, _index, root_s, index_s) = fixture_two_files();
    run_index(&bin, &index_s, &root_s);
    let targeted = run_targeted(&bin, &index_s, &root_s, "alpha.rs");
    assert_eq!(targeted["targeted"], true, "{targeted}");
    assert_eq!(targeted["path_count"], 1, "{targeted}");
    assert_eq!(targeted["stats"]["files_indexed"], 0, "{targeted}");
    assert_eq!(targeted["stats"]["files_skipped"], 1, "{targeted}");
    assert_eq!(targeted["stats"]["files_removed"], 0, "{targeted}");

    // Facet B — the targeted delta lands in served output (same fixture: it
    // is converged after facet A, exactly facet B's precondition).
    rewrite_with_mtime_bump(
        &root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\npub fn alpha_two() -> u32 { 2 }\n",
    );
    let edited = run_targeted(&bin, &index_s, &root_s, "alpha.rs");
    assert_eq!(edited["exit_code"], 0, "{edited}");
    assert_served_only_at(
        &run_search(&bin, &index_s, &root_s, "word:alpha_two"),
        "alpha.rs",
    );
    assert_served_only_at(
        &run_search(&bin, &index_s, &root_s, "word:alpha_one"),
        "alpha.rs",
    );
    assert_served_only_at(
        &run_search(&bin, &index_s, &root_s, "word:beta_one"),
        "beta.rs",
    );
    let outline = run_outline(&bin, &index_s, &root_s, "alpha.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 2, "{outline_value}");

    // Facet C — two targeted refreshes == one full refresh over served bytes.
    let (dir, root, _index, root_s, index_s) = fixture_two_files();
    let full_s = second_index_path(&dir);
    run_index(&bin, &index_s, &root_s);
    run_index(&bin, &full_s, &root_s);
    rewrite_with_mtime_bump(
        &root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\npub fn alpha_two() -> u32 { 2 }\n",
    );
    rewrite_with_mtime_bump(
        &root.join("beta.rs"),
        "pub fn beta_one() -> u32 { 2 }\npub fn beta_two() -> u32 { 4 }\n",
    );
    run_index(&bin, &full_s, &root_s);
    run_targeted(&bin, &index_s, &root_s, "alpha.rs");
    run_targeted(&bin, &index_s, &root_s, "beta.rs");

    let targeted_search = snapshot_search(&bin, &index_s, &root_s);
    let full_search = snapshot_search(&bin, &full_s, &root_s);
    for (query, (left, right)) in PROBE_QUERIES
        .iter()
        .zip(targeted_search.iter().zip(full_search.iter()))
    {
        assert_eq!((left.0, right.0), (Some(0), Some(0)), "{query}");
        assert_eq!(left.1, right.1, "{query}: targeted != full refresh");
    }
    let rels = ["alpha.rs", "beta.rs"];
    let targeted_outline = snapshot_outline(&bin, &index_s, &root_s, &rels);
    let full_outline = snapshot_outline(&bin, &full_s, &root_s, &rels);
    for (rel, (left, right)) in rels
        .iter()
        .zip(targeted_outline.iter().zip(full_outline.iter()))
    {
        assert_eq!((left.0, right.0), (Some(0), Some(0)), "{rel}");
        assert_eq!(left.1, right.1, "{rel}: targeted != full refresh");
    }
}

/// INTENT: incremental refresh (`index` over an existing DB) == clean rebuild
/// (fresh DB over the same final tree) over normalized search bytes per probe
/// query AND outline bytes incl. deleted-path refusal codes.
/// KILLS: incremental-divergence (stale-row | missing-prune) |
/// incremental-outline-divergence.
/// ABSORBS: `i3_incremental_refresh_matches_clean_rebuild_search_bytes` +
/// `i3_incremental_refresh_matches_clean_rebuild_outline_bytes` (I3
/// MERGE→matrix/parity-incremental-vs-clean ×2). The DELETEd sibling counts
/// test is strictly implied by this byte identity.
#[test]
fn matrix_parity_incremental_vs_clean() {
    let bin = asgrep_bin();
    let (dir, root, _index, root_s, index_s) = fixture_two_files();
    let clean_s = second_index_path(&dir);

    // Incremental path: index v0, mutate, refresh.
    run_index(&bin, &index_s, &root_s);
    mutate_to_final_tree(&root);
    run_index(&bin, &index_s, &root_s);

    // Clean path: fresh database indexed once over the final tree.
    run_index(&bin, &clean_s, &root_s);

    let incremental = snapshot_search(&bin, &index_s, &root_s);
    let clean = snapshot_search(&bin, &clean_s, &root_s);
    assert_eq!(incremental.len(), clean.len());
    for (query, (left, right)) in PROBE_QUERIES
        .iter()
        .zip(incremental.iter().zip(clean.iter()))
    {
        assert_eq!(left.0, Some(0), "{query}: incremental search must exit 0");
        assert_eq!(right.0, Some(0), "{query}: clean search must exit 0");
        assert_eq!(left.1, right.1, "{query}: incremental != clean rebuild");
    }

    // Retained + modified + added paths serve bytes; the deleted path refuses
    // identically on both sides (exit code is part of the relation).
    let rels = ["alpha.rs", "gamma.rs", "beta.rs"];
    let incremental_outline = snapshot_outline(&bin, &index_s, &root_s, &rels);
    let clean_outline = snapshot_outline(&bin, &clean_s, &root_s, &rels);
    for (rel, (left, right)) in rels
        .iter()
        .zip(incremental_outline.iter().zip(clean_outline.iter()))
    {
        assert_eq!(left.0, right.0, "{rel}: outline exit codes diverge");
        assert_eq!(left.1, right.1, "{rel}: outline bytes diverge");
    }
    assert_eq!(incremental_outline[0].0, Some(0));
    assert_eq!(incremental_outline[1].0, Some(0));
    assert_eq!(incremental_outline[2].0, Some(2));
}

/// INTENT: delta application order is unobservable — add→modify ==
/// modify→add, and delete→add == add→delete, over search+outline bytes.
/// KILLS: order-dependent-state | order-dependent-prune/add.
/// ABSORBS: `i3_delta_order_add_then_modify_matches_modify_then_add` +
/// `i3_delta_order_delete_then_add_matches_add_then_delete` (I3
/// MERGE→matrix/parity-order-independence ×2).
#[test]
fn matrix_parity_order_independence() {
    let bin = asgrep_bin();

    // Pair 1: add→modify == modify→add.
    {
        let (_dir, root, _index, root_s, index_s) = fixture_two_files();
        let index = PathBuf::from(&index_s);
        stamp_v0(&root);
        run_index(&bin, &index_s, &root_s);
        add_gamma(&root);
        run_index(&bin, &index_s, &root_s);
        modify_alpha(&root);
        run_index(&bin, &index_s, &root_s);
        let order1_search = snapshot_search(&bin, &index_s, &root_s);
        let order1_outline = snapshot_outline(&bin, &index_s, &root_s, &["alpha.rs", "gamma.rs"]);

        reset_to_v0(&root, &index);
        run_index(&bin, &index_s, &root_s);
        modify_alpha(&root);
        run_index(&bin, &index_s, &root_s);
        add_gamma(&root);
        run_index(&bin, &index_s, &root_s);
        let order2_search = snapshot_search(&bin, &index_s, &root_s);
        let order2_outline = snapshot_outline(&bin, &index_s, &root_s, &["alpha.rs", "gamma.rs"]);

        for (query, (left, right)) in PROBE_QUERIES
            .iter()
            .zip(order1_search.iter().zip(order2_search.iter()))
        {
            assert_eq!((left.0, right.0), (Some(0), Some(0)), "{query}");
            assert_eq!(left.1, right.1, "{query}: delta order is observable");
        }
        for (left, right) in order1_outline.iter().zip(order2_outline.iter()) {
            assert_eq!((left.0, right.0), (Some(0), Some(0)));
            assert_eq!(left.1, right.1, "outline: delta order is observable");
        }
    }

    // Pair 2: delete→add == add→delete.
    {
        let (_dir, root, _index, root_s, index_s) = fixture_two_files();
        let index = PathBuf::from(&index_s);
        stamp_v0(&root);
        run_index(&bin, &index_s, &root_s);
        fs::remove_file(root.join("beta.rs")).expect("delete beta");
        run_index(&bin, &index_s, &root_s);
        add_gamma(&root);
        run_index(&bin, &index_s, &root_s);
        let order1_search = snapshot_search(&bin, &index_s, &root_s);
        let order1_outline = snapshot_outline(
            &bin,
            &index_s,
            &root_s,
            &["alpha.rs", "gamma.rs", "beta.rs"],
        );

        reset_to_v0(&root, &index);
        run_index(&bin, &index_s, &root_s);
        add_gamma(&root);
        run_index(&bin, &index_s, &root_s);
        fs::remove_file(root.join("beta.rs")).expect("delete beta");
        run_index(&bin, &index_s, &root_s);
        let order2_search = snapshot_search(&bin, &index_s, &root_s);
        let order2_outline = snapshot_outline(
            &bin,
            &index_s,
            &root_s,
            &["alpha.rs", "gamma.rs", "beta.rs"],
        );

        for (query, (left, right)) in PROBE_QUERIES
            .iter()
            .zip(order1_search.iter().zip(order2_search.iter()))
        {
            assert_eq!((left.0, right.0), (Some(0), Some(0)), "{query}");
            assert_eq!(left.1, right.1, "{query}: delta order is observable");
        }
        for (left, right) in order1_outline.iter().zip(order2_outline.iter()) {
            assert_eq!(left.0, right.0, "outline exit codes diverge by order");
            assert_eq!(left.1, right.1, "outline: delta order is observable");
        }
    }
}

/// INTENT: reindex is idempotent (twice == once over search+outline bytes and
/// status counts) and in-place reindex == clean rebuild over search+outline
/// bytes.
/// KILLS: reindex-non-idempotence | reindex-outline/count-drift |
/// rewrite-path-row-skip.
/// ABSORBS: `i3_reindex_twice_serves_byte_identical_search` +
/// `i3_reindex_twice_outline_and_status_counts_stable` +
/// `i3_reindex_matches_clean_rebuild_bytes` (I3 MERGE→matrix/parity-reindex ×3).
#[test]
fn matrix_parity_reindex() {
    let bin = asgrep_bin();

    // Idempotence: one setup, search+outline+status snapshotted after each of
    // two reindexes — every assertion of both I3 idempotence tests.
    {
        let (_dir, root, _index, root_s, index_s) = fixture_two_files();
        run_index(&bin, &index_s, &root_s);
        mutate_to_final_tree(&root);

        run_reindex(&bin, &index_s, &root_s);
        let search_once = snapshot_search(&bin, &index_s, &root_s);
        let rels = ["alpha.rs", "gamma.rs", "beta.rs"];
        let outline_once = snapshot_outline(&bin, &index_s, &root_s, &rels);
        let status_once = assert_success(&run_status_raw(&bin, &index_s, &root_s), "status");

        run_reindex(&bin, &index_s, &root_s);
        let search_twice = snapshot_search(&bin, &index_s, &root_s);
        let outline_twice = snapshot_outline(&bin, &index_s, &root_s, &rels);
        let status_twice = assert_success(&run_status_raw(&bin, &index_s, &root_s), "status");

        for (query, (left, right)) in PROBE_QUERIES
            .iter()
            .zip(search_once.iter().zip(search_twice.iter()))
        {
            assert_eq!((left.0, right.0), (Some(0), Some(0)), "{query}");
            assert_eq!(left.1, right.1, "{query}: reindex is not idempotent");
        }
        for (rel, (left, right)) in rels
            .iter()
            .zip(outline_once.iter().zip(outline_twice.iter()))
        {
            assert_eq!(left.0, right.0, "{rel}: outline exit codes diverge");
            assert_eq!(left.1, right.1, "{rel}: reindex is not idempotent");
        }
        assert_eq!(
            status_once["file_count"], status_twice["file_count"],
            "{status_once} vs {status_twice}"
        );
        assert_eq!(
            status_once["symbol_count"], status_twice["symbol_count"],
            "{status_once} vs {status_twice}"
        );
    }

    // Reindex == clean rebuild over search+outline bytes.
    {
        let (dir, root, _index, root_s, index_s) = fixture_two_files();
        let clean_s = second_index_path(&dir);
        run_index(&bin, &index_s, &root_s);
        mutate_to_final_tree(&root);
        run_reindex(&bin, &index_s, &root_s);
        run_index(&bin, &clean_s, &root_s);

        let reindexed = snapshot_search(&bin, &index_s, &root_s);
        let clean = snapshot_search(&bin, &clean_s, &root_s);
        for (query, (left, right)) in PROBE_QUERIES.iter().zip(reindexed.iter().zip(clean.iter())) {
            assert_eq!((left.0, right.0), (Some(0), Some(0)), "{query}");
            assert_eq!(left.1, right.1, "{query}: reindex != clean rebuild");
        }

        let rels = ["alpha.rs", "gamma.rs"];
        let reindexed_outline = snapshot_outline(&bin, &index_s, &root_s, &rels);
        let clean_outline = snapshot_outline(&bin, &clean_s, &root_s, &rels);
        for (rel, (left, right)) in rels
            .iter()
            .zip(reindexed_outline.iter().zip(clean_outline.iter()))
        {
            assert_eq!((left.0, right.0), (Some(0), Some(0)), "{rel}");
            assert_eq!(left.1, right.1, "{rel}: reindex != clean rebuild");
        }
    }
}

/// INTENT: post-convergence refresh is a counted noop with zero
/// search/outline/status byte drift.
/// KILLS: converged-refresh-mutation.
/// ABSORBS: none — KEEP of `i3_converged_refresh_changes_no_observable_bytes` (I3).
#[test]
fn i3_converged_refresh_changes_no_observable_bytes() {
    let bin = asgrep_bin();
    let (_dir, root, _index, root_s, index_s) = fixture_two_files();

    run_index(&bin, &index_s, &root_s);
    mutate_to_final_tree(&root);
    run_index(&bin, &index_s, &root_s);

    let search_before = snapshot_search(&bin, &index_s, &root_s);
    let outline_before = snapshot_outline(&bin, &index_s, &root_s, &["alpha.rs", "gamma.rs"]);
    let status_before = run_status_raw(&bin, &index_s, &root_s);

    // The converged refresh must be a reuse no-op ...
    let converged = run_index(&bin, &index_s, &root_s);
    assert_eq!(converged["files_indexed"], 0, "{converged}");
    assert_eq!(converged["files_removed"], 0, "{converged}");

    // ... and no observable byte may change across it.
    let search_after = snapshot_search(&bin, &index_s, &root_s);
    let outline_after = snapshot_outline(&bin, &index_s, &root_s, &["alpha.rs", "gamma.rs"]);
    let status_after = run_status_raw(&bin, &index_s, &root_s);
    assert_eq!(status_before.status.code(), Some(0));
    assert_eq!(status_after.status.code(), Some(0));
    assert_eq!(
        normalized_status_bytes(&status_before),
        normalized_status_bytes(&status_after),
        "status drifted"
    );

    for (query, (left, right)) in PROBE_QUERIES
        .iter()
        .zip(search_before.iter().zip(search_after.iter()))
    {
        assert_eq!(left.1, right.1, "{query}: served bytes drifted");
    }
    for (left, right) in outline_before.iter().zip(outline_after.iter()) {
        assert_eq!(left.1, right.1, "outline bytes drifted");
    }
}

//! CLI invalidation: end-to-end change→detect→reindex→serve drills + watch gates.
//!
//! Canonical successor of `invalidation_pass4.rs` (I4) per
//! `tests/catalog/invalidation-cli.md`, plus the two I1 watch-gate KEEPs
//! (kept next to the watch-mode drill they gate). Per change kind the full
//! lifecycle is pinned via the real binary: indexed tree SERVING search
//! (baseline proved) → real filesystem change → DETECT staleness (status
//! counts still pre-change AND served outputs still pre-change, read with
//! `--no-auto-index`) → explicit `reindex` → SERVE proves the exact new
//! outputs.
//!
//! Absorption map: the 5 single-kind drills fuse into the parameterized
//! `matrix_drill_lifecycle`; `i4_drill_chained…` and
//! `i4_drill_watch_mode…` are KEEPs, as are the I1
//! `watch_serves_only…` / `watch_restart_resumes…` gates.

#[path = "invalidation_common.rs"]
mod common;

use ast_sgrep_testkit::{asgrep_bin, assert_success, parse_stdout, run};
use common::*;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

// why: row-scoped drill context owning the TempDir lifetime.
// WHY area-local: drill-specific fixture context — single-suite type.
struct DrillCtx {
    _dir: tempfile::TempDir,
    bin: PathBuf,
    root: PathBuf,
    root_s: String,
    index_s: String,
}

impl DrillCtx {
    fn fresh() -> Self {
        let bin = asgrep_bin();
        let (_dir, root, _index, root_s, index_s) = fixture_two_files();
        Self {
            _dir,
            bin,
            root,
            root_s,
            index_s,
        }
    }
}

// why: the five single-kind lifecycles the matrix drill parameterizes over.
// WHY area-local: drill-specific — single-suite type.
#[derive(Clone, Copy)]
enum DrillKind {
    ModifyAdd,
    AddFile,
    DeleteFile,
    RenameFile,
    ModifyRemove,
}

impl DrillKind {
    const ALL: &[DrillKind] = &[
        DrillKind::ModifyAdd,
        DrillKind::AddFile,
        DrillKind::DeleteFile,
        DrillKind::RenameFile,
        DrillKind::ModifyRemove,
    ];
}

// why: MODIFY-add lifecycle: baseline→stale-invisible→reindex→exact serve.
// WHY area-local: drill-specific phase script — single-suite helper.
fn drill_modify_add(ctx: &DrillCtx) {
    // SERVE (baseline): the indexed tree serves the seed symbols.
    run_index(&ctx.bin, &ctx.index_s, &ctx.root_s);
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_one"),
        "alpha.rs",
    );
    let baseline = run_status(&ctx.bin, &ctx.index_s, &ctx.root_s);
    assert_eq!(baseline["file_count"], 2, "{baseline}");

    // CHANGE: alpha gains a second symbol.
    rewrite_with_mtime_bump(
        &ctx.root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\npub fn i4_modify_added() -> u32 { 4 }\n",
    );

    // DETECT (stale): status counts are still pre-change and the new symbol
    // is not served, while the old hit set is intact.
    let stale = run_status(&ctx.bin, &ctx.index_s, &ctx.root_s);
    assert_eq!(stale["file_count"], 2, "{stale}");
    assert_eq!(
        stale["symbol_count"], baseline["symbol_count"],
        "{stale} vs {baseline}"
    );
    assert_served_nowhere(&run_search(
        &ctx.bin,
        &ctx.index_s,
        &ctx.root_s,
        "word:i4_modify_added",
    ));
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_one"),
        "alpha.rs",
    );

    // REINDEX → SERVE: the exact new outputs are served.
    let rebuilt = run_reindex(&ctx.bin, &ctx.index_s, &ctx.root_s);
    assert_eq!(rebuilt["files_indexed"], 2, "{rebuilt}");
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:i4_modify_added"),
        "alpha.rs",
    );
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_one"),
        "alpha.rs",
    );
    let outline = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "alpha.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 2, "{outline_value}");
    assert_eq!(
        outline_names(&outline_value),
        vec!["alpha_one", "i4_modify_added"],
        "{outline_value}"
    );
    let converged = run_status(&ctx.bin, &ctx.index_s, &ctx.root_s);
    assert_eq!(
        converged["symbol_count"].as_u64().expect("symbol count"),
        baseline["symbol_count"].as_u64().expect("baseline count") + 1,
        "{converged} vs {baseline}"
    );
}

// why: ADD lifecycle incl. stale outline-refusal of the unindexed path.
// WHY area-local: drill-specific phase script — single-suite helper.
fn drill_add_file(ctx: &DrillCtx) {
    // SERVE (baseline).
    run_index(&ctx.bin, &ctx.index_s, &ctx.root_s);
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:beta_one"),
        "beta.rs",
    );
    assert_eq!(
        run_status(&ctx.bin, &ctx.index_s, &ctx.root_s)["file_count"],
        2
    );

    // CHANGE: a third file appears.
    fs::write(
        ctx.root.join("gamma.rs"),
        "pub fn i4_add_gamma() -> u32 { 3 }\n",
    )
    .expect("add gamma");

    // DETECT (stale): status still reports the old file count, the new
    // symbol is served nowhere, and outline refuses the unindexed path.
    assert_eq!(
        run_status(&ctx.bin, &ctx.index_s, &ctx.root_s)["file_count"],
        2
    );
    assert_served_nowhere(&run_search(
        &ctx.bin,
        &ctx.index_s,
        &ctx.root_s,
        "word:i4_add_gamma",
    ));
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_one"),
        "alpha.rs",
    );
    let stale_outline = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "gamma.rs");
    assert_eq!(
        stale_outline.status.code(),
        Some(2),
        "unindexed path must refuse"
    );
    assert_eq!(parse_stdout(&stale_outline)["ok"], false);

    // REINDEX → SERVE: the added file is served exactly.
    let rebuilt = run_reindex(&ctx.bin, &ctx.index_s, &ctx.root_s);
    assert_eq!(rebuilt["files_indexed"], 3, "{rebuilt}");
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:i4_add_gamma"),
        "gamma.rs",
    );
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_one"),
        "alpha.rs",
    );
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:beta_one"),
        "beta.rs",
    );
    let outline = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "gamma.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 1, "{outline_value}");
    assert_eq!(
        outline_names(&outline_value),
        vec!["i4_add_gamma"],
        "{outline_value}"
    );
    assert_eq!(
        run_status(&ctx.bin, &ctx.index_s, &ctx.root_s)["file_count"],
        3
    );
}

// why: DELETE lifecycle incl. stale-still-served no-implicit-refresh proof.
// WHY area-local: drill-specific phase script — single-suite helper.
fn drill_delete_file(ctx: &DrillCtx) {
    // SERVE (baseline).
    run_index(&ctx.bin, &ctx.index_s, &ctx.root_s);
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:beta_one"),
        "beta.rs",
    );
    let baseline = run_status(&ctx.bin, &ctx.index_s, &ctx.root_s);
    assert_eq!(baseline["file_count"], 2, "{baseline}");

    // CHANGE: beta is deleted from the tree.
    fs::remove_file(ctx.root.join("beta.rs")).expect("delete beta");

    // DETECT (stale): status counts are still pre-change and the deleted
    // symbol is STILL served — no implicit refresh happened.
    let stale = run_status(&ctx.bin, &ctx.index_s, &ctx.root_s);
    assert_eq!(stale["file_count"], 2, "{stale}");
    assert_eq!(
        stale["symbol_count"], baseline["symbol_count"],
        "{stale} vs {baseline}"
    );
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:beta_one"),
        "beta.rs",
    );

    // REINDEX → SERVE: the deleted symbol is gone everywhere, the survivor
    // is intact.
    let rebuilt = run_reindex(&ctx.bin, &ctx.index_s, &ctx.root_s);
    assert_eq!(rebuilt["files_indexed"], 1, "{rebuilt}");
    assert_served_nowhere(&run_search(
        &ctx.bin,
        &ctx.index_s,
        &ctx.root_s,
        "word:beta_one",
    ));
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_one"),
        "alpha.rs",
    );
    let gone = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "beta.rs");
    assert_eq!(gone.status.code(), Some(2), "deleted path must refuse");
    assert_eq!(parse_stdout(&gone)["ok"], false);
    let kept = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "alpha.rs");
    assert_eq!(kept.status.code(), Some(0));
    let converged = run_status(&ctx.bin, &ctx.index_s, &ctx.root_s);
    assert_eq!(converged["file_count"], 1, "{converged}");
    assert_eq!(
        converged["symbol_count"].as_u64().expect("symbol count"),
        baseline["symbol_count"].as_u64().expect("baseline count") - 1,
        "{converged} vs {baseline}"
    );
}

// why: RENAME lifecycle: stale old-path-only → new-path-only after reindex.
// WHY area-local: drill-specific phase script — single-suite helper.
fn drill_rename_file(ctx: &DrillCtx) {
    // SERVE (baseline).
    run_index(&ctx.bin, &ctx.index_s, &ctx.root_s);
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:beta_one"),
        "beta.rs",
    );
    assert_eq!(
        run_status(&ctx.bin, &ctx.index_s, &ctx.root_s)["file_count"],
        2
    );

    // CHANGE: beta moves to a new path with identical content.
    fs::rename(ctx.root.join("beta.rs"), ctx.root.join("beta_moved.rs")).expect("rename beta");

    // DETECT (stale): the hit is still served at the OLD path only.
    assert_eq!(
        run_status(&ctx.bin, &ctx.index_s, &ctx.root_s)["file_count"],
        2
    );
    let stale = run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:beta_one");
    assert_served_only_at(&stale, "beta.rs");
    assert_eq!(
        hits_in(&stale, "beta_moved.rs"),
        0,
        "unrefreshed new path: {stale}"
    );

    // REINDEX → SERVE: the hit follows the new path exactly.
    let rebuilt = run_reindex(&ctx.bin, &ctx.index_s, &ctx.root_s);
    assert_eq!(rebuilt["files_indexed"], 2, "{rebuilt}");
    let moved = run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:beta_one");
    assert_served_only_at(&moved, "beta_moved.rs");
    assert_eq!(hits_in(&moved, "beta.rs"), 0, "stale old-path hit: {moved}");
    let stale_outline = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "beta.rs");
    assert_eq!(stale_outline.status.code(), Some(2), "old path must refuse");
    assert_eq!(parse_stdout(&stale_outline)["ok"], false);
    let outline = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "beta_moved.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 1, "{outline_value}");
    assert_eq!(
        outline_names(&outline_value),
        vec!["beta_one"],
        "{outline_value}"
    );
    assert_eq!(
        run_status(&ctx.bin, &ctx.index_s, &ctx.root_s)["file_count"],
        2
    );
}

// why: MODIFY-remove lifecycle: stale old-served/new-absent → swapped after
// reindex. WHY area-local: drill-specific phase script — single-suite helper.
fn drill_modify_remove(ctx: &DrillCtx) {
    // SERVE (baseline).
    run_index(&ctx.bin, &ctx.index_s, &ctx.root_s);
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_one"),
        "alpha.rs",
    );
    let baseline = run_status(&ctx.bin, &ctx.index_s, &ctx.root_s);
    assert_eq!(baseline["file_count"], 2, "{baseline}");

    // CHANGE: alpha_one is removed; an unrelated symbol takes its place.
    rewrite_with_mtime_bump(
        &ctx.root.join("alpha.rs"),
        "pub fn i4_modify_replacement() -> u32 { 5 }\n",
    );

    // DETECT (stale): the removed symbol is STILL served and the replacement
    // is served nowhere; status counts are still pre-change.
    let stale = run_status(&ctx.bin, &ctx.index_s, &ctx.root_s);
    assert_eq!(stale["file_count"], 2, "{stale}");
    assert_eq!(
        stale["symbol_count"], baseline["symbol_count"],
        "{stale} vs {baseline}"
    );
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_one"),
        "alpha.rs",
    );
    assert_served_nowhere(&run_search(
        &ctx.bin,
        &ctx.index_s,
        &ctx.root_s,
        "word:i4_modify_replacement",
    ));

    // REINDEX → SERVE: old token gone, new token served, outline exact.
    let rebuilt = run_reindex(&ctx.bin, &ctx.index_s, &ctx.root_s);
    assert_eq!(rebuilt["files_indexed"], 2, "{rebuilt}");
    assert_served_nowhere(&run_search(
        &ctx.bin,
        &ctx.index_s,
        &ctx.root_s,
        "word:alpha_one",
    ));
    assert_served_only_at(
        &run_search(
            &ctx.bin,
            &ctx.index_s,
            &ctx.root_s,
            "word:i4_modify_replacement",
        ),
        "alpha.rs",
    );
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:beta_one"),
        "beta.rs",
    );
    let outline = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "alpha.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 1, "{outline_value}");
    assert_eq!(
        outline_names(&outline_value),
        vec!["i4_modify_replacement"],
        "{outline_value}"
    );
    assert_eq!(
        run_status(&ctx.bin, &ctx.index_s, &ctx.root_s)["file_count"],
        2
    );
}

// why: single dispatch over the five drill rows so the matrix test cannot
// silently drop a kind. WHY area-local: drill-specific — single-suite helper.
fn run_drill(kind: DrillKind, ctx: &DrillCtx) {
    match kind {
        DrillKind::ModifyAdd => drill_modify_add(ctx),
        DrillKind::AddFile => drill_add_file(ctx),
        DrillKind::DeleteFile => drill_delete_file(ctx),
        DrillKind::RenameFile => drill_rename_file(ctx),
        DrillKind::ModifyRemove => drill_modify_remove(ctx),
    }
}

// --- live-watch helpers (single copy; was duplicated in I1/I4) ---

// why: spawned `watch` daemon (50ms debounce, null stdio) reaped on drop.
// WHY area-local: testkit `KillOnDrop` is kill-oriented with no watch-spawn
// shape or status-gated servability poll; only this suite spawns watch —
// single-suite type.
struct WatchChild {
    child: Child,
}

impl WatchChild {
    fn spawn(bin: &Path, root: &str, index: &str) -> Self {
        let child = Command::new(bin)
            .args([
                "--no-embed",
                "--index-path",
                index,
                "watch",
                "--debounce-ms",
                "50",
                root,
            ])
            .env("NO_COLOR", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn asgrep watch");
        Self { child }
    }
}

impl Drop for WatchChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// why: resume gate — the index is servable once `status` reports the full
// file count. Polling the CLI discriminant keeps these oracles independent of
// watch's human-readable log lines. WHY area-local: same as `WatchChild`.
fn wait_until_servable(bin: &Path, index_s: &str, root_s: &str, files: u64) {
    let started = Instant::now();
    let timeout = Duration::from_secs(30);
    loop {
        let output = run(
            bin,
            &[
                "--json",
                "--no-embed",
                "--index-path",
                index_s,
                "status",
                root_s,
            ],
        );
        if output.status.code() == Some(0) {
            if let Ok(value) = serde_json::from_slice::<Value>(&output.stdout) {
                if value["file_count"] == files {
                    return;
                }
            }
        }
        assert!(
            started.elapsed() < timeout,
            "watch never served {files} files within {timeout:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

// why: keyword-face served read for the resume gates. WHY area-local: no
// keyword runner exists in testkit, and only this suite reads the keyword face
// for resume gates — single-suite helper.
fn keyword_hits(bin: &Path, index_s: &str, root_s: &str, query: &str) -> Value {
    assert_success(
        &run(
            bin,
            &[
                "--json",
                "--no-embed",
                "--index-path",
                index_s,
                "keyword",
                query,
                root_s,
            ],
        ),
        "keyword",
    )
}

// why: poll served search until `predicate` holds or the timeout expires.
// Returns the last observed value; the caller asserts exact end-state counts
// on it. WHY area-local: daemon-convergence poll, specific to the watch drill
// — single-suite helper.
fn poll_search_until(
    bin: &Path,
    index_s: &str,
    root_s: &str,
    query: &str,
    timeout: Duration,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
    let started = Instant::now();
    loop {
        let value = run_search(bin, index_s, root_s, query);
        if predicate(&value) {
            return value;
        }
        assert!(
            started.elapsed() < timeout,
            "watch never converged for {query} within {timeout:?}; last: {value}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// INTENT: one parameterized drill — for each of the 5 delta kinds
/// (MODIFY-add, ADD, DELETE, RENAME, MODIFY-remove): baseline serve proved →
/// real change → stale middle (pre-change status AND pre-change served
/// outputs, proving no implicit refresh) → explicit reindex → exact new
/// outputs served.
/// KILLS: implicit-refresh | reindex-non-convergence |
/// prune-non-convergence | rename-non-convergence | swap-non-convergence.
/// ABSORBS: `i4_drill_modify_add_symbol_full_cycle` +
/// `i4_drill_add_file_full_cycle` + `i4_drill_delete_file_full_cycle` +
/// `i4_drill_rename_file_full_cycle` + `i4_drill_modify_remove_symbol_full_cycle`
/// (I4 MERGE→matrix/drill-lifecycle ×5).
#[test]
fn matrix_drill_lifecycle() {
    assert_eq!(
        DrillKind::ALL.len(),
        5,
        "the I4 single-kind set is 5 drills"
    );
    for kind in DrillKind::ALL {
        let ctx = DrillCtx::fresh();
        run_drill(*kind, &ctx);
    }
}

/// INTENT: chained modify+delete+add+rename converges exactly in one reindex,
/// with every observable still pre-change until the refresh.
/// KILLS: chained-delta-interference (rename-loses-edit, tombstone-blocks-add).
/// ABSORBS: none — KEEP of `i4_drill_chained_add_modify_delete_rename_single_reindex` (I4).
#[test]
fn i4_drill_chained_add_modify_delete_rename_single_reindex() {
    let bin = asgrep_bin();
    let (_dir, root, _index, root_s, index_s) = fixture_two_files();

    // SERVE (baseline): both seeds served.
    run_index(&bin, &index_s, &root_s);
    assert_served_only_at(
        &run_search(&bin, &index_s, &root_s, "word:alpha_one"),
        "alpha.rs",
    );
    assert_served_only_at(
        &run_search(&bin, &index_s, &root_s, "word:beta_one"),
        "beta.rs",
    );
    assert_eq!(run_status(&bin, &index_s, &root_s)["file_count"], 2);

    // CHANGE (chained): modify alpha, delete beta, add gamma, rename alpha —
    // all before any refresh.
    rewrite_with_mtime_bump(
        &root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\npub fn i4_chain_added() -> u32 { 6 }\n",
    );
    fs::remove_file(root.join("beta.rs")).expect("delete beta");
    fs::write(
        root.join("gamma.rs"),
        "pub fn i4_chain_gamma() -> u32 { 7 }\n",
    )
    .expect("add gamma");
    fs::rename(root.join("alpha.rs"), root.join("alpha_moved.rs")).expect("rename alpha");

    // DETECT (stale): every observable is still the pre-change world.
    assert_eq!(run_status(&bin, &index_s, &root_s)["file_count"], 2);
    assert_served_only_at(
        &run_search(&bin, &index_s, &root_s, "word:alpha_one"),
        "alpha.rs",
    );
    assert_served_only_at(
        &run_search(&bin, &index_s, &root_s, "word:beta_one"),
        "beta.rs",
    );
    assert_served_nowhere(&run_search(&bin, &index_s, &root_s, "word:i4_chain_added"));
    assert_served_nowhere(&run_search(&bin, &index_s, &root_s, "word:i4_chain_gamma"));

    // One REINDEX converges the whole chain → SERVE proves every exact end.
    let rebuilt = run_reindex(&bin, &index_s, &root_s);
    assert_eq!(rebuilt["files_indexed"], 2, "{rebuilt}");
    let one = run_search(&bin, &index_s, &root_s, "word:alpha_one");
    assert_served_only_at(&one, "alpha_moved.rs");
    assert_eq!(hits_in(&one, "alpha.rs"), 0, "stale old-path hit: {one}");
    assert_served_only_at(
        &run_search(&bin, &index_s, &root_s, "word:i4_chain_added"),
        "alpha_moved.rs",
    );
    assert_served_nowhere(&run_search(&bin, &index_s, &root_s, "word:beta_one"));
    assert_served_only_at(
        &run_search(&bin, &index_s, &root_s, "word:i4_chain_gamma"),
        "gamma.rs",
    );

    let moved_outline = run_outline(&bin, &index_s, &root_s, "alpha_moved.rs");
    assert_eq!(moved_outline.status.code(), Some(0));
    let moved_value = parse_stdout(&moved_outline);
    assert_eq!(moved_value["count"], 2, "{moved_value}");
    assert_eq!(
        outline_names(&moved_value),
        vec!["alpha_one", "i4_chain_added"],
        "{moved_value}"
    );
    let gamma_outline = run_outline(&bin, &index_s, &root_s, "gamma.rs");
    assert_eq!(gamma_outline.status.code(), Some(0));
    assert_eq!(parse_stdout(&gamma_outline)["count"], 1);
    for rel in ["alpha.rs", "beta.rs"] {
        let stale = run_outline(&bin, &index_s, &root_s, rel);
        assert_eq!(stale.status.code(), Some(2), "{rel} must refuse");
        assert_eq!(parse_stdout(&stale)["ok"], false);
    }
    let converged = run_status(&bin, &index_s, &root_s);
    assert_eq!(converged["file_count"], 2, "{converged}");
    assert_eq!(converged["symbol_count"], 3, "{converged}");
}

/// INTENT: live watch daemon converges MODIFY+DELETE with zero manual refresh
/// invocations (no `index`/`reindex` anywhere in this drill).
/// KILLS: watch-no-detect | no-serve-without-manual.
/// ABSORBS: none — KEEP of
/// `i4_drill_watch_mode_serves_modify_and_delete_without_manual_reindex` (I4).
#[test]
fn i4_drill_watch_mode_serves_modify_and_delete_without_manual_reindex() {
    let bin = asgrep_bin();
    let (_dir, root, _index, root_s, index_s) = fixture_two_files();
    // No `index`/`reindex` invocation anywhere in this drill: the watch
    // daemon performs the initial index and every incremental refresh.
    let _watch = WatchChild::spawn(&bin, &root_s, &index_s);
    wait_until_servable(&bin, &index_s, &root_s, 2);

    // SERVE (baseline): the daemon's initial index serves the seeds.
    assert_served_only_at(
        &run_search(&bin, &index_s, &root_s, "word:alpha_one"),
        "alpha.rs",
    );
    assert_served_only_at(
        &run_search(&bin, &index_s, &root_s, "word:beta_one"),
        "beta.rs",
    );

    // CHANGE: modify alpha. The daemon must detect and serve the new symbol
    // with no manual refresh.
    rewrite_with_mtime_bump(
        &root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\npub fn i4_watch_token() -> u32 { 8 }\n",
    );
    let modified = poll_search_until(
        &bin,
        &index_s,
        &root_s,
        "word:i4_watch_token",
        Duration::from_secs(30),
        |value| total_hits(value) == 1,
    );
    assert_served_only_at(&modified, "alpha.rs");
    assert_served_only_at(
        &run_search(&bin, &index_s, &root_s, "word:alpha_one"),
        "alpha.rs",
    );

    // CHANGE: delete beta. The daemon must converge to the pruned hit set.
    fs::remove_file(root.join("beta.rs")).expect("delete beta");
    let pruned = poll_search_until(
        &bin,
        &index_s,
        &root_s,
        "word:beta_one",
        Duration::from_secs(30),
        |value| total_hits(value) == 0,
    );
    assert_served_nowhere(&pruned);
    assert_served_only_at(
        &run_search(&bin, &index_s, &root_s, "word:alpha_one"),
        "alpha.rs",
    );

    // Outline converges too: the deleted path refuses, the survivor serves.
    let started = Instant::now();
    let timeout = Duration::from_secs(30);
    loop {
        let gone = run_outline(&bin, &index_s, &root_s, "beta.rs");
        if gone.status.code() == Some(2) {
            assert_eq!(parse_stdout(&gone)["ok"], false);
            break;
        }
        assert!(
            started.elapsed() < timeout,
            "watch never pruned outline for beta.rs within {timeout:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let kept = run_outline(&bin, &index_s, &root_s, "alpha.rs");
    assert_eq!(kept.status.code(), Some(0));
    assert_eq!(parse_stdout(&kept)["count"], 2);
}

/// INTENT: watch first-boot is servable only after the full initial index
/// (status-gated, keyword-served).
/// KILLS: serve-before-ready (gate-omission).
/// ABSORBS: none — KEEP of `watch_serves_only_after_initial_index_completes` (I1).
#[test]
fn watch_serves_only_after_initial_index_completes() {
    let bin = asgrep_bin();
    let (_dir, _root, _index, root_s, index_s) = fixture_two_files();
    let _watch = WatchChild::spawn(&bin, &root_s, &index_s);

    wait_until_servable(&bin, &index_s, &root_s, 2);
    let found = keyword_hits(&bin, &index_s, &root_s, "alpha_one");
    let hits = found["hits"].as_array().expect("hits array");
    assert!(!hits.is_empty(), "seed must be served after gate: {found}");
}

/// INTENT: watch restart over an existing DB resumes to a servable index
/// across two restarts.
/// KILLS: resume-regression.
/// ABSORBS: none — KEEP of `watch_restart_resumes_to_servable_index` (I1).
#[test]
fn watch_restart_resumes_to_servable_index() {
    let bin = asgrep_bin();
    let (_dir, _root, _index, root_s, index_s) = fixture_two_files();
    run_index(&bin, &index_s, &root_s);

    for _ in 0..2 {
        let _watch = WatchChild::spawn(&bin, &root_s, &index_s);
        wait_until_servable(&bin, &index_s, &root_s, 2);
        let found = keyword_hits(&bin, &index_s, &root_s, "beta_one");
        let hits = found["hits"].as_array().expect("hits array");
        assert!(!hits.is_empty(), "seed must survive watch resume: {found}");
    }

    let status = run_status(&bin, &index_s, &root_s);
    assert_eq!(status["file_count"], 2, "{status}");
}

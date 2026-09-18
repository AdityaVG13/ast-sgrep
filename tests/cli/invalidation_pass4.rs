//! I4 end-to-end watch drills: FULL change→detect→reindex→serve lifecycles.
//!
//! Prior art this file does NOT duplicate:
//! - `invalidation_pass1` (I1) — freshness reuse/rebuild COUNTS, `reindex`
//!   full-rewrite selection, targeted-unchanged noop counts, status freshness
//!   discriminants, `--auto-index` flag precedence, the `mutated()` reopen
//!   predicate, watch resume gates (initial index only, no change→serve),
//!   `--dry-run`.
//! - `invalidation_pass2` (I2) — per-DELTA served-output oracles after one
//!   `index` refresh. I2 never asserts the stale middle state and never
//!   drives the refresh via `reindex`.
//! - `invalidation_pass3` (I3) — RELATIONS between refresh paths
//!   (incremental==clean, targeted==full, order-independence, reindex
//!   idempotence, convergence stability). I3 never asserts a stale middle
//!   state and never runs a baseline-serve→change→stale→refresh→serve drill.
//! - `watch_incremental` — library `update_paths` semantics via the store.
//! - `watch_daemon_e2e` — live `watch` CREATE via stderr log markers and a
//!   keyword mention check. I4 asserts no log text and drills MODIFY+DELETE
//!   via exact search hit counts.
//! - `machine_contracts`, `outline_cmd`, `cli_smoke` — shapes, counts, and
//!   refusal defaults, no full drills.
//!
//! I4 pins, per change kind, the complete lifecycle via the real binary:
//! indexed tree SERVING search (baseline proved) → apply a real filesystem
//! change → DETECT staleness (status counts still pre-change AND served
//! outputs still pre-change, with `--no-auto-index` so the read is exactly
//! what the last refresh wrote) → explicit `reindex` → SERVE proves the
//! exact new outputs (hit sets per path, hit counts, outline symbol sets,
//! status counts). Plus one chained multi-change drill converging in a
//! single `reindex`, and one live `watch`-mode drill (drivable: bounded
//! polls, no log-text assertions) proving MODIFY+DELETE converge with no
//! manual refresh invocation at all.
//!
//! Every assertion is on exit codes, output counts/sets, and status counts —
//! never message text.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

fn asgrep_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_asgrep"))
}

fn run(bin: &Path, args: &[&str]) -> Output {
    Command::new(bin)
        .args(args)
        .env("NO_COLOR", "1")
        .output()
        .expect("run asgrep")
}

fn parse_stdout(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not JSON: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn assert_success(output: &Output, command: &str) -> Value {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{command} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = parse_stdout(output);
    assert_eq!(value["ok"], true, "{command}: {value}");
    assert_eq!(value["command"], command, "{command}: {value}");
    value
}

fn fixture_two_files() -> (tempfile::TempDir, PathBuf, String, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("proj");
    fs::create_dir_all(&root).expect("proj");
    fs::write(
        root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\n",
    )
    .expect("alpha");
    fs::write(root.join("beta.rs"), "pub fn beta_one() -> u32 { 2 }\n").expect("beta");
    let index = dir.path().join("idx").join("index.db");
    let root_s = root.to_str().expect("root utf8").to_owned();
    let index_s = index.to_str().expect("index utf8").to_owned();
    (dir, root, root_s, index_s)
}

fn run_index(bin: &Path, index_s: &str, root_s: &str) -> Value {
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
            ],
        ),
        "index",
    )
}

fn run_reindex(bin: &Path, index_s: &str, root_s: &str) -> Value {
    assert_success(
        &run(
            bin,
            &[
                "--json",
                "--no-embed",
                "--index-path",
                index_s,
                "reindex",
                root_s,
            ],
        ),
        "reindex",
    )
}

fn run_status(bin: &Path, index_s: &str, root_s: &str) -> Value {
    assert_success(
        &run(
            bin,
            &[
                "--json",
                "--no-embed",
                "--index-path",
                index_s,
                "status",
                root_s,
            ],
        ),
        "status",
    )
}

/// Deterministic served-output read: `--no-auto-index` so the hit set is
/// exactly what the last explicit refresh wrote — no implicit refresh.
fn run_search(bin: &Path, index_s: &str, root_s: &str, query: &str) -> Value {
    assert_success(
        &run(
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
        ),
        "search",
    )
}

fn run_outline(bin: &Path, index_s: &str, root_s: &str, rel: &str) -> Output {
    run(
        bin,
        &[
            "--index-path",
            index_s,
            "--no-auto-index",
            "--root",
            root_s,
            "outline",
            rel,
            "--json",
        ],
    )
}

fn hits_in(value: &Value, file: &str) -> usize {
    value["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .filter(|hit| hit["file"].as_str() == Some(file))
        .count()
}

fn total_hits(value: &Value) -> usize {
    value["hits"].as_array().expect("hits array").len()
}

/// Assert a word query is served by exactly one path with exactly one hit.
fn assert_served_only_at(value: &Value, file: &str) {
    assert_eq!(total_hits(value), 1, "exactly one hit total: {value}");
    assert_eq!(hits_in(value, file), 1, "hit must be in {file}: {value}");
}

/// Assert a word query matches nothing anywhere (exit 0, empty hit set).
fn assert_served_nowhere(value: &Value) {
    assert_eq!(total_hits(value), 0, "stale hit served: {value}");
}

fn outline_names(value: &Value) -> Vec<String> {
    value["symbols"]
        .as_array()
        .expect("symbols array")
        .iter()
        .map(|s| s["name"].as_str().expect("symbol name").to_owned())
        .collect()
}

/// Make staleness unambiguous: content change plus an mtime step, so both the
/// mtime fast path and the content-hash fallback agree the file is stale.
fn rewrite_with_mtime_bump(path: &Path, content: &str) {
    fs::write(path, content).expect("rewrite");
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    fs::File::options()
        .write(true)
        .open(path)
        .expect("open for mtime")
        .set_modified(later)
        .expect("bump mtime");
}

// --- DRILL: MODIFY (added symbol) ---

#[test]
fn i4_drill_modify_add_symbol_full_cycle() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();

    // SERVE (baseline): the indexed tree serves the seed symbols.
    run_index(&bin, &index_s, &root_s);
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");
    let baseline = run_status(&bin, &index_s, &root_s);
    assert_eq!(baseline["file_count"], 2, "{baseline}");

    // CHANGE: alpha gains a second symbol.
    rewrite_with_mtime_bump(
        &root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\npub fn i4_modify_added() -> u32 { 4 }\n",
    );

    // DETECT (stale): status counts are still pre-change and the new symbol
    // is not served, while the old hit set is intact.
    let stale = run_status(&bin, &index_s, &root_s);
    assert_eq!(stale["file_count"], 2, "{stale}");
    assert_eq!(stale["symbol_count"], baseline["symbol_count"], "{stale} vs {baseline}");
    assert_served_nowhere(&run_search(&bin, &index_s, &root_s, "word:i4_modify_added"));
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");

    // REINDEX → SERVE: the exact new outputs are served.
    let rebuilt = run_reindex(&bin, &index_s, &root_s);
    assert_eq!(rebuilt["files_indexed"], 2, "{rebuilt}");
    assert_served_only_at(
        &run_search(&bin, &index_s, &root_s, "word:i4_modify_added"),
        "alpha.rs",
    );
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");
    let outline = run_outline(&bin, &index_s, &root_s, "alpha.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 2, "{outline_value}");
    assert_eq!(
        outline_names(&outline_value),
        vec!["alpha_one", "i4_modify_added"],
        "{outline_value}"
    );
    let converged = run_status(&bin, &index_s, &root_s);
    assert_eq!(
        converged["symbol_count"].as_u64().expect("symbol count"),
        baseline["symbol_count"].as_u64().expect("baseline count") + 1,
        "{converged} vs {baseline}"
    );
}

// --- DRILL: ADD ---

#[test]
fn i4_drill_add_file_full_cycle() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();

    // SERVE (baseline).
    run_index(&bin, &index_s, &root_s);
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:beta_one"), "beta.rs");
    assert_eq!(run_status(&bin, &index_s, &root_s)["file_count"], 2);

    // CHANGE: a third file appears.
    fs::write(
        root.join("gamma.rs"),
        "pub fn i4_add_gamma() -> u32 { 3 }\n",
    )
    .expect("add gamma");

    // DETECT (stale): status still reports the old file count, the new
    // symbol is served nowhere, and outline refuses the unindexed path.
    assert_eq!(run_status(&bin, &index_s, &root_s)["file_count"], 2);
    assert_served_nowhere(&run_search(&bin, &index_s, &root_s, "word:i4_add_gamma"));
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");
    let stale_outline = run_outline(&bin, &index_s, &root_s, "gamma.rs");
    assert_eq!(stale_outline.status.code(), Some(2), "unindexed path must refuse");
    assert_eq!(parse_stdout(&stale_outline)["ok"], false);

    // REINDEX → SERVE: the added file is served exactly.
    let rebuilt = run_reindex(&bin, &index_s, &root_s);
    assert_eq!(rebuilt["files_indexed"], 3, "{rebuilt}");
    assert_served_only_at(
        &run_search(&bin, &index_s, &root_s, "word:i4_add_gamma"),
        "gamma.rs",
    );
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:beta_one"), "beta.rs");
    let outline = run_outline(&bin, &index_s, &root_s, "gamma.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 1, "{outline_value}");
    assert_eq!(outline_names(&outline_value), vec!["i4_add_gamma"], "{outline_value}");
    assert_eq!(run_status(&bin, &index_s, &root_s)["file_count"], 3);
}

// --- DRILL: DELETE ---

#[test]
fn i4_drill_delete_file_full_cycle() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();

    // SERVE (baseline).
    run_index(&bin, &index_s, &root_s);
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:beta_one"), "beta.rs");
    let baseline = run_status(&bin, &index_s, &root_s);
    assert_eq!(baseline["file_count"], 2, "{baseline}");

    // CHANGE: beta is deleted from the tree.
    fs::remove_file(root.join("beta.rs")).expect("delete beta");

    // DETECT (stale): status counts are still pre-change and the deleted
    // symbol is STILL served — no implicit refresh happened.
    let stale = run_status(&bin, &index_s, &root_s);
    assert_eq!(stale["file_count"], 2, "{stale}");
    assert_eq!(stale["symbol_count"], baseline["symbol_count"], "{stale} vs {baseline}");
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:beta_one"), "beta.rs");

    // REINDEX → SERVE: the deleted symbol is gone everywhere, the survivor
    // is intact.
    let rebuilt = run_reindex(&bin, &index_s, &root_s);
    assert_eq!(rebuilt["files_indexed"], 1, "{rebuilt}");
    assert_served_nowhere(&run_search(&bin, &index_s, &root_s, "word:beta_one"));
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");
    let gone = run_outline(&bin, &index_s, &root_s, "beta.rs");
    assert_eq!(gone.status.code(), Some(2), "deleted path must refuse");
    assert_eq!(parse_stdout(&gone)["ok"], false);
    let kept = run_outline(&bin, &index_s, &root_s, "alpha.rs");
    assert_eq!(kept.status.code(), Some(0));
    let converged = run_status(&bin, &index_s, &root_s);
    assert_eq!(converged["file_count"], 1, "{converged}");
    assert_eq!(
        converged["symbol_count"].as_u64().expect("symbol count"),
        baseline["symbol_count"].as_u64().expect("baseline count") - 1,
        "{converged} vs {baseline}"
    );
}

// --- DRILL: RENAME ---

#[test]
fn i4_drill_rename_file_full_cycle() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();

    // SERVE (baseline).
    run_index(&bin, &index_s, &root_s);
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:beta_one"), "beta.rs");
    assert_eq!(run_status(&bin, &index_s, &root_s)["file_count"], 2);

    // CHANGE: beta moves to a new path with identical content.
    fs::rename(root.join("beta.rs"), root.join("beta_moved.rs")).expect("rename beta");

    // DETECT (stale): the hit is still served at the OLD path only.
    assert_eq!(run_status(&bin, &index_s, &root_s)["file_count"], 2);
    let stale = run_search(&bin, &index_s, &root_s, "word:beta_one");
    assert_served_only_at(&stale, "beta.rs");
    assert_eq!(hits_in(&stale, "beta_moved.rs"), 0, "unrefreshed new path: {stale}");

    // REINDEX → SERVE: the hit follows the new path exactly.
    let rebuilt = run_reindex(&bin, &index_s, &root_s);
    assert_eq!(rebuilt["files_indexed"], 2, "{rebuilt}");
    let moved = run_search(&bin, &index_s, &root_s, "word:beta_one");
    assert_served_only_at(&moved, "beta_moved.rs");
    assert_eq!(hits_in(&moved, "beta.rs"), 0, "stale old-path hit: {moved}");
    let stale_outline = run_outline(&bin, &index_s, &root_s, "beta.rs");
    assert_eq!(stale_outline.status.code(), Some(2), "old path must refuse");
    assert_eq!(parse_stdout(&stale_outline)["ok"], false);
    let outline = run_outline(&bin, &index_s, &root_s, "beta_moved.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 1, "{outline_value}");
    assert_eq!(outline_names(&outline_value), vec!["beta_one"], "{outline_value}");
    assert_eq!(run_status(&bin, &index_s, &root_s)["file_count"], 2);
}

// --- DRILL: MODIFY (removed symbol) ---

#[test]
fn i4_drill_modify_remove_symbol_full_cycle() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();

    // SERVE (baseline).
    run_index(&bin, &index_s, &root_s);
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");
    let baseline = run_status(&bin, &index_s, &root_s);
    assert_eq!(baseline["file_count"], 2, "{baseline}");

    // CHANGE: alpha_one is removed; an unrelated symbol takes its place.
    rewrite_with_mtime_bump(
        &root.join("alpha.rs"),
        "pub fn i4_modify_replacement() -> u32 { 5 }\n",
    );

    // DETECT (stale): the removed symbol is STILL served and the replacement
    // is served nowhere; status counts are still pre-change.
    let stale = run_status(&bin, &index_s, &root_s);
    assert_eq!(stale["file_count"], 2, "{stale}");
    assert_eq!(stale["symbol_count"], baseline["symbol_count"], "{stale} vs {baseline}");
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");
    assert_served_nowhere(&run_search(&bin, &index_s, &root_s, "word:i4_modify_replacement"));

    // REINDEX → SERVE: old token gone, new token served, outline exact.
    let rebuilt = run_reindex(&bin, &index_s, &root_s);
    assert_eq!(rebuilt["files_indexed"], 2, "{rebuilt}");
    assert_served_nowhere(&run_search(&bin, &index_s, &root_s, "word:alpha_one"));
    assert_served_only_at(
        &run_search(&bin, &index_s, &root_s, "word:i4_modify_replacement"),
        "alpha.rs",
    );
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:beta_one"), "beta.rs");
    let outline = run_outline(&bin, &index_s, &root_s, "alpha.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 1, "{outline_value}");
    assert_eq!(
        outline_names(&outline_value),
        vec!["i4_modify_replacement"],
        "{outline_value}"
    );
    assert_eq!(run_status(&bin, &index_s, &root_s)["file_count"], 2);
}

// --- DRILL: chained multi-change, one reindex ---

#[test]
fn i4_drill_chained_add_modify_delete_rename_single_reindex() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();

    // SERVE (baseline): both seeds served.
    run_index(&bin, &index_s, &root_s);
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:beta_one"), "beta.rs");
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
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:beta_one"), "beta.rs");
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

// --- DRILL: live watch mode (no manual refresh invocation) ---

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

/// Resume gate: the index is servable once `status` reports the full file
/// count. Polling the CLI discriminant keeps this drill independent of
/// watch's human-readable log lines.
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

/// Poll served search until `predicate` holds or the timeout expires. Returns
/// the last observed value; the caller asserts exact end-state counts on it.
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

#[test]
fn i4_drill_watch_mode_serves_modify_and_delete_without_manual_reindex() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();
    // No `index`/`reindex` invocation anywhere in this drill: the watch
    // daemon performs the initial index and every incremental refresh.
    let _watch = WatchChild::spawn(&bin, &root_s, &index_s);
    wait_until_servable(&bin, &index_s, &root_s, 2);

    // SERVE (baseline): the daemon's initial index serves the seeds.
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:beta_one"), "beta.rs");

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
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");

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
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");

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

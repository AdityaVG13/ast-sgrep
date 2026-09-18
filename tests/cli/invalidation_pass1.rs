//! I1 invalidation-contract oracles: CLI-side staleness decisions.
//!
//! Prior art this file does NOT duplicate:
//! - `watch_incremental` — library `update_paths` semantics (prune, ignore,
//!   symlink, batch-error atomicity).
//! - `watch_daemon_e2e` — live `watch` incremental updates via log markers.
//! - `machine_contracts::targeted_index_updates_are_bounded_deduplicated_and_confined`
//!   (`--path` edit/remove/dedup/confine/oversize) and `index_dry_run_does_not_mutate`.
//! - `cli_smoke` — search empty-refusal, `--auto-index` opt-in on empty,
//!   default/`--no-auto-index` no-refresh-after-edit.
//! - `durable_recovery_pass3::relation_double_reindex_observable_idempotence`
//!   (reindex==reindex, not index-then-index reuse).
//!
//! I1 pins: index freshness reuse vs rebuild counts, reindex full-rewrite
//! selection, targeted-unchanged noop, full-refresh prune, status freshness
//! discriminants, `--auto-index` stale refresh, flag precedence, the
//! `mutated()` reopen predicate, watch initial-index resume gates (observed
//! via `status`/`keyword` discriminants, never log text), and the
//! `reindex --dry-run` no-write branch.

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

fn fixture_two_files() -> (tempfile::TempDir, PathBuf, PathBuf, String, String) {
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
    (dir, root, index, root_s, index_s)
}

fn run_index(bin: &Path, index_s: &str, root_s: &str) -> Output {
    run(
        bin,
        &[
            "--json",
            "--no-embed",
            "--index-path",
            index_s,
            "index",
            root_s,
        ],
    )
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

// --- index subcommand: freshness detection (rebuild vs reuse) ---

#[test]
fn index_second_run_reuses_fresh_rows() {
    let bin = asgrep_bin();
    let (_dir, _root, _index, root_s, index_s) = fixture_two_files();
    let first = assert_success(&run_index(&bin, &index_s, &root_s), "index");
    assert_eq!(first["files_indexed"], 2, "{first}");
    assert_eq!(first["exit_code"], 0, "{first}");

    let second = assert_success(&run_index(&bin, &index_s, &root_s), "index");
    assert_eq!(second["files_indexed"], 0, "fresh rows must be reused: {second}");
    assert_eq!(second["files_skipped"], 2, "{second}");
    assert_eq!(second["files_removed"], 0, "{second}");
    assert_eq!(second["files_failed"], 0, "{second}");
}

#[test]
fn index_rebuilds_only_stale_file_after_edit() {
    let bin = asgrep_bin();
    let (_dir, root, _index, root_s, index_s) = fixture_two_files();
    assert_success(&run_index(&bin, &index_s, &root_s), "index");

    rewrite_with_mtime_bump(
        &root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\npub fn alpha_two() -> u32 { 2 }\n",
    );
    let refreshed = assert_success(&run_index(&bin, &index_s, &root_s), "index");
    assert_eq!(refreshed["files_indexed"], 1, "{refreshed}");
    assert_eq!(refreshed["files_skipped"], 1, "{refreshed}");
    assert_eq!(refreshed["files_removed"], 0, "{refreshed}");
}

#[test]
fn index_refresh_prunes_deleted_file() {
    let bin = asgrep_bin();
    let (_dir, root, _index, root_s, index_s) = fixture_two_files();
    assert_success(&run_index(&bin, &index_s, &root_s), "index");

    fs::remove_file(root.join("beta.rs")).expect("delete beta");
    let pruned = assert_success(&run_index(&bin, &index_s, &root_s), "index");
    assert_eq!(pruned["files_removed"], 1, "{pruned}");
    assert_eq!(pruned["files_indexed"], 0, "{pruned}");

    let status = assert_success(
        &run(
            &bin,
            &[
                "--json",
                "--no-embed",
                "--index-path",
                &index_s,
                "status",
                &root_s,
            ],
        ),
        "status",
    );
    assert_eq!(status["file_count"], 1, "{status}");
}

// --- incremental vs full rebuild selection ---

#[test]
fn reindex_rewrites_all_rows_without_edits() {
    let bin = asgrep_bin();
    let (_dir, _root, _index, root_s, index_s) = fixture_two_files();
    assert_success(&run_index(&bin, &index_s, &root_s), "index");

    let rebuilt = assert_success(
        &run(
            &bin,
            &[
                "--json",
                "--no-embed",
                "--index-path",
                &index_s,
                "reindex",
                &root_s,
            ],
        ),
        "reindex",
    );
    assert_eq!(
        rebuilt["files_indexed"], 2,
        "reindex must ignore freshness and rewrite all rows: {rebuilt}"
    );
    assert_eq!(rebuilt["files_skipped"], 0, "{rebuilt}");
}

#[test]
fn index_path_on_unchanged_file_is_noop_reuse() {
    let bin = asgrep_bin();
    let (_dir, _root, _index, root_s, index_s) = fixture_two_files();
    assert_success(&run_index(&bin, &index_s, &root_s), "index");

    let targeted = assert_success(
        &run(
            &bin,
            &[
                "--json",
                "--no-embed",
                "--index-path",
                &index_s,
                "index",
                &root_s,
                "--path",
                "alpha.rs",
            ],
        ),
        "index",
    );
    assert_eq!(targeted["targeted"], true, "{targeted}");
    assert_eq!(targeted["path_count"], 1, "{targeted}");
    assert_eq!(targeted["stats"]["files_indexed"], 0, "{targeted}");
    assert_eq!(targeted["stats"]["files_skipped"], 1, "{targeted}");
    assert_eq!(targeted["stats"]["files_removed"], 0, "{targeted}");
}

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

// --- status reporting discriminants ---

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
    assert_success(&run_index(&bin, &index_s, &root_s), "index");
    let status = assert_success(
        &run(
            &bin,
            &[
                "--json",
                "--no-embed",
                "--index-path",
                &index_s,
                "status",
                &root_s,
            ],
        ),
        "status",
    );
    assert_eq!(status["file_count"], 2, "{status}");
    assert!(status["writer_generation"].is_u64(), "{status}");
    assert!(
        status["durability"].as_str().is_some_and(|d| !d.is_empty()),
        "{status}"
    );
    assert!(status["semantic_ivf_present"].is_boolean(), "{status}");
    assert!(status["symbol_count"].is_u64(), "{status}");
}

// --- stale-cache refresh vs refusal ---

#[test]
fn search_auto_index_refreshes_stale_index_after_edit() {
    let bin = asgrep_bin();
    let (_dir, root, _index, root_s, index_s) = fixture_two_files();
    assert_success(&run_index(&bin, &index_s, &root_s), "index");

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
    let value = parse_stdout(&output);
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["error"]["kind"], "operational", "{value}");
}

// --- reopen predicate behind ensure_fresh_index (lib) ---

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

// --- watch resume-gate boundaries (status/keyword observables only) ---

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
/// count. Polling the CLI discriminant keeps this oracle independent of
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

#[test]
fn watch_restart_resumes_to_servable_index() {
    let bin = asgrep_bin();
    let (_dir, _root, _index, root_s, index_s) = fixture_two_files();
    assert_success(&run_index(&bin, &index_s, &root_s), "index");

    for _ in 0..2 {
        let _watch = WatchChild::spawn(&bin, &root_s, &index_s);
        wait_until_servable(&bin, &index_s, &root_s, 2);
        let found = keyword_hits(&bin, &index_s, &root_s, "beta_one");
        let hits = found["hits"].as_array().expect("hits array");
        assert!(!hits.is_empty(), "seed must survive watch resume: {found}");
    }

    let status = assert_success(
        &run(
            &bin,
            &[
                "--json",
                "--no-embed",
                "--index-path",
                &index_s,
                "status",
                &root_s,
            ],
        ),
        "status",
    );
    assert_eq!(status["file_count"], 2, "{status}");
}

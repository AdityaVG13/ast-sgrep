//! I3 rebuild-parity metamorphic relations: INCREMENTAL == FULL REBUILD.
//!
//! Prior art this file does NOT duplicate:
//! - `invalidation_pass1` (I1) — index freshness reuse/rebuild COUNTS,
//!   `reindex` full-rewrite selection, targeted-unchanged noop counts,
//!   status freshness discriminants, `--auto-index` flag precedence, the
//!   `mutated()` reopen predicate, watch resume gates, `--dry-run`.
//! - `invalidation_pass2` (I2) — per-DELTA served-output oracles (ADD / MODIFY
//!   / DELETE / RENAME / NO-OP-REWRITE each reflected exactly in search +
//!   outline after one refresh).
//! - `watch_incremental` — library `update_paths` semantics via the store.
//! - `watch_daemon_e2e` — live `watch` incremental updates via log markers.
//! - `machine_contracts` — `--path` counts, dry-run non-mutation, shapes.
//! - `durable_recovery_pass3::relation_double_reindex_observable_idempotence`
//!   (reindex==reindex observed via one status/count path).
//!
//! I3 pins RELATIONS between two refresh paths over the same final tree,
//! observed via the real binary's served bytes with `--no-auto-index`
//! (exactly what the last explicit refresh wrote — no implicit refresh):
//! - incremental refresh (`index` over an existing DB) == clean rebuild
//!   (fresh DB over the same final tree): identical search/outline bytes.
//! - targeted `--path` refreshes == full `index` refresh: identical bytes.
//! - delta application order is unobservable: different orders ending at the
//!   same tree serve identical bytes.
//! - `reindex` is idempotent: reindex twice == reindex once (byte-identical
//!   served outputs), and reindex == clean rebuild.
//! - after convergence (a refresh that changes nothing), repeated `status`
//!   and served outputs are stable.
//!
//! Every assertion is on exit codes, output bytes, and counts — never message
//! text.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

/// A second, independent index database over the same root.
fn second_index_path(dir: &tempfile::TempDir) -> String {
    dir.path()
        .join("idx2")
        .join("index.db")
        .to_str()
        .expect("index2 utf8")
        .to_owned()
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

/// Raw served search output: exit code + stdout bytes are the relation.
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

fn run_outline_raw(bin: &Path, index_s: &str, root_s: &str, rel: &str) -> Output {
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

fn run_status(bin: &Path, index_s: &str, root_s: &str) -> Output {
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

/// Queries spanning added, modified, retained, and deleted symbols. Served
/// over a converged index, the (exit code, stdout bytes) vector fully
/// characterizes the observable search state for this fixture family.
const PROBE_QUERIES: &[&str] = &[
    "word:alpha_one",
    "word:alpha_two",
    "word:beta_one",
    "word:gamma_new",
];

/// Search envelope with the write counter removed. `snapshot.generation`
/// counts index writes, so it legitimately differs between an incrementally
/// refreshed database and a clean rebuild (or across two reindexes); every
/// other field — hits, lines, scores, `worktree_revision` — must agree.
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

fn snapshot_search(bin: &Path, index_s: &str, root_s: &str) -> Vec<(Option<i32>, Vec<u8>)> {
    PROBE_QUERIES
        .iter()
        .map(|query| {
            let output = run_search_raw(bin, index_s, root_s, query);
            (output.status.code(), normalized_search_bytes(&output))
        })
        .collect()
}

/// Status envelope with the per-process writer token removed.
/// `writer_generation` is random per process open, so it cannot participate
/// in any cross-invocation relation; all other fields must agree.
fn normalized_status_bytes(output: &Output) -> Vec<u8> {
    let mut value = parse_stdout(output);
    if let Some(map) = value.as_object_mut() {
        map.remove("writer_generation");
    }
    serde_json::to_vec(&value).expect("reserialize normalized status")
}

fn snapshot_outline(bin: &Path, index_s: &str, root_s: &str, rels: &[&str]) -> Vec<(Option<i32>, Vec<u8>)> {
    rels.iter()
        .map(|rel| {
            let output = run_outline_raw(bin, index_s, root_s, rel);
            (output.status.code(), output.stdout.clone())
        })
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

/// Mutate the two-file fixture into its canonical "final" tree: alpha gains a
/// symbol, beta is deleted, gamma is added.
fn mutate_to_final_tree(root: &Path) {
    rewrite_with_mtime_bump(
        &root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\npub fn alpha_two() -> u32 { 2 }\n",
    );
    fs::remove_file(root.join("beta.rs")).expect("delete beta");
    fs::write(
        root.join("gamma.rs"),
        "pub fn gamma_new() -> u32 { 3 }\n",
    )
    .expect("add gamma");
}

// --- RELATION: incremental refresh == clean rebuild ---

#[test]
fn i3_incremental_refresh_matches_clean_rebuild_search_bytes() {
    let bin = asgrep_bin();
    let (dir, root, root_s, index_s) = fixture_two_files();
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
    for (query, (left, right)) in PROBE_QUERIES.iter().zip(incremental.iter().zip(clean.iter())) {
        assert_eq!(left.0, Some(0), "{query}: incremental search must exit 0");
        assert_eq!(right.0, Some(0), "{query}: clean search must exit 0");
        assert_eq!(left.1, right.1, "{query}: incremental != clean rebuild");
    }
}

#[test]
fn i3_incremental_refresh_matches_clean_rebuild_outline_bytes() {
    let bin = asgrep_bin();
    let (dir, root, root_s, index_s) = fixture_two_files();
    let clean_s = second_index_path(&dir);

    run_index(&bin, &index_s, &root_s);
    mutate_to_final_tree(&root);
    run_index(&bin, &index_s, &root_s);
    run_index(&bin, &clean_s, &root_s);

    // Retained + modified + added paths serve bytes; the deleted path refuses
    // identically on both sides (exit code is part of the relation).
    let rels = ["alpha.rs", "gamma.rs", "beta.rs"];
    let incremental = snapshot_outline(&bin, &index_s, &root_s, &rels);
    let clean = snapshot_outline(&bin, &clean_s, &root_s, &rels);
    for (rel, (left, right)) in rels.iter().zip(incremental.iter().zip(clean.iter())) {
        assert_eq!(left.0, right.0, "{rel}: outline exit codes diverge");
        assert_eq!(left.1, right.1, "{rel}: outline bytes diverge");
    }
    assert_eq!(incremental[0].0, Some(0));
    assert_eq!(incremental[1].0, Some(0));
    assert_eq!(incremental[2].0, Some(2));
}

#[test]
fn i3_incremental_refresh_matches_clean_rebuild_counts() {
    let bin = asgrep_bin();
    let (dir, root, root_s, index_s) = fixture_two_files();
    let clean_s = second_index_path(&dir);

    run_index(&bin, &index_s, &root_s);
    mutate_to_final_tree(&root);
    run_index(&bin, &index_s, &root_s);
    run_index(&bin, &clean_s, &root_s);

    let left = assert_success(&run_status(&bin, &index_s, &root_s), "status");
    let right = assert_success(&run_status(&bin, &clean_s, &root_s), "status");
    assert_eq!(left["file_count"], 2, "{left}");
    assert_eq!(left["file_count"], right["file_count"], "{left} vs {right}");
    assert_eq!(left["symbol_count"], right["symbol_count"], "{left} vs {right}");

    // Total served hit counts agree per query as well.
    for query in PROBE_QUERIES {
        let l = parse_stdout(&run_search_raw(&bin, &index_s, &root_s, query));
        let r = parse_stdout(&run_search_raw(&bin, &clean_s, &root_s, query));
        assert_eq!(
            l["hits"].as_array().expect("hits").len(),
            r["hits"].as_array().expect("hits").len(),
            "{query}: hit counts diverge"
        );
    }
}

#[test]
fn i3_targeted_path_refreshes_match_full_refresh_bytes() {
    let bin = asgrep_bin();
    let (dir, root, root_s, index_s) = fixture_two_files();
    let full_s = second_index_path(&dir);

    run_index(&bin, &index_s, &root_s);
    run_index(&bin, &full_s, &root_s);

    // Two edits land via one full refresh on one DB and via two targeted
    // `--path` refreshes on the other; both must converge to the same bytes.
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

    let targeted = snapshot_search(&bin, &index_s, &root_s);
    let full = snapshot_search(&bin, &full_s, &root_s);
    for (query, (left, right)) in PROBE_QUERIES.iter().zip(targeted.iter().zip(full.iter())) {
        assert_eq!((left.0, right.0), (Some(0), Some(0)), "{query}");
        assert_eq!(left.1, right.1, "{query}: targeted != full refresh");
    }

    let rels = ["alpha.rs", "beta.rs"];
    let targeted_outline = snapshot_outline(&bin, &index_s, &root_s, &rels);
    let full_outline = snapshot_outline(&bin, &full_s, &root_s, &rels);
    for (rel, (left, right)) in rels.iter().zip(targeted_outline.iter().zip(full_outline.iter())) {
        assert_eq!((left.0, right.0), (Some(0), Some(0)), "{rel}");
        assert_eq!(left.1, right.1, "{rel}: targeted != full refresh");
    }
}

// --- RELATION: delta application order is unobservable ---

/// Fixed per-step mtimes for order-independence convergence.
/// `worktree_revision` is `MAX(mtime_secs)` over stored rows, and a refresh
/// rewrites a row only when its content changed — mtime-only touches never
/// update the recorded value. Both orders must therefore stamp every content
/// write to the same fixed instant: distinct steps get distinct instants so
/// the mtime fast path still fires, and both orders record identical mtimes.
const MTIME_V0: u64 = 1_900_000_000;
const MTIME_ADDED: u64 = 1_900_000_001;
const MTIME_MODIFIED: u64 = 1_900_000_002;

fn stamp(path: &Path, secs: u64) {
    let fixed = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
    fs::File::options()
        .write(true)
        .open(path)
        .expect("open for stamp")
        .set_modified(fixed)
        .expect("stamp mtime");
}

/// Stamp the freshly created v0 fixture so its recorded mtimes are fixed.
fn stamp_v0(root: &Path) {
    stamp(&root.join("alpha.rs"), MTIME_V0);
    stamp(&root.join("beta.rs"), MTIME_V0);
}

/// Reset the fixture root to its v0 bytes at the fixed v0 instant, and drop
/// the index so the next order starts clean.
fn reset_to_v0(root: &Path, index: &Path) {
    fs::write(root.join("alpha.rs"), "pub fn alpha_one() -> u32 { 1 }\n").expect("restore alpha");
    stamp(&root.join("alpha.rs"), MTIME_V0);
    fs::write(root.join("beta.rs"), "pub fn beta_one() -> u32 { 2 }\n").expect("restore beta");
    stamp(&root.join("beta.rs"), MTIME_V0);
    let gamma = root.join("gamma.rs");
    if gamma.exists() {
        fs::remove_file(&gamma).expect("remove gamma");
    }
    if index.exists() {
        fs::remove_file(index).expect("drop index");
    }
}

fn add_gamma(root: &Path) {
    fs::write(
        root.join("gamma.rs"),
        "pub fn gamma_new() -> u32 { 3 }\n",
    )
    .expect("add gamma");
    stamp(&root.join("gamma.rs"), MTIME_ADDED);
}

fn modify_alpha(root: &Path) {
    fs::write(
        &root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\npub fn alpha_two() -> u32 { 2 }\n",
    )
    .expect("modify alpha");
    stamp(&root.join("alpha.rs"), MTIME_MODIFIED);
}

#[test]
fn i3_delta_order_add_then_modify_matches_modify_then_add() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();
    let index = PathBuf::from(&index_s);

    // Order 1: add gamma (refresh), then modify alpha (refresh).
    stamp_v0(&root);
    run_index(&bin, &index_s, &root_s);
    add_gamma(&root);
    run_index(&bin, &index_s, &root_s);
    modify_alpha(&root);
    run_index(&bin, &index_s, &root_s);
    let order1_search = snapshot_search(&bin, &index_s, &root_s);
    let order1_outline = snapshot_outline(&bin, &index_s, &root_s, &["alpha.rs", "gamma.rs"]);

    // Order 2: modify alpha (refresh), then add gamma (refresh).
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

#[test]
fn i3_delta_order_delete_then_add_matches_add_then_delete() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();
    let index = PathBuf::from(&index_s);

    // Order 1: delete beta (refresh), then add gamma (refresh).
    stamp_v0(&root);
    run_index(&bin, &index_s, &root_s);
    fs::remove_file(root.join("beta.rs")).expect("delete beta");
    run_index(&bin, &index_s, &root_s);
    add_gamma(&root);
    run_index(&bin, &index_s, &root_s);
    let order1_search = snapshot_search(&bin, &index_s, &root_s);
    let order1_outline =
        snapshot_outline(&bin, &index_s, &root_s, &["alpha.rs", "gamma.rs", "beta.rs"]);

    // Order 2: add gamma (refresh), then delete beta (refresh).
    reset_to_v0(&root, &index);
    run_index(&bin, &index_s, &root_s);
    add_gamma(&root);
    run_index(&bin, &index_s, &root_s);
    fs::remove_file(root.join("beta.rs")).expect("delete beta");
    run_index(&bin, &index_s, &root_s);
    let order2_search = snapshot_search(&bin, &index_s, &root_s);
    let order2_outline =
        snapshot_outline(&bin, &index_s, &root_s, &["alpha.rs", "gamma.rs", "beta.rs"]);

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

// --- RELATION: reindex idempotence ---

#[test]
fn i3_reindex_twice_serves_byte_identical_search() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();

    run_index(&bin, &index_s, &root_s);
    mutate_to_final_tree(&root);

    run_reindex(&bin, &index_s, &root_s);
    let once = snapshot_search(&bin, &index_s, &root_s);
    run_reindex(&bin, &index_s, &root_s);
    let twice = snapshot_search(&bin, &index_s, &root_s);

    for (query, (left, right)) in PROBE_QUERIES.iter().zip(once.iter().zip(twice.iter())) {
        assert_eq!((left.0, right.0), (Some(0), Some(0)), "{query}");
        assert_eq!(left.1, right.1, "{query}: reindex is not idempotent");
    }
}

#[test]
fn i3_reindex_twice_outline_and_status_counts_stable() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();

    run_index(&bin, &index_s, &root_s);
    mutate_to_final_tree(&root);

    run_reindex(&bin, &index_s, &root_s);
    let rels = ["alpha.rs", "gamma.rs", "beta.rs"];
    let outline_once = snapshot_outline(&bin, &index_s, &root_s, &rels);
    let status_once = assert_success(&run_status(&bin, &index_s, &root_s), "status");

    run_reindex(&bin, &index_s, &root_s);
    let outline_twice = snapshot_outline(&bin, &index_s, &root_s, &rels);
    let status_twice = assert_success(&run_status(&bin, &index_s, &root_s), "status");

    for (rel, (left, right)) in rels.iter().zip(outline_once.iter().zip(outline_twice.iter())) {
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

#[test]
fn i3_reindex_matches_clean_rebuild_bytes() {
    let bin = asgrep_bin();
    let (dir, root, root_s, index_s) = fixture_two_files();
    let clean_s = second_index_path(&dir);

    // Reindex path: index v0, mutate, full rewrite in place.
    run_index(&bin, &index_s, &root_s);
    mutate_to_final_tree(&root);
    run_reindex(&bin, &index_s, &root_s);

    // Clean path: fresh database indexed once over the final tree.
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

// --- RELATION: stability after convergence ---

#[test]
fn i3_converged_refresh_changes_no_observable_bytes() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();

    run_index(&bin, &index_s, &root_s);
    mutate_to_final_tree(&root);
    run_index(&bin, &index_s, &root_s);

    let search_before = snapshot_search(&bin, &index_s, &root_s);
    let outline_before = snapshot_outline(&bin, &index_s, &root_s, &["alpha.rs", "gamma.rs"]);
    let status_before = run_status(&bin, &index_s, &root_s);

    // The converged refresh must be a reuse no-op ...
    let converged = run_index(&bin, &index_s, &root_s);
    assert_eq!(converged["files_indexed"], 0, "{converged}");
    assert_eq!(converged["files_removed"], 0, "{converged}");

    // ... and no observable byte may change across it.
    let search_after = snapshot_search(&bin, &index_s, &root_s);
    let outline_after = snapshot_outline(&bin, &index_s, &root_s, &["alpha.rs", "gamma.rs"]);
    let status_after = run_status(&bin, &index_s, &root_s);
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

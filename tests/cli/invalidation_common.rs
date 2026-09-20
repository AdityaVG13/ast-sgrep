//! Shared harness for the CLI invalidation suites.
//!
//! Single canonical copy of the helpers the four `invalidation_pass*` files
//! each re-declared (~60 lines ×4, plus `rewrite_with_mtime_bump` ×4 and the
//! watch helpers ×2). Spawn/parse/assert core is reused from testkit; every
//! helper defined here is invalidation-specific and NOT in testkit: each
//! carries a WHY area-local note naming the testkit gap. This include is the
//! shared home (single definition across the four suites); crate promotion
//! would add public API with no new sharing.
//!
//! Included via `#[path = "invalidation_common.rs"]`; it is NOT a test target.

#![allow(dead_code)] // why: shared across 4 test targets, each using a subset.

use ast_sgrep_testkit::{assert_success, run};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

// why: the invalidation two-file (alpha/beta) project + explicit index-path
// layout every suite builds from. WHY area-local: testkit `seed_project` is a
// single-file `src/lib.rs` recovery fixture, not this shape; shared here via
// the include.
pub fn fixture_two_files() -> (tempfile::TempDir, PathBuf, PathBuf, String, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("proj");
    fs::create_dir_all(&root).expect("proj");
    fs::write(root.join("alpha.rs"), "pub fn alpha_one() -> u32 { 1 }\n").expect("alpha");
    fs::write(root.join("beta.rs"), "pub fn beta_one() -> u32 { 2 }\n").expect("beta");
    let index = dir.path().join("idx").join("index.db");
    let root_s = root.to_str().expect("root utf8").to_owned();
    let index_s = index.to_str().expect("index utf8").to_owned();
    (dir, root, index, root_s, index_s)
}

// why: makes staleness unambiguous — content change plus an mtime step, so
// both the mtime fast path and the content-hash fallback agree the file is
// stale. WHY area-local: no content+mtime rewrite helper exists in testkit;
// shared here via the include.
pub fn rewrite_with_mtime_bump(path: &Path, content: &str) {
    fs::write(path, content).expect("rewrite");
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
    fs::File::options()
        .write(true)
        .open(path)
        .expect("open for mtime")
        .set_modified(later)
        .expect("bump mtime");
}

// why: full refresh (`index`) beat returning the success envelope for count
// facets. WHY area-local: `cli_recovery::run_index` is TempDir/cwd-bound with
// a different arg shape and cannot serve these suites' explicit-path calls;
// shared here via the include.
pub fn run_index(bin: &Path, index_s: &str, root_s: &str) -> Value {
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

// why: full rewrite (`reindex`) beat returning the success envelope.
// WHY area-local: same TempDir/cwd/arg-shape mismatch as `run_index`; shared
// here via the include.
pub fn run_reindex(bin: &Path, index_s: &str, root_s: &str) -> Value {
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

// why: `status` success-envelope beat for count discriminants.
// WHY area-local: `cli_recovery::run_status` is TempDir/cwd-bound and omits
// `--no-embed`, a different invocation shape; shared here via the include.
pub fn run_status(bin: &Path, index_s: &str, root_s: &str) -> Value {
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

// why: deterministic served-output read — `--no-auto-index` so the hit set is
// exactly what the last explicit refresh wrote, no implicit refresh.
// WHY area-local: `cli_recovery::run_search` omits `--no-embed` and is
// TempDir/cwd-bound; this pins the exact invalidation read shape, shared here
// via the include.
pub fn run_search(bin: &Path, index_s: &str, root_s: &str, query: &str) -> Value {
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

// why: raw `outline` run (no `--no-embed`: outline takes `--root` + rel path)
// returning the raw Output so refusal exit codes stay assertable.
// WHY area-local: `cli_recovery::run_outline_snapshot` asserts success, so it
// cannot pin refusal codes; shared here via the include.
pub fn run_outline(bin: &Path, index_s: &str, root_s: &str, rel: &str) -> Output {
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

// why: hit count for one indexed path within a search response.
// WHY area-local: served-set projection specific to invalidation oracles;
// shared here via the include.
pub fn hits_in(value: &Value, file: &str) -> usize {
    value["hits"]
        .as_array()
        .expect("hits array")
        .iter()
        .filter(|hit| hit["file"].as_str() == Some(file))
        .count()
}

// why: total hit count within a search response. WHY area-local: same as
// `hits_in`.
pub fn total_hits(value: &Value) -> usize {
    value["hits"].as_array().expect("hits array").len()
}

// why: assert a word query is served by exactly one path with exactly one hit.
// WHY area-local: same as `hits_in`.
pub fn assert_served_only_at(value: &Value, file: &str) {
    assert_eq!(total_hits(value), 1, "exactly one hit total: {value}");
    assert_eq!(hits_in(value, file), 1, "hit must be in {file}: {value}");
}

// why: assert a word query matches nothing anywhere (exit 0, empty hit set).
// WHY area-local: same as `hits_in`.
pub fn assert_served_nowhere(value: &Value) {
    assert_eq!(total_hits(value), 0, "stale hit served: {value}");
}

// why: served symbol-name projection for outline responses.
// WHY area-local: same as `hits_in`.
pub fn outline_names(value: &Value) -> Vec<String> {
    value["symbols"]
        .as_array()
        .expect("symbols array")
        .iter()
        .map(|s| s["name"].as_str().expect("symbol name").to_owned())
        .collect()
}

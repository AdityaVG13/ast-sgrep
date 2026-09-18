//! I2 delta-discriminating oracles: each FILE DELTA is proven via the real
//! binary's served output (search hit sets per path + outline symbol sets).
//!
//! Prior art this file does NOT duplicate:
//! - `invalidation_pass1` (I1) — index freshness reuse/rebuild COUNTS,
//!   `reindex` full-rewrite selection, targeted-unchanged noop counts,
//!   status freshness discriminants, `--auto-index` flag precedence, the
//!   `mutated()` reopen predicate, watch resume gates, `--dry-run`.
//! - `watch_incremental` — library `update_paths` semantics (prune, ignore,
//!   symlink, batch-error atomicity) observed via the store, never via CLI.
//! - `watch_daemon_e2e` — live `watch` incremental updates via log markers.
//! - `machine_contracts` — `--path` edit/remove/dedup/confine/oversize counts,
//!   dry-run non-mutation, agent/capsule/compact shapes.
//! - `outline_cmd` — basic outline surface (JSON/human/refusal), no deltas.
//! - `cli_smoke` — empty-refusal and auto-index refusal defaults, no deltas.
//!
//! I2 pins: after an explicit refresh invocation, ADD / MODIFY / DELETE /
//! RENAME / NO-OP-REWRITE deltas are each reflected EXACTLY in subsequent
//! `search` (hit sets per path, hit counts, hit lines) and `outline`
//! (symbol sets, counts) output. Stale hits are never served after refresh.
//! Every assertion is on exit codes and hit counts/sets — never message text.
//! Every refresh is an explicit CLI invocation (deterministic, no watchers).

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

/// Full refresh (`index`): picks up adds, modifies, deletes, renames.
fn run_refresh(bin: &Path, index_s: &str, root_s: &str) -> Value {
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

/// Deterministic served-output read: `--no-auto-index` so the served hit set
/// is exactly what the last explicit refresh wrote — no implicit refresh.
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

/// Hit count for one indexed path within a search response.
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

// --- DELTA: ADD ---

#[test]
fn i2_delta_add_single_file_search_and_outline_serve_new_symbol() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();
    run_refresh(&bin, &index_s, &root_s);

    fs::write(
        root.join("gamma.rs"),
        "pub fn gamma_new() -> u32 { 3 }\n",
    )
    .expect("add gamma");
    run_refresh(&bin, &index_s, &root_s);

    let added = run_search(&bin, &index_s, &root_s, "word:gamma_new");
    assert_served_only_at(&added, "gamma.rs");

    // Pre-existing hit sets are untouched by the add.
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:beta_one"), "beta.rs");

    let outline = run_outline(&bin, &index_s, &root_s, "gamma.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["ok"], true, "{outline_value}");
    assert_eq!(outline_value["count"], 1, "{outline_value}");
    assert_eq!(outline_names(&outline_value), vec!["gamma_new"], "{outline_value}");
}

#[test]
fn i2_delta_add_two_files_hit_sets_partitioned_per_path() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();
    run_refresh(&bin, &index_s, &root_s);

    fs::write(root.join("file_cee.rs"), "pub fn cee_sym() -> u32 { 7 }\n").expect("add cee");
    fs::write(root.join("file_dee.rs"), "pub fn dee_sym() -> u32 { 8 }\n").expect("add dee");
    run_refresh(&bin, &index_s, &root_s);

    let cee = run_search(&bin, &index_s, &root_s, "word:cee_sym");
    assert_served_only_at(&cee, "file_cee.rs");
    assert_eq!(hits_in(&cee, "file_dee.rs"), 0, "cross-file leak: {cee}");

    let dee = run_search(&bin, &index_s, &root_s, "word:dee_sym");
    assert_served_only_at(&dee, "file_dee.rs");
    assert_eq!(hits_in(&dee, "file_cee.rs"), 0, "cross-file leak: {dee}");
}

// --- DELTA: MODIFY ---

#[test]
fn i2_delta_modify_added_symbol_served_old_symbols_retained() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();
    run_refresh(&bin, &index_s, &root_s);

    rewrite_with_mtime_bump(
        &root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\npub fn alpha_two() -> u32 { 2 }\n",
    );
    run_refresh(&bin, &index_s, &root_s);

    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_two"), "alpha.rs");
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:beta_one"), "beta.rs");

    let outline = run_outline(&bin, &index_s, &root_s, "alpha.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 2, "{outline_value}");
    assert_eq!(
        outline_names(&outline_value),
        vec!["alpha_one", "alpha_two"],
        "{outline_value}"
    );
}

#[test]
fn i2_delta_modify_removed_symbol_stale_hit_never_served() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();
    run_refresh(&bin, &index_s, &root_s);
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");

    rewrite_with_mtime_bump(&root.join("alpha.rs"), "pub fn alpha_two() -> u32 { 2 }\n");
    run_refresh(&bin, &index_s, &root_s);

    assert_served_nowhere(&run_search(&bin, &index_s, &root_s, "word:alpha_one"));
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_two"), "alpha.rs");

    let outline = run_outline(&bin, &index_s, &root_s, "alpha.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 1, "{outline_value}");
    assert_eq!(outline_names(&outline_value), vec!["alpha_two"], "{outline_value}");
}

#[test]
fn i2_delta_modify_renamed_symbol_old_token_gone_new_token_served() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();
    run_refresh(&bin, &index_s, &root_s);

    rewrite_with_mtime_bump(
        &root.join("alpha.rs"),
        "pub fn alpha_renamed() -> u32 { 1 }\n",
    );
    run_refresh(&bin, &index_s, &root_s);

    assert_served_nowhere(&run_search(&bin, &index_s, &root_s, "word:alpha_one"));
    assert_served_only_at(
        &run_search(&bin, &index_s, &root_s, "word:alpha_renamed"),
        "alpha.rs",
    );

    let outline = run_outline(&bin, &index_s, &root_s, "alpha.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 1, "{outline_value}");
    assert_eq!(
        outline_names(&outline_value),
        vec!["alpha_renamed"],
        "{outline_value}"
    );
}

#[test]
fn i2_delta_modify_line_shift_hit_lines_follow_new_rows() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();
    run_refresh(&bin, &index_s, &root_s);

    // Prepend two lines so the symbol moves from line 1 to line 3. A stale
    // row would still serve line_start == 1.
    rewrite_with_mtime_bump(
        &root.join("alpha.rs"),
        "// pad line\n\npub fn alpha_one() -> u32 { 1 }\n",
    );
    run_refresh(&bin, &index_s, &root_s);

    let found = run_search(&bin, &index_s, &root_s, "word:alpha_one");
    assert_served_only_at(&found, "alpha.rs");
    assert_eq!(
        found["hits"][0]["line_start"], 3,
        "hit must follow the symbol to its new line: {found}"
    );

    let outline = run_outline(&bin, &index_s, &root_s, "alpha.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(
        outline_value["symbols"][0]["line_start"], 3,
        "outline must follow the symbol to its new line: {outline_value}"
    );
}

// --- DELTA: DELETE ---

#[test]
fn i2_delta_delete_file_prunes_search_and_outline() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();
    run_refresh(&bin, &index_s, &root_s);
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:beta_one"), "beta.rs");

    fs::remove_file(root.join("beta.rs")).expect("delete beta");
    run_refresh(&bin, &index_s, &root_s);

    assert_served_nowhere(&run_search(&bin, &index_s, &root_s, "word:beta_one"));
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");

    let gone = run_outline(&bin, &index_s, &root_s, "beta.rs");
    assert_eq!(gone.status.code(), Some(2), "deleted path must refuse");
    assert_eq!(parse_stdout(&gone)["ok"], false);

    let kept = run_outline(&bin, &index_s, &root_s, "alpha.rs");
    assert_eq!(kept.status.code(), Some(0));
}

#[test]
fn i2_delta_delete_then_readd_restores_exact_hit_set() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();
    run_refresh(&bin, &index_s, &root_s);

    fs::remove_file(root.join("beta.rs")).expect("delete beta");
    run_refresh(&bin, &index_s, &root_s);
    assert_served_nowhere(&run_search(&bin, &index_s, &root_s, "word:beta_one"));

    fs::write(root.join("beta.rs"), "pub fn beta_one() -> u32 { 2 }\n").expect("re-add beta");
    run_refresh(&bin, &index_s, &root_s);

    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:beta_one"), "beta.rs");

    let outline = run_outline(&bin, &index_s, &root_s, "beta.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 1, "{outline_value}");
    assert_eq!(outline_names(&outline_value), vec!["beta_one"], "{outline_value}");
}

// --- DELTA: RENAME ---

#[test]
fn i2_delta_rename_file_hits_follow_new_path_only() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();
    run_refresh(&bin, &index_s, &root_s);

    fs::rename(root.join("beta.rs"), root.join("beta_moved.rs")).expect("rename beta");
    run_refresh(&bin, &index_s, &root_s);

    let found = run_search(&bin, &index_s, &root_s, "word:beta_one");
    assert_served_only_at(&found, "beta_moved.rs");
    assert_eq!(hits_in(&found, "beta.rs"), 0, "stale old-path hit: {found}");

    let stale = run_outline(&bin, &index_s, &root_s, "beta.rs");
    assert_eq!(stale.status.code(), Some(2), "old path must refuse");
    assert_eq!(parse_stdout(&stale)["ok"], false);

    let moved = run_outline(&bin, &index_s, &root_s, "beta_moved.rs");
    assert_eq!(moved.status.code(), Some(0));
    let moved_value = parse_stdout(&moved);
    assert_eq!(moved_value["count"], 1, "{moved_value}");
    assert_eq!(outline_names(&moved_value), vec!["beta_one"], "{moved_value}");
}

#[test]
fn i2_delta_rename_with_edit_new_symbol_at_new_path_only() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();
    run_refresh(&bin, &index_s, &root_s);

    fs::remove_file(root.join("beta.rs")).expect("remove old beta");
    fs::write(
        root.join("moved.rs"),
        "pub fn moved_sym() -> u32 { 9 }\n",
    )
    .expect("write moved");
    run_refresh(&bin, &index_s, &root_s);

    assert_served_nowhere(&run_search(&bin, &index_s, &root_s, "word:beta_one"));
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:moved_sym"), "moved.rs");
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");

    let stale = run_outline(&bin, &index_s, &root_s, "beta.rs");
    assert_eq!(stale.status.code(), Some(2), "old path must refuse");
}

// --- DELTA: NO-OP REWRITE ---

#[test]
fn i2_delta_noop_rewrite_same_bytes_hit_sets_identical() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();
    run_refresh(&bin, &index_s, &root_s);

    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:beta_one"), "beta.rs");

    // Same bytes, fresh mtime: even if the mtime fast path fires, the served
    // output must be byte-identical to before the rewrite.
    rewrite_with_mtime_bump(&root.join("alpha.rs"), "pub fn alpha_one() -> u32 { 1 }\n");
    let refreshed = run_refresh(&bin, &index_s, &root_s);
    assert_eq!(refreshed["exit_code"], 0, "{refreshed}");

    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:beta_one"), "beta.rs");

    let outline = run_outline(&bin, &index_s, &root_s, "alpha.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 1, "{outline_value}");
    assert_eq!(outline_names(&outline_value), vec!["alpha_one"], "{outline_value}");
}

// --- DELTA: TARGETED REFRESH ---

#[test]
fn i2_delta_targeted_path_refresh_serves_edit_without_full_rescan() {
    let bin = asgrep_bin();
    let (_dir, root, root_s, index_s) = fixture_two_files();
    run_refresh(&bin, &index_s, &root_s);

    rewrite_with_mtime_bump(
        &root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\npub fn alpha_two() -> u32 { 2 }\n",
    );
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
    assert_eq!(targeted["exit_code"], 0, "{targeted}");

    // The targeted delta lands in served output; the untouched file is intact.
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_two"), "alpha.rs");
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:alpha_one"), "alpha.rs");
    assert_served_only_at(&run_search(&bin, &index_s, &root_s, "word:beta_one"), "beta.rs");

    let outline = run_outline(&bin, &index_s, &root_s, "alpha.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 2, "{outline_value}");
}

//! CLI invalidation: per-delta served-output matrices (search × outline).
//!
//! Canonical successor of `invalidation_pass2.rs` (I2) per
//! `tests/catalog/invalidation-cli.md`. The 11 MERGE→matrix/search-delta +
//! matrix/outline-delta tests fuse into TWO tests sharing one 11-row delta
//! table: `matrix_search_delta` pins every delta class × search hit sets,
//! `matrix_outline_delta` pins every delta class × outline symbol sets. (The
//! I2 targeted-refresh MERGE moved to `matrix_targeted_refresh` in
//! `invalidation_parity.rs`.)
//!
//! Every row: fresh two-file fixture → explicit `index` → apply the delta →
//! explicit `index` refresh → assert served output read with
//! `--no-auto-index` (exactly what the refresh wrote). Exit codes, hit
//! counts/sets, outline symbol sets — never message text.

#[path = "invalidation_common.rs"]
mod common;

use ast_sgrep_testkit::{asgrep_bin, parse_stdout};
use common::*;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

// why: one fresh indexed fixture per matrix row, owning the TempDir lifetime.
// WHY area-local: row-scoped two-file context (testkit sessions pin different
// fixture shapes); only this suite drives the delta table — single-suite type.
struct DeltaCtx {
    _dir: tempfile::TempDir,
    bin: PathBuf,
    root: PathBuf,
    root_s: String,
    index_s: String,
}

impl DeltaCtx {
    fn fresh() -> Self {
        let bin = asgrep_bin();
        let (_dir, root, _index, root_s, index_s) = fixture_two_files();
        run_index(&bin, &index_s, &root_s);
        Self {
            _dir,
            bin,
            root,
            root_s,
            index_s,
        }
    }

    fn refresh(&self) -> Value {
        run_index(&self.bin, &self.index_s, &self.root_s)
    }
}

// why: one row per I2 delta class — the mutation plus its per-surface served
// assertions. Both matrix tests drive this table so the delta set cannot
// drift between surfaces. WHY area-local: invalidation-delta-specific —
// single-suite type.
// (`name` is review documentation; assertion values identify a failing row.)
#[allow(dead_code)]
struct DeltaRow {
    name: &'static str,
    mutate: fn(&DeltaCtx),
    check_search: fn(&DeltaCtx, &Value),
    check_outline: fn(&DeltaCtx),
}

// --- row mutations (baseline assertions folded in where I2 had them) ---

fn mutate_add_single(ctx: &DeltaCtx) {
    fs::write(
        ctx.root.join("gamma.rs"),
        "pub fn gamma_new() -> u32 { 3 }\n",
    )
    .expect("add gamma");
}

fn mutate_add_two(ctx: &DeltaCtx) {
    fs::write(ctx.root.join("file_cee.rs"), "pub fn cee_sym() -> u32 { 7 }\n").expect("add cee");
    fs::write(ctx.root.join("file_dee.rs"), "pub fn dee_sym() -> u32 { 8 }\n").expect("add dee");
}

fn mutate_modify_add(ctx: &DeltaCtx) {
    rewrite_with_mtime_bump(
        &ctx.root.join("alpha.rs"),
        "pub fn alpha_one() -> u32 { 1 }\npub fn alpha_two() -> u32 { 2 }\n",
    );
}

fn mutate_modify_remove(ctx: &DeltaCtx) {
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_one"),
        "alpha.rs",
    );
    rewrite_with_mtime_bump(&ctx.root.join("alpha.rs"), "pub fn alpha_two() -> u32 { 2 }\n");
}

fn mutate_modify_rename(ctx: &DeltaCtx) {
    rewrite_with_mtime_bump(
        &ctx.root.join("alpha.rs"),
        "pub fn alpha_renamed() -> u32 { 1 }\n",
    );
}

fn mutate_modify_shift(ctx: &DeltaCtx) {
    // Prepend two lines so the symbol moves from line 1 to line 3. A stale
    // row would still serve line_start == 1.
    rewrite_with_mtime_bump(
        &ctx.root.join("alpha.rs"),
        "// pad line\n\npub fn alpha_one() -> u32 { 1 }\n",
    );
}

fn mutate_delete(ctx: &DeltaCtx) {
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:beta_one"),
        "beta.rs",
    );
    fs::remove_file(ctx.root.join("beta.rs")).expect("delete beta");
}

fn mutate_delete_readd(ctx: &DeltaCtx) {
    fs::remove_file(ctx.root.join("beta.rs")).expect("delete beta");
    ctx.refresh();
    assert_served_nowhere(&run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:beta_one"));
    fs::write(ctx.root.join("beta.rs"), "pub fn beta_one() -> u32 { 2 }\n").expect("re-add beta");
}

fn mutate_rename(ctx: &DeltaCtx) {
    fs::rename(ctx.root.join("beta.rs"), ctx.root.join("beta_moved.rs")).expect("rename beta");
}

fn mutate_rename_edit(ctx: &DeltaCtx) {
    fs::remove_file(ctx.root.join("beta.rs")).expect("remove old beta");
    fs::write(
        ctx.root.join("moved.rs"),
        "pub fn moved_sym() -> u32 { 9 }\n",
    )
    .expect("write moved");
}

fn mutate_noop_rewrite(ctx: &DeltaCtx) {
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_one"),
        "alpha.rs",
    );
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:beta_one"),
        "beta.rs",
    );
    // Same bytes, fresh mtime: even if the mtime fast path fires, the served
    // output must be byte-identical to before the rewrite.
    rewrite_with_mtime_bump(&ctx.root.join("alpha.rs"), "pub fn alpha_one() -> u32 { 1 }\n");
}

// --- row search checks ---

fn search_add_single(ctx: &DeltaCtx, _refreshed: &Value) {
    let added = run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:gamma_new");
    assert_served_only_at(&added, "gamma.rs");
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_one"),
        "alpha.rs",
    );
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:beta_one"),
        "beta.rs",
    );
}

fn search_add_two(ctx: &DeltaCtx, _refreshed: &Value) {
    let cee = run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:cee_sym");
    assert_served_only_at(&cee, "file_cee.rs");
    assert_eq!(hits_in(&cee, "file_dee.rs"), 0, "cross-file leak: {cee}");
    let dee = run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:dee_sym");
    assert_served_only_at(&dee, "file_dee.rs");
    assert_eq!(hits_in(&dee, "file_cee.rs"), 0, "cross-file leak: {dee}");
}

fn search_modify_add(ctx: &DeltaCtx, _refreshed: &Value) {
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_two"),
        "alpha.rs",
    );
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_one"),
        "alpha.rs",
    );
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:beta_one"),
        "beta.rs",
    );
}

fn search_modify_remove(ctx: &DeltaCtx, _refreshed: &Value) {
    assert_served_nowhere(&run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_one"));
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_two"),
        "alpha.rs",
    );
}

fn search_modify_rename(ctx: &DeltaCtx, _refreshed: &Value) {
    assert_served_nowhere(&run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_one"));
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_renamed"),
        "alpha.rs",
    );
}

fn search_modify_shift(ctx: &DeltaCtx, _refreshed: &Value) {
    let found = run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_one");
    assert_served_only_at(&found, "alpha.rs");
    assert_eq!(
        found["hits"][0]["line_start"], 3,
        "hit must follow the symbol to its new line: {found}"
    );
}

fn search_delete(ctx: &DeltaCtx, _refreshed: &Value) {
    assert_served_nowhere(&run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:beta_one"));
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_one"),
        "alpha.rs",
    );
}

fn search_delete_readd(ctx: &DeltaCtx, _refreshed: &Value) {
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:beta_one"),
        "beta.rs",
    );
}

fn search_rename(ctx: &DeltaCtx, _refreshed: &Value) {
    let found = run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:beta_one");
    assert_served_only_at(&found, "beta_moved.rs");
    assert_eq!(hits_in(&found, "beta.rs"), 0, "stale old-path hit: {found}");
}

fn search_rename_edit(ctx: &DeltaCtx, _refreshed: &Value) {
    assert_served_nowhere(&run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:beta_one"));
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:moved_sym"),
        "moved.rs",
    );
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_one"),
        "alpha.rs",
    );
}

fn search_noop_rewrite(ctx: &DeltaCtx, refreshed: &Value) {
    assert_eq!(refreshed["exit_code"], 0, "{refreshed}");
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:alpha_one"),
        "alpha.rs",
    );
    assert_served_only_at(
        &run_search(&ctx.bin, &ctx.index_s, &ctx.root_s, "word:beta_one"),
        "beta.rs",
    );
}

// --- row outline checks ---

fn outline_add_single(ctx: &DeltaCtx) {
    let outline = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "gamma.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["ok"], true, "{outline_value}");
    assert_eq!(outline_value["count"], 1, "{outline_value}");
    assert_eq!(outline_names(&outline_value), vec!["gamma_new"], "{outline_value}");
}

fn outline_add_two(ctx: &DeltaCtx) {
    // Natural outline facet for the ADD×2 row (I2 asserted search only here):
    // both added paths serve their own symbol.
    for (rel, name) in [("file_cee.rs", "cee_sym"), ("file_dee.rs", "dee_sym")] {
        let outline = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, rel);
        assert_eq!(outline.status.code(), Some(0), "{rel}");
        let outline_value = parse_stdout(&outline);
        assert_eq!(outline_value["count"], 1, "{outline_value}");
        assert_eq!(outline_names(&outline_value), vec![name], "{outline_value}");
    }
}

fn outline_modify_add(ctx: &DeltaCtx) {
    let outline = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "alpha.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 2, "{outline_value}");
    assert_eq!(
        outline_names(&outline_value),
        vec!["alpha_one", "alpha_two"],
        "{outline_value}"
    );
}

fn outline_modify_remove(ctx: &DeltaCtx) {
    let outline = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "alpha.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 1, "{outline_value}");
    assert_eq!(outline_names(&outline_value), vec!["alpha_two"], "{outline_value}");
}

fn outline_modify_rename(ctx: &DeltaCtx) {
    let outline = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "alpha.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 1, "{outline_value}");
    assert_eq!(
        outline_names(&outline_value),
        vec!["alpha_renamed"],
        "{outline_value}"
    );
}

fn outline_modify_shift(ctx: &DeltaCtx) {
    let outline = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "alpha.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(
        outline_value["symbols"][0]["line_start"], 3,
        "outline must follow the symbol to its new line: {outline_value}"
    );
}

fn outline_delete(ctx: &DeltaCtx) {
    let gone = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "beta.rs");
    assert_eq!(gone.status.code(), Some(2), "deleted path must refuse");
    assert_eq!(parse_stdout(&gone)["ok"], false);
    let kept = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "alpha.rs");
    assert_eq!(kept.status.code(), Some(0));
}

fn outline_delete_readd(ctx: &DeltaCtx) {
    let outline = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "beta.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 1, "{outline_value}");
    assert_eq!(outline_names(&outline_value), vec!["beta_one"], "{outline_value}");
}

fn outline_rename(ctx: &DeltaCtx) {
    let stale = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "beta.rs");
    assert_eq!(stale.status.code(), Some(2), "old path must refuse");
    assert_eq!(parse_stdout(&stale)["ok"], false);
    let moved = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "beta_moved.rs");
    assert_eq!(moved.status.code(), Some(0));
    let moved_value = parse_stdout(&moved);
    assert_eq!(moved_value["count"], 1, "{moved_value}");
    assert_eq!(outline_names(&moved_value), vec!["beta_one"], "{moved_value}");
}

fn outline_rename_edit(ctx: &DeltaCtx) {
    let stale = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "beta.rs");
    assert_eq!(stale.status.code(), Some(2), "old path must refuse");
    // Natural outline facet for the new path (I2 asserted the refusal only).
    let moved = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "moved.rs");
    assert_eq!(moved.status.code(), Some(0));
    let moved_value = parse_stdout(&moved);
    assert_eq!(moved_value["count"], 1, "{moved_value}");
    assert_eq!(outline_names(&moved_value), vec!["moved_sym"], "{moved_value}");
}

fn outline_noop_rewrite(ctx: &DeltaCtx) {
    let outline = run_outline(&ctx.bin, &ctx.index_s, &ctx.root_s, "alpha.rs");
    assert_eq!(outline.status.code(), Some(0));
    let outline_value = parse_stdout(&outline);
    assert_eq!(outline_value["count"], 1, "{outline_value}");
    assert_eq!(outline_names(&outline_value), vec!["alpha_one"], "{outline_value}");
}

static DELTA_ROWS: &[DeltaRow] = &[
    DeltaRow {
        name: "add-single",
        mutate: mutate_add_single,
        check_search: search_add_single,
        check_outline: outline_add_single,
    },
    DeltaRow {
        name: "add-two",
        mutate: mutate_add_two,
        check_search: search_add_two,
        check_outline: outline_add_two,
    },
    DeltaRow {
        name: "modify-add",
        mutate: mutate_modify_add,
        check_search: search_modify_add,
        check_outline: outline_modify_add,
    },
    DeltaRow {
        name: "modify-remove",
        mutate: mutate_modify_remove,
        check_search: search_modify_remove,
        check_outline: outline_modify_remove,
    },
    DeltaRow {
        name: "modify-rename",
        mutate: mutate_modify_rename,
        check_search: search_modify_rename,
        check_outline: outline_modify_rename,
    },
    DeltaRow {
        name: "modify-shift",
        mutate: mutate_modify_shift,
        check_search: search_modify_shift,
        check_outline: outline_modify_shift,
    },
    DeltaRow {
        name: "delete",
        mutate: mutate_delete,
        check_search: search_delete,
        check_outline: outline_delete,
    },
    DeltaRow {
        name: "delete-readd",
        mutate: mutate_delete_readd,
        check_search: search_delete_readd,
        check_outline: outline_delete_readd,
    },
    DeltaRow {
        name: "rename",
        mutate: mutate_rename,
        check_search: search_rename,
        check_outline: outline_rename,
    },
    DeltaRow {
        name: "rename-edit",
        mutate: mutate_rename_edit,
        check_search: search_rename_edit,
        check_outline: outline_rename_edit,
    },
    DeltaRow {
        name: "noop-rewrite",
        mutate: mutate_noop_rewrite,
        check_search: search_noop_rewrite,
        check_outline: outline_noop_rewrite,
    },
];

/// INTENT: per-surface delta matrix — after one explicit refresh, every delta
/// class (ADD×2, MODIFY×4, DELETE×2, RENAME×2, NOOP) is reflected EXACTLY in
/// served search hit sets per path (counts, partitioning, line starts).
/// KILLS: add-omission | cross-file-hit-leak | modify-rewrite-skipped |
/// stale-token-retention | token-swap-incompleteness | position-staleness |
/// prune-omission | tombstone-persistence | old-path-retention |
/// new-path-omission | rename/edit-confusion | hash-fallback-corruption.
/// ABSORBS: the search facets of all 11 I2 MERGE→matrix/search-delta tests
/// (`i2_delta_add_single_file…` through `i2_delta_noop_rewrite…`).
#[test]
fn matrix_search_delta() {
    assert_eq!(DELTA_ROWS.len(), 11, "the I2 delta set is 11 rows");
    for row in DELTA_ROWS {
        let ctx = DeltaCtx::fresh();
        (row.mutate)(&ctx);
        let refreshed = ctx.refresh();
        (row.check_search)(&ctx, &refreshed);
    }
}

/// INTENT: per-surface delta matrix — after one explicit refresh, every delta
/// class is reflected EXACTLY in served outline symbol sets (counts, names,
/// line starts, deleted-path refusal codes).
/// KILLS: same delta-confusion family as `matrix_search_delta`, observed via
/// outline (symbol-set drift, refusal-missing, line-staleness).
/// ABSORBS: the outline facets of all 11 I2 MERGE→matrix/outline-delta tests,
/// plus the natural outline facets for the ADD×2 and RENAME+EDIT rows (I2
/// pinned search/refusal only there).
#[test]
fn matrix_outline_delta() {
    assert_eq!(DELTA_ROWS.len(), 11, "the I2 delta set is 11 rows");
    for row in DELTA_ROWS {
        let ctx = DeltaCtx::fresh();
        (row.mutate)(&ctx);
        ctx.refresh();
        (row.check_outline)(&ctx);
    }
}

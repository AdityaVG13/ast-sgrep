//! I2 invalidation-contract oracles: FILE-DELTA discriminants.
//!
//! Scope: each filesystem delta class is reflected EXACTLY in search-visible
//! state after an explicit refresh (`index_all`, or `update_paths` where the
//! test names it): ADD (the new file's symbols are searchable, siblings
//! untouched), MODIFY (old symbols gone, new present, siblings untouched),
//! DELETE (its hits vanish, siblings remain), RENAME (old-path hits gone,
//! new-path hits present), NOOP (a byte-identical rewrite leaves search
//! results identical). Every test name carries its delta class.
//!
//! Non-duplication vs `invalidation_pass1.rs`: I1 pins staleness DETECTION
//! (writer-generation epochs, `index_data_version` monotonicity, mtime-trust
//! gates, schema-version refusal, cache routing, stored-count status).
//! These oracles assert search-visible CONSEQUENCES ONLY (def / literal /
//! caller hit sets and per-path counts, before/after equality, emptiness,
//! and path exclusivity) and never inspect message text, versions, or epochs.
//!
//! Determinism: content-change detection never relies on filesystem mtime
//! granularity — MODIFY/NOOP tests forge explicit whole-second mtimes with
//! read-back preconditions (same technique as I1). No wall-clock sleeps.
//! Hermeticity: every open uses an explicit `index_path` outside the corpus
//! root, so no test depends on ambient `ASGREP_*` routing.

use ast_sgrep_core::{
    HitKind, IndexOptions, IndexStats, IndexStore, Indexer, SearchOptions, Searcher,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

/// Fixed whole-second stamps (nanos = 0 survives every filesystem timestamp
/// granularity). MODIFY/NOOP tests forge T0 at creation and T1 after the
/// rewrite so change detection is deterministic.
const WHOLE_SECOND_T0: u64 = 1_700_000_000;
const WHOLE_SECOND_T1: u64 = 1_700_003_600;

fn set_mtime_checked(path: &Path, secs: u64) {
    let time = UNIX_EPOCH + Duration::new(secs, 0);
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(time)
        .unwrap();
    assert_eq!(
        path.metadata().unwrap().modified().unwrap(),
        time,
        "filesystem must store the forged mtime exactly"
    );
}

struct Fx {
    _corpus: tempfile::TempDir,
    _index: tempfile::TempDir,
    root: PathBuf,
    db: PathBuf,
}

impl Fx {
    fn new() -> Self {
        let corpus = tempfile::tempdir().unwrap();
        let index = tempfile::tempdir().unwrap();
        let root = corpus.path().to_path_buf();
        std::fs::create_dir_all(root.join("src")).unwrap();
        let db = index.path().join("index.db");
        Self {
            _corpus: corpus,
            _index: index,
            root,
            db,
        }
    }

    fn write(&self, rel: &str, content: &str) -> PathBuf {
        let abs = self.root.join(rel);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&abs, content).unwrap();
        abs
    }

    fn reindex(&self) -> IndexStats {
        Indexer::new(IndexOptions {
            root: self.root.clone(),
            index_path: Some(self.db.clone()),
            use_tantivy: false,
            embed_semantic: false,
            ..IndexOptions::default()
        })
        .unwrap()
        .index_all()
        .unwrap()
    }

    fn searcher(&self) -> Searcher {
        Searcher::new(SearchOptions {
            root: self.root.clone(),
            index_path: Some(self.db.clone()),
            limit: 64,
            use_embed: false,
            ..SearchOptions::default()
        })
        .unwrap()
    }

    /// Sorted `file` values of Def hits whose symbol is exactly `symbol`.
    fn def_files(&self, symbol: &str) -> Vec<String> {
        let query = format!("defs:{symbol}");
        let mut files: Vec<String> = self
            .searcher()
            .search(&query)
            .unwrap()
            .hits
            .iter()
            .filter(|h| h.kind == HitKind::Def && h.symbol.as_deref() == Some(symbol))
            .map(|h| h.file.clone())
            .collect();
        files.sort();
        files
    }

    /// Sorted `(file, line_start)` spots for a `literal:` query (all hits).
    fn literal_spots(&self, token: &str) -> Vec<(String, u32)> {
        let query = format!("literal:{token}");
        let mut spots: Vec<(String, u32)> = self
            .searcher()
            .search(&query)
            .unwrap()
            .hits
            .iter()
            .map(|h| (h.file.clone(), h.line_start))
            .collect();
        spots.sort();
        spots
    }

    /// Sorted `file` values of Caller hits whose callee is exactly `callee`.
    fn caller_files(&self, callee: &str) -> Vec<String> {
        let query = format!("callers:{callee}");
        let mut files: Vec<String> = self
            .searcher()
            .search(&query)
            .unwrap()
            .hits
            .iter()
            .filter(|h| h.kind == HitKind::Caller && h.callee.as_deref() == Some(callee))
            .map(|h| h.file.clone())
            .collect();
        files.sort();
        files
    }

    fn stored_counts(&self) -> (usize, usize, usize) {
        let status = IndexStore::open(&self.root, Some(&self.db))
            .unwrap()
            .status()
            .unwrap();
        (status.file_count, status.line_count, status.symbol_count)
    }
}

fn count_in(spots: &[(String, u32)], file: &str) -> usize {
    spots.iter().filter(|(f, _)| f == file).count()
}

// ---- ADD: new file searchable, siblings untouched ----

#[test]
fn add_file_makes_its_symbols_searchable() {
    let fx = Fx::new();
    fx.write("src/a.rs", "fn i2_alpha_one() {}\n");
    assert_eq!(fx.reindex().files_indexed, 1);
    let sibling_before = fx.def_files("i2_alpha_one");
    assert!(!sibling_before.is_empty(), "precondition: initial symbol indexed");

    fx.write("src/b.rs", "fn i2_beta_two() {}\n");
    assert_eq!(fx.reindex().files_indexed, 1);

    let added = fx.def_files("i2_beta_two");
    assert!(!added.is_empty(), "ADD must make the new file's symbol searchable");
    assert!(
        added.iter().all(|f| f == "src/b.rs"),
        "added symbol hits only under the new path: {added:?}"
    );
    assert_eq!(
        fx.def_files("i2_alpha_one"),
        sibling_before,
        "ADD must leave sibling def hits untouched"
    );
}

#[test]
fn add_file_leaves_sibling_hit_sets_untouched() {
    let fx = Fx::new();
    fx.write("src/a.rs", "fn i2_keep_me() { let _t = \"tokkeepzz\"; }\n");
    assert_eq!(fx.reindex().files_indexed, 1);
    let defs_before = fx.def_files("i2_keep_me");
    let lits_before = fx.literal_spots("tokkeepzz");
    assert!(!defs_before.is_empty(), "precondition: sibling def indexed");
    assert!(!lits_before.is_empty(), "precondition: sibling literal indexed");

    fx.write("src/b.rs", "fn i2_newcomer() { let _t = \"toknewzz\"; }\n");
    assert_eq!(fx.reindex().files_indexed, 1);

    assert_eq!(
        fx.def_files("i2_keep_me"),
        defs_before,
        "ADD must not touch sibling def hits"
    );
    assert_eq!(
        fx.literal_spots("tokkeepzz"),
        lits_before,
        "ADD must not touch sibling literal hits"
    );
    assert!(!fx.def_files("i2_newcomer").is_empty());
    assert!(!fx.literal_spots("toknewzz").is_empty());
}

#[test]
fn add_file_with_caller_edge_exposes_callers() {
    let fx = Fx::new();
    fx.write("src/a.rs", "fn i2_add_callee() {}\n");
    assert_eq!(fx.reindex().files_indexed, 1);
    assert!(fx.caller_files("i2_add_callee").is_empty());

    fx.write("src/b.rs", "fn i2_add_caller() { i2_add_callee(); }\n");
    assert_eq!(fx.reindex().files_indexed, 1);

    let callers = fx.caller_files("i2_add_callee");
    assert!(!callers.is_empty(), "ADD must expose the new file's caller edge");
    assert!(
        callers.iter().all(|f| f == "src/b.rs"),
        "new caller hits only under the new path: {callers:?}"
    );
    let defs = fx.def_files("i2_add_callee");
    assert!(!defs.is_empty() && defs.iter().all(|f| f == "src/a.rs"));
}

// ---- MODIFY: old gone, new present, siblings untouched ----

#[test]
fn modify_file_retires_old_symbol_publishes_new() {
    let fx = Fx::new();
    let abs = fx.write("src/a.rs", "fn i2_retire_old() {}\n");
    set_mtime_checked(&abs, WHOLE_SECOND_T0);
    assert_eq!(fx.reindex().files_indexed, 1);
    assert!(!fx.def_files("i2_retire_old").is_empty());

    fx.write("src/a.rs", "fn i2_fresh_new() {}\n");
    set_mtime_checked(&abs, WHOLE_SECOND_T1);
    assert_eq!(fx.reindex().files_indexed, 1);

    assert!(
        fx.def_files("i2_retire_old").is_empty(),
        "MODIFY must retire the old symbol"
    );
    let fresh = fx.def_files("i2_fresh_new");
    assert!(!fresh.is_empty(), "MODIFY must publish the new symbol");
    assert!(fresh.iter().all(|f| f == "src/a.rs"));
}

#[test]
fn modify_file_sibling_defs_and_literals_untouched() {
    let fx = Fx::new();
    let abs = fx.write(
        "src/a.rs",
        "fn i2_churn_old() { let _t = \"tokchurnold\"; }\n",
    );
    set_mtime_checked(&abs, WHOLE_SECOND_T0);
    fx.write("src/b.rs", "fn i2_stable_sib() { let _t = \"tokstablezz\"; }\n");
    assert_eq!(fx.reindex().files_indexed, 2);
    let sib_defs = fx.def_files("i2_stable_sib");
    let sib_lits = fx.literal_spots("tokstablezz");
    assert!(!sib_defs.is_empty() && !sib_lits.is_empty());

    fx.write("src/a.rs", "fn i2_churn_new() { let _t = \"tokchurnnew\"; }\n");
    set_mtime_checked(&abs, WHOLE_SECOND_T1);
    assert_eq!(fx.reindex().files_indexed, 1);

    assert_eq!(
        fx.def_files("i2_stable_sib"),
        sib_defs,
        "MODIFY must not touch sibling def hits"
    );
    assert_eq!(
        fx.literal_spots("tokstablezz"),
        sib_lits,
        "MODIFY must not touch sibling literal hits"
    );
    assert!(fx.def_files("i2_churn_old").is_empty());
    assert!(fx.literal_spots("tokchurnold").is_empty());
    assert!(!fx.def_files("i2_churn_new").is_empty());
    assert!(!fx.literal_spots("tokchurnnew").is_empty());
}

#[test]
fn modify_file_literal_counts_exact_per_path() {
    let fx = Fx::new();
    let abs = fx.write(
        "src/a.rs",
        "fn i2_cnt_a() {}\n// tokcntzz alpha\n// tokcntzz beta\n",
    );
    set_mtime_checked(&abs, WHOLE_SECOND_T0);
    fx.write("src/b.rs", "fn i2_cnt_b() {}\n// tokcntzz gamma\n");
    assert_eq!(fx.reindex().files_indexed, 2);
    let before = fx.literal_spots("tokcntzz");
    assert!(count_in(&before, "src/a.rs") > 0);
    assert!(count_in(&before, "src/b.rs") > 0);

    fx.write("src/a.rs", "fn i2_cnt_a() {}\n// scrubbed line one\n// scrubbed line two\n");
    set_mtime_checked(&abs, WHOLE_SECOND_T1);
    assert_eq!(fx.reindex().files_indexed, 1);

    let after = fx.literal_spots("tokcntzz");
    assert_eq!(
        count_in(&after, "src/a.rs"),
        0,
        "MODIFY must drop the edited file's stale literal hits"
    );
    assert_eq!(
        count_in(&after, "src/b.rs"),
        count_in(&before, "src/b.rs"),
        "MODIFY must preserve the sibling's literal count"
    );
    assert_eq!(
        after.len(),
        count_in(&before, "src/b.rs"),
        "MODIFY total equals the untouched remainder"
    );
}

#[test]
fn modify_file_via_update_paths_reflects_delta() {
    let fx = Fx::new();
    let abs = fx.write("src/a.rs", "fn i2_up_old() {}\n");
    set_mtime_checked(&abs, WHOLE_SECOND_T0);
    assert_eq!(fx.reindex().files_indexed, 1);
    assert!(!fx.def_files("i2_up_old").is_empty());

    fx.write("src/a.rs", "fn i2_up_new() {}\n");
    set_mtime_checked(&abs, WHOLE_SECOND_T1);
    let stats = Indexer::new(IndexOptions {
        root: fx.root.clone(),
        index_path: Some(fx.db.clone()),
        use_tantivy: false,
        embed_semantic: false,
        ..IndexOptions::default()
    })
    .unwrap()
    .update_paths(std::slice::from_ref(&abs))
    .unwrap();
    assert_eq!(stats.files_indexed, 1);

    assert!(
        fx.def_files("i2_up_old").is_empty(),
        "incremental MODIFY must retire the old symbol"
    );
    let fresh = fx.def_files("i2_up_new");
    assert!(!fresh.is_empty(), "incremental MODIFY must publish the new symbol");
    assert!(fresh.iter().all(|f| f == "src/a.rs"));
}

// ---- DELETE: its hits vanish, siblings remain ----

#[test]
fn delete_file_defs_vanish_sibling_defs_remain() {
    let fx = Fx::new();
    fx.write("src/a.rs", "fn i2_doomed_sym() {}\n");
    fx.write("src/b.rs", "fn i2_survivor() {}\n");
    assert_eq!(fx.reindex().files_indexed, 2);
    assert!(!fx.def_files("i2_doomed_sym").is_empty());
    let survivor_before = fx.def_files("i2_survivor");
    assert!(!survivor_before.is_empty());

    std::fs::remove_file(fx.root.join("src/a.rs")).unwrap();
    assert_eq!(fx.reindex().files_removed, 1);

    assert!(
        fx.def_files("i2_doomed_sym").is_empty(),
        "DELETE must vanish the removed file's defs"
    );
    assert_eq!(
        fx.def_files("i2_survivor"),
        survivor_before,
        "DELETE must leave the surviving sibling's defs untouched"
    );
}

#[test]
fn delete_file_prunes_literal_and_caller_hits() {
    let fx = Fx::new();
    fx.write("src/a.rs", "fn i2_sole_callee() {}\n");
    fx.write(
        "src/b.rs",
        "fn i2_gone_caller() { i2_sole_callee(); let _t = \"tokgonezz\"; }\n",
    );
    assert_eq!(fx.reindex().files_indexed, 2);
    assert!(!fx.caller_files("i2_sole_callee").is_empty());
    assert!(!fx.literal_spots("tokgonezz").is_empty());

    std::fs::remove_file(fx.root.join("src/b.rs")).unwrap();
    assert_eq!(fx.reindex().files_removed, 1);

    assert!(
        fx.caller_files("i2_sole_callee").is_empty(),
        "DELETE must prune the removed file's caller hits"
    );
    assert!(
        fx.literal_spots("tokgonezz").is_empty(),
        "DELETE must prune the removed file's literal hits"
    );
    let defs = fx.def_files("i2_sole_callee");
    assert!(!defs.is_empty() && defs.iter().all(|f| f == "src/a.rs"));
}

// ---- RENAME: old-path hits gone, new-path hits present ----

#[test]
fn rename_file_moves_hits_to_new_path() {
    let fx = Fx::new();
    fx.write("src/old_name.rs", "fn i2_moved_sym() { let _t = \"tokmovezz\"; }\n");
    assert_eq!(fx.reindex().files_indexed, 1);
    let defs_before = fx.def_files("i2_moved_sym");
    assert!(!defs_before.is_empty());
    assert!(defs_before.iter().all(|f| f == "src/old_name.rs"));

    std::fs::rename(
        fx.root.join("src/old_name.rs"),
        fx.root.join("src/new_name.rs"),
    )
    .unwrap();
    let stats = fx.reindex();
    assert_eq!(stats.files_removed, 1);
    assert_eq!(stats.files_indexed, 1);

    let defs = fx.def_files("i2_moved_sym");
    assert!(!defs.is_empty(), "RENAME must keep the symbol searchable");
    assert!(
        defs.iter().all(|f| f == "src/new_name.rs"),
        "no def hits under the old path: {defs:?}"
    );
    let lits = fx.literal_spots("tokmovezz");
    assert!(!lits.is_empty(), "RENAME must keep literals searchable");
    assert!(
        lits.iter().all(|(f, _)| f == "src/new_name.rs"),
        "no literal hits under the old path: {lits:?}"
    );
}

#[test]
fn rename_file_preserves_sibling_hits_and_total_counts() {
    let fx = Fx::new();
    fx.write("src/a.rs", "fn i2_stay_put() { let _t = \"tokstayzz\"; }\n");
    fx.write("src/b.rs", "fn i2_roamer() { let _t = \"tokroamzz\"; }\n");
    assert_eq!(fx.reindex().files_indexed, 2);
    let sib_defs = fx.def_files("i2_stay_put");
    let sib_lits = fx.literal_spots("tokstayzz");
    let roam_before = fx.literal_spots("tokroamzz");
    assert!(!sib_defs.is_empty() && !sib_lits.is_empty() && !roam_before.is_empty());

    std::fs::rename(fx.root.join("src/b.rs"), fx.root.join("src/z.rs")).unwrap();
    fx.reindex();

    assert_eq!(
        fx.def_files("i2_stay_put"),
        sib_defs,
        "RENAME must not touch sibling def hits"
    );
    assert_eq!(
        fx.literal_spots("tokstayzz"),
        sib_lits,
        "RENAME must not touch sibling literal hits"
    );
    let roam_after = fx.literal_spots("tokroamzz");
    assert_eq!(
        roam_after.len(),
        roam_before.len(),
        "RENAME preserves the moved file's hit count"
    );
    assert!(roam_after.iter().all(|(f, _)| f == "src/z.rs"));
    assert_eq!(
        roam_after.iter().map(|(_, l)| l).collect::<Vec<_>>(),
        roam_before.iter().map(|(_, l)| l).collect::<Vec<_>>(),
        "RENAME preserves line numbers"
    );
    let roam_defs = fx.def_files("i2_roamer");
    assert!(!roam_defs.is_empty() && roam_defs.iter().all(|f| f == "src/z.rs"));
}

// ---- NOOP: byte-identical rewrite leaves search results identical ----

#[test]
fn noop_rewrite_same_bytes_search_results_identical() {
    let fx = Fx::new();
    let abs = fx.write("src/a.rs", "fn i2_noop_one() { let _t = \"toknoopzz\"; }\n");
    set_mtime_checked(&abs, WHOLE_SECOND_T0);
    fx.write("src/b.rs", "fn i2_noop_two() { let _t = \"toknoopzz\"; }\n");
    assert_eq!(fx.reindex().files_indexed, 2);
    let defs_a = fx.def_files("i2_noop_one");
    let defs_b = fx.def_files("i2_noop_two");
    let lits = fx.literal_spots("toknoopzz");
    let counts = fx.stored_counts();
    assert!(!defs_a.is_empty() && !defs_b.is_empty());
    assert!(lits.iter().any(|(f, _)| f == "src/a.rs"));
    assert!(lits.iter().any(|(f, _)| f == "src/b.rs"));

    // Byte-identical rewrite with a NEWER forged mtime: pushes past the mtime
    // fast path so the content comparison must recognize identical bytes.
    fx.write("src/a.rs", "fn i2_noop_one() { let _t = \"toknoopzz\"; }\n");
    set_mtime_checked(&abs, WHOLE_SECOND_T1);
    fx.reindex();

    assert_eq!(
        fx.def_files("i2_noop_one"),
        defs_a,
        "NOOP rewrite must leave def hits identical"
    );
    assert_eq!(
        fx.def_files("i2_noop_two"),
        defs_b,
        "NOOP rewrite must leave sibling def hits identical"
    );
    assert_eq!(
        fx.literal_spots("toknoopzz"),
        lits,
        "NOOP rewrite must leave literal hits identical"
    );
    assert_eq!(
        fx.stored_counts(),
        counts,
        "NOOP rewrite must leave stored row counts identical"
    );
}

#[test]
fn noop_rewrite_same_bytes_caller_graph_stable() {
    let fx = Fx::new();
    fx.write("src/a.rs", "fn i2_callee_zz() {}\n");
    let abs_b = fx.write("src/b.rs", "fn i2_caller_zz() { i2_callee_zz(); }\n");
    set_mtime_checked(&abs_b, WHOLE_SECOND_T0);
    assert_eq!(fx.reindex().files_indexed, 2);
    let callers_before = fx.caller_files("i2_callee_zz");
    let defs_before = fx.def_files("i2_callee_zz");
    assert!(!callers_before.is_empty(), "precondition: caller edge indexed");
    assert!(callers_before.iter().all(|f| f == "src/b.rs"));
    assert!(!defs_before.is_empty());

    fx.write("src/b.rs", "fn i2_caller_zz() { i2_callee_zz(); }\n");
    set_mtime_checked(&abs_b, WHOLE_SECOND_T1);
    fx.reindex();

    assert_eq!(
        fx.caller_files("i2_callee_zz"),
        callers_before,
        "NOOP rewrite must leave caller hits identical"
    );
    assert_eq!(
        fx.def_files("i2_callee_zz"),
        defs_before,
        "NOOP rewrite must leave def hits identical"
    );
}

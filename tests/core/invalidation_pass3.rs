//! I3 invalidation-contract oracles: REBUILD-PARITY metamorphic relations.
//!
//! Scope: RELATIONS between refresh strategies, not single-delta consequences.
//! Every test asserts that two ways of reaching the same filesystem state
//! produce the same search-visible state:
//!
//! - incremental-refresh parity: `update_paths` deltas == fresh full rebuild
//!   (identical hit sets over a fixed query battery, identical stored counts).
//! - delta order independence: the same delta multiset applied in different
//!   orders converges to identical hit sets, counts, and generation.
//! - refresh idempotence: a second identical refresh is a mutation-free noop
//!   (identical battery, generation, and counts).
//! - generation monotonicity: `index_data_version` never moves backward; a
//!   refresh mutating exactly one file bumps it by exactly one; a bulk
//!   refresh bumps it by exactly the mutated-file count.
//! - untouched-file stability: files outside the delta set keep byte-identical
//!   stored rows (files/lines/symbols/callers/imports/pattern_nodes content,
//!   surrogate ids excluded) and identical hit subsets.
//!
//! Non-duplication vs `invalidation_pass1.rs` / `invalidation_pass2.rs`: I1
//! pins staleness DETECTION discriminants (epochs, mtime gates, schema
//! refusal, routing); I2 pins single-delta search-visible CONSEQUENCES
//! (ADD/MODIFY/DELETE/RENAME/NOOP hit sets). These oracles assert only
//! cross-strategy EQUALITY (battery vectors, count tuples, i64 generations,
//! row dumps) and never inspect message text.
//!
//! Determinism: MODIFY deltas forge explicit whole-second mtimes with
//! read-back preconditions (same technique as I1/I2), so `index_all` change
//! detection never depends on filesystem timestamp granularity. No sleeps.
//! Hermeticity: every open uses an explicit `index_path` outside the corpus
//! root, so no test depends on ambient `ASGREP_*` routing.

use ast_sgrep_core::index::WatchUpdateStats;
use ast_sgrep_core::{
    IndexOptions, IndexStats, IndexStore, Indexer, SearchOptions, Searcher,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

/// Fixed whole-second stamps (nanos = 0 survives every filesystem timestamp
/// granularity). MODIFY tests forge T0 at creation and T1 after the rewrite.
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

/// One battery hit: (query, kind, file, line_start, line_end, symbol, caller, callee).
/// Scores and excerpts are excluded: the relation is structural identity.
type HitTuple = (String, String, String, u32, u32, String, String, String);

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

    fn indexer(&self) -> Indexer {
        Indexer::new(IndexOptions {
            root: self.root.clone(),
            index_path: Some(self.db.clone()),
            use_tantivy: false,
            embed_semantic: false,
            ..IndexOptions::default()
        })
        .unwrap()
    }

    fn reindex(&self) -> IndexStats {
        self.indexer().index_all().unwrap()
    }

    fn update(&self, paths: &[PathBuf]) -> WatchUpdateStats {
        self.indexer().update_paths(paths).unwrap()
    }

    fn searcher(&self) -> Searcher {
        Searcher::new(SearchOptions {
            root: self.root.clone(),
            index_path: Some(self.db.clone()),
            limit: 256,
            use_embed: false,
            ..SearchOptions::default()
        })
        .unwrap()
    }

    /// Sorted hit tuples for a fixed query battery.
    fn battery(&self, queries: &[&str]) -> Vec<HitTuple> {
        let searcher = self.searcher();
        let mut out = Vec::new();
        for query in queries {
            for h in searcher.search(query).unwrap().hits.iter() {
                out.push((
                    query.to_string(),
                    h.kind.as_str().to_string(),
                    h.file.clone(),
                    h.line_start,
                    h.line_end,
                    h.symbol.clone().unwrap_or_default(),
                    h.caller.clone().unwrap_or_default(),
                    h.callee.clone().unwrap_or_default(),
                ));
            }
        }
        out.sort();
        out
    }

    fn generation(&self) -> i64 {
        IndexStore::open(&self.root, Some(&self.db))
            .unwrap()
            .index_data_version()
            .unwrap()
    }

    fn stored_counts(&self) -> (usize, usize, usize, usize, usize) {
        let status = IndexStore::open(&self.root, Some(&self.db))
            .unwrap()
            .status()
            .unwrap();
        (
            status.file_count,
            status.line_count,
            status.symbol_count,
            status.caller_count,
            status.import_count,
        )
    }

    /// Canonical content rows for one stored file, joined by path and ordered.
    /// Surrogate integer ids are excluded; everything else is byte-compared.
    fn file_rows(&self, rel: &str) -> Vec<String> {
        let store = IndexStore::open(&self.root, Some(&self.db)).unwrap();
        let conn = store.connection();
        let mut rows = Vec::new();
        rows.push(
            conn.query_row(
                "SELECT path, language, mtime_secs, mtime_nanos, content_hash, depth_truncated
                 FROM files WHERE path = ?1",
                [rel],
                |row| {
                    let path: String = row.get(0)?;
                    let lang: Option<String> = row.get(1)?;
                    let secs: i64 = row.get(2)?;
                    let nanos: i64 = row.get(3)?;
                    let hash: String = row.get(4)?;
                    let trunc: i64 = row.get(5)?;
                    Ok(format!("files|{path}|{lang:?}|{secs}|{nanos}|{hash}|{trunc}"))
                },
            )
            .unwrap(),
        );
        let mut lines: Vec<String> = conn
            .prepare(
                "SELECT l.line_no, l.content FROM lines l JOIN files f ON f.id = l.file_id
                 WHERE f.path = ?1 ORDER BY l.line_no",
            )
            .unwrap()
            .query_map([rel], |row| {
                let no: i64 = row.get(0)?;
                let content: String = row.get(1)?;
                Ok(format!("lines|{no}|{content}"))
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        rows.append(&mut lines);
        let mut symbols: Vec<String> = conn
            .prepare(
                "SELECT s.name, s.kind, s.line_start, s.line_end, s.byte_start, s.byte_end
                 FROM symbols s JOIN files f ON f.id = s.file_id
                 WHERE f.path = ?1 ORDER BY s.name, s.line_start, s.byte_start",
            )
            .unwrap()
            .query_map([rel], |row| {
                let (name, kind): (String, String) = (row.get(0)?, row.get(1)?);
                let (ls, le, bs, be): (i64, i64, i64, i64) =
                    (row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?);
                Ok(format!("symbols|{name}|{kind}|{ls}|{le}|{bs}|{be}"))
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        rows.append(&mut symbols);
        let mut callers: Vec<String> = conn
            .prepare(
                "SELECT c.caller, c.callee, c.line_no, c.byte_start, c.byte_end
                 FROM callers c JOIN files f ON f.id = c.file_id
                 WHERE f.path = ?1 ORDER BY c.caller, c.callee, c.line_no",
            )
            .unwrap()
            .query_map([rel], |row| {
                let (caller, callee): (String, String) = (row.get(0)?, row.get(1)?);
                let (no, bs, be): (i64, i64, i64) = (row.get(2)?, row.get(3)?, row.get(4)?);
                Ok(format!("callers|{caller}|{callee}|{no}|{bs}|{be}"))
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        rows.append(&mut callers);
        let mut imports: Vec<String> = conn
            .prepare(
                "SELECT i.module_path, i.line_no FROM imports i JOIN files f ON f.id = i.file_id
                 WHERE f.path = ?1 ORDER BY i.module_path, i.line_no",
            )
            .unwrap()
            .query_map([rel], |row| {
                let module: String = row.get(0)?;
                let no: i64 = row.get(1)?;
                Ok(format!("imports|{module}|{no}"))
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        rows.append(&mut imports);
        let mut nodes: Vec<String> = conn
            .prepare(
                "SELECT p.signature, p.line_start, p.line_end, p.excerpt
                 FROM pattern_nodes p JOIN files f ON f.id = p.file_id
                 WHERE f.path = ?1 ORDER BY p.signature, p.line_start",
            )
            .unwrap()
            .query_map([rel], |row| {
                let sig: String = row.get(0)?;
                let (ls, le): (i64, i64) = (row.get(1)?, row.get(2)?);
                let excerpt: String = row.get(3)?;
                Ok(format!("pattern_nodes|{sig}|{ls}|{le}|{excerpt}"))
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        rows.append(&mut nodes);
        rows
    }
}

/// Restrict a battery to hits from one file (tuple field 2).
fn only_file(battery: &[HitTuple], file: &str) -> Vec<HitTuple> {
    battery.iter().filter(|t| t.2 == file).cloned().collect()
}

// ---- incremental-refresh parity: update_paths deltas == fresh full rebuild ----

#[test]
fn incremental_add_modify_parity_with_fresh_rebuild() {
    const A_V1: &str = "fn i3_alpha_one() {}\nfn i3_shared_callee() {}\n";
    const A_V2: &str = "fn i3_alpha_two() {}\nfn i3_shared_callee() {}\n// tokgammazz note\n";
    const B: &str = "fn i3_beta_caller() { i3_shared_callee(); }\n";
    const C: &str = "fn i3_gamma_new() { let _t = \"tokdeltazz\"; }\n";
    const BATTERY: &[&str] = &[
        "defs:i3_alpha_one",
        "defs:i3_alpha_two",
        "defs:i3_beta_caller",
        "defs:i3_gamma_new",
        "defs:i3_shared_callee",
        "literal:tokgammazz",
        "literal:tokdeltazz",
        "callers:i3_shared_callee",
    ];

    // Incremental side: base index, then one update_paths call per delta.
    let incr = Fx::new();
    let abs_a = incr.write("src/a.rs", A_V1);
    set_mtime_checked(&abs_a, WHOLE_SECOND_T0);
    incr.write("src/b.rs", B);
    assert_eq!(incr.reindex().files_indexed, 2);
    assert!(!incr.battery(&["defs:i3_alpha_one"]).is_empty());

    incr.write("src/a.rs", A_V2);
    set_mtime_checked(&abs_a, WHOLE_SECOND_T1);
    assert_eq!(incr.update(std::slice::from_ref(&abs_a)).files_indexed, 1);
    let abs_c = incr.write("src/c.rs", C);
    assert_eq!(incr.update(std::slice::from_ref(&abs_c)).files_indexed, 1);
    let incr_hits = incr.battery(BATTERY);
    assert!(!incr_hits.is_empty(), "parity battery must be non-vacuous");

    // Fresh side: the same final tree indexed once from scratch.
    let fresh = Fx::new();
    fresh.write("src/a.rs", A_V2);
    fresh.write("src/b.rs", B);
    fresh.write("src/c.rs", C);
    assert_eq!(fresh.reindex().files_indexed, 3);
    let fresh_hits = fresh.battery(BATTERY);

    assert_eq!(
        incr_hits, fresh_hits,
        "incremental deltas must converge to the fresh-rebuild hit sets"
    );
    assert_eq!(
        incr.stored_counts(),
        fresh.stored_counts(),
        "incremental deltas must converge to the fresh-rebuild row counts"
    );
}

#[test]
fn incremental_delete_rename_parity_with_fresh_rebuild() {
    const A: &str = "fn i3_keep_alpha() { let _t = \"tokkeepaazz\"; }\n";
    const B: &str = "fn i3_doomed_beta() { let _t = \"tokdoomedzz\"; }\n";
    const C: &str = "fn i3_roam_gamma() { i3_keep_alpha(); let _t = \"tokroamzz\"; }\n";
    const BATTERY: &[&str] = &[
        "defs:i3_keep_alpha",
        "defs:i3_doomed_beta",
        "defs:i3_roam_gamma",
        "literal:tokkeepaazz",
        "literal:tokdoomedzz",
        "literal:tokroamzz",
        "callers:i3_keep_alpha",
    ];

    // Incremental side: delete via update_paths, rename as remove+add updates.
    let incr = Fx::new();
    incr.write("src/a.rs", A);
    let abs_b = incr.write("src/b.rs", B);
    let abs_c = incr.write("src/c.rs", C);
    assert_eq!(incr.reindex().files_indexed, 3);

    std::fs::remove_file(&abs_b).unwrap();
    assert_eq!(incr.update(std::slice::from_ref(&abs_b)).files_removed, 1);
    let abs_d = incr.root.join("src/d.rs");
    std::fs::rename(&abs_c, &abs_d).unwrap();
    assert_eq!(incr.update(std::slice::from_ref(&abs_c)).files_removed, 1);
    assert_eq!(incr.update(std::slice::from_ref(&abs_d)).files_indexed, 1);
    let incr_hits = incr.battery(BATTERY);
    assert!(!incr_hits.is_empty(), "parity battery must be non-vacuous");

    // Fresh side: the same final tree indexed once from scratch.
    let fresh = Fx::new();
    fresh.write("src/a.rs", A);
    fresh.write("src/d.rs", C);
    assert_eq!(fresh.reindex().files_indexed, 2);

    assert_eq!(
        incr_hits,
        fresh.battery(BATTERY),
        "incremental delete+rename must converge to the fresh-rebuild hit sets"
    );
    assert_eq!(
        incr.stored_counts(),
        fresh.stored_counts(),
        "incremental delete+rename must converge to the fresh-rebuild row counts"
    );
}

// ---- delta order independence ----

#[test]
fn delta_order_independence_add_then_modify_vs_modify_then_add() {
    const A_V1: &str = "fn i3_ord_alpha() {}\n";
    const A_V2: &str = "fn i3_ord_alpha_v2() {}\n";
    const B: &str = "fn i3_ord_beta() {}\n";
    const C: &str = "fn i3_ord_gamma() { let _t = \"tokordzz\"; }\n";
    const BATTERY: &[&str] = &[
        "defs:i3_ord_alpha",
        "defs:i3_ord_alpha_v2",
        "defs:i3_ord_beta",
        "defs:i3_ord_gamma",
        "literal:tokordzz",
    ];

    // Order X: modify first, then add.
    let fx_x = Fx::new();
    let abs_a = fx_x.write("src/a.rs", A_V1);
    set_mtime_checked(&abs_a, WHOLE_SECOND_T0);
    fx_x.write("src/b.rs", B);
    assert_eq!(fx_x.reindex().files_indexed, 2);
    fx_x.write("src/a.rs", A_V2);
    set_mtime_checked(&abs_a, WHOLE_SECOND_T1);
    assert_eq!(fx_x.update(std::slice::from_ref(&abs_a)).files_indexed, 1);
    let abs_c = fx_x.write("src/c.rs", C);
    assert_eq!(fx_x.update(std::slice::from_ref(&abs_c)).files_indexed, 1);

    // Order Y: the same deltas, add first, then modify.
    let fx_y = Fx::new();
    let abs_a = fx_y.write("src/a.rs", A_V1);
    set_mtime_checked(&abs_a, WHOLE_SECOND_T0);
    fx_y.write("src/b.rs", B);
    assert_eq!(fx_y.reindex().files_indexed, 2);
    let abs_c = fx_y.write("src/c.rs", C);
    assert_eq!(fx_y.update(std::slice::from_ref(&abs_c)).files_indexed, 1);
    fx_y.write("src/a.rs", A_V2);
    set_mtime_checked(&abs_a, WHOLE_SECOND_T1);
    assert_eq!(fx_y.update(std::slice::from_ref(&abs_a)).files_indexed, 1);

    let hits_x = fx_x.battery(BATTERY);
    assert!(!hits_x.is_empty(), "order battery must be non-vacuous");
    assert_eq!(
        hits_x,
        fx_y.battery(BATTERY),
        "delta order must not change the final hit sets"
    );
    assert_eq!(
        fx_x.stored_counts(),
        fx_y.stored_counts(),
        "delta order must not change the final row counts"
    );
    assert_eq!(
        fx_x.generation(),
        fx_y.generation(),
        "delta order must not change the final generation"
    );
}

#[test]
fn delta_order_independence_delete_vs_modify() {
    const A_V1: &str = "fn i3_chg_old() { let _t = \"tokchgzz\"; }\n";
    const A_V2: &str = "fn i3_chg_new() { let _t = \"tokchgzz\"; }\n";
    const B: &str = "fn i3_gone_beta() {}\n";
    const C: &str = "fn i3_anchor_gamma() {}\n";
    const BATTERY: &[&str] = &[
        "defs:i3_chg_old",
        "defs:i3_chg_new",
        "defs:i3_gone_beta",
        "defs:i3_anchor_gamma",
        "literal:tokchgzz",
    ];

    // Order X: delete first, then modify.
    let fx_x = Fx::new();
    let abs_a = fx_x.write("src/a.rs", A_V1);
    set_mtime_checked(&abs_a, WHOLE_SECOND_T0);
    let abs_b = fx_x.write("src/b.rs", B);
    fx_x.write("src/c.rs", C);
    assert_eq!(fx_x.reindex().files_indexed, 3);
    std::fs::remove_file(&abs_b).unwrap();
    assert_eq!(fx_x.update(std::slice::from_ref(&abs_b)).files_removed, 1);
    fx_x.write("src/a.rs", A_V2);
    set_mtime_checked(&abs_a, WHOLE_SECOND_T1);
    assert_eq!(fx_x.update(std::slice::from_ref(&abs_a)).files_indexed, 1);

    // Order Y: modify first, then delete.
    let fx_y = Fx::new();
    let abs_a = fx_y.write("src/a.rs", A_V1);
    set_mtime_checked(&abs_a, WHOLE_SECOND_T0);
    let abs_b = fx_y.write("src/b.rs", B);
    fx_y.write("src/c.rs", C);
    assert_eq!(fx_y.reindex().files_indexed, 3);
    fx_y.write("src/a.rs", A_V2);
    set_mtime_checked(&abs_a, WHOLE_SECOND_T1);
    assert_eq!(fx_y.update(std::slice::from_ref(&abs_a)).files_indexed, 1);
    std::fs::remove_file(&abs_b).unwrap();
    assert_eq!(fx_y.update(std::slice::from_ref(&abs_b)).files_removed, 1);

    let hits_x = fx_x.battery(BATTERY);
    assert!(!hits_x.is_empty(), "order battery must be non-vacuous");
    assert_eq!(
        hits_x,
        fx_y.battery(BATTERY),
        "delete/modify order must not change the final hit sets"
    );
    assert_eq!(
        fx_x.stored_counts(),
        fx_y.stored_counts(),
        "delete/modify order must not change the final row counts"
    );
    assert_eq!(
        fx_x.generation(),
        fx_y.generation(),
        "delete/modify order must not change the final generation"
    );
}

// ---- refresh idempotence ----

#[test]
fn full_refresh_twice_identical_to_once() {
    const A_V1: &str = "fn i3_idem_old() { let _t = \"tokidemzz\"; }\n";
    const A_V2: &str = "fn i3_idem_new() { let _t = \"tokidemzz\"; }\n";
    const B: &str = "fn i3_idem_gone() {}\n";
    const C: &str = "fn i3_idem_added() {}\n";
    const BATTERY: &[&str] = &[
        "defs:i3_idem_old",
        "defs:i3_idem_new",
        "defs:i3_idem_gone",
        "defs:i3_idem_added",
        "literal:tokidemzz",
    ];

    let fx = Fx::new();
    let abs_a = fx.write("src/a.rs", A_V1);
    set_mtime_checked(&abs_a, WHOLE_SECOND_T0);
    fx.write("src/b.rs", B);
    assert_eq!(fx.reindex().files_indexed, 2);

    fx.write("src/a.rs", A_V2);
    set_mtime_checked(&abs_a, WHOLE_SECOND_T1);
    fx.write("src/c.rs", C);
    std::fs::remove_file(fx.root.join("src/b.rs")).unwrap();
    let first = fx.reindex();
    assert_eq!(first.files_indexed, 2);
    assert_eq!(first.files_removed, 1);
    let hits_once = fx.battery(BATTERY);
    assert!(!hits_once.is_empty(), "idempotence battery must be non-vacuous");
    let gen_once = fx.generation();
    let counts_once = fx.stored_counts();

    let second = fx.reindex();
    assert_eq!(second.files_indexed, 0);
    assert_eq!(second.files_removed, 0);
    assert_eq!(second.files_failed, 0);
    assert_eq!(
        fx.battery(BATTERY),
        hits_once,
        "a second full refresh must leave hit sets identical"
    );
    assert_eq!(
        fx.generation(),
        gen_once,
        "a second full refresh must not move the generation"
    );
    assert_eq!(
        fx.stored_counts(),
        counts_once,
        "a second full refresh must leave row counts identical"
    );
}

#[test]
fn incremental_refresh_twice_identical_to_once() {
    const A_V1: &str = "fn i3_inc_old() {}\n";
    const A_V2: &str = "fn i3_inc_new() {}\n";
    const B: &str = "fn i3_inc_gone() {}\n";
    const C: &str = "fn i3_inc_added() { let _t = \"tokinccc\"; }\n";
    const BATTERY: &[&str] = &[
        "defs:i3_inc_old",
        "defs:i3_inc_new",
        "defs:i3_inc_gone",
        "defs:i3_inc_added",
        "literal:tokinccc",
    ];

    let fx = Fx::new();
    let abs_a = fx.write("src/a.rs", A_V1);
    set_mtime_checked(&abs_a, WHOLE_SECOND_T0);
    let abs_b = fx.write("src/b.rs", B);
    assert_eq!(fx.reindex().files_indexed, 2);

    // ADD applied twice: the second application is a mutation-free skip.
    let abs_c = fx.write("src/c.rs", C);
    assert_eq!(fx.update(std::slice::from_ref(&abs_c)).files_indexed, 1);
    let gen_after_add = fx.generation();
    let repeat_add = fx.update(std::slice::from_ref(&abs_c));
    assert_eq!(repeat_add.files_indexed, 0);
    assert_eq!(repeat_add.files_skipped, 1);
    assert_eq!(fx.generation(), gen_after_add);

    // MODIFY applied twice: same skip on the repeat.
    fx.write("src/a.rs", A_V2);
    set_mtime_checked(&abs_a, WHOLE_SECOND_T1);
    assert_eq!(fx.update(std::slice::from_ref(&abs_a)).files_indexed, 1);
    let gen_after_mod = fx.generation();
    let repeat_mod = fx.update(std::slice::from_ref(&abs_a));
    assert_eq!(repeat_mod.files_indexed, 0);
    assert_eq!(repeat_mod.files_skipped, 1);
    assert_eq!(fx.generation(), gen_after_mod);

    // DELETE applied twice: the repeat finds no stored row and mutates nothing.
    std::fs::remove_file(&abs_b).unwrap();
    assert_eq!(fx.update(std::slice::from_ref(&abs_b)).files_removed, 1);
    let hits_once = fx.battery(BATTERY);
    assert!(!hits_once.is_empty(), "idempotence battery must be non-vacuous");
    let gen_after_del = fx.generation();
    let counts_once = fx.stored_counts();
    let repeat_del = fx.update(std::slice::from_ref(&abs_b));
    assert_eq!(repeat_del.files_indexed, 0);
    assert_eq!(repeat_del.files_removed, 0);
    assert_eq!(
        fx.battery(BATTERY),
        hits_once,
        "repeating a delete must leave hit sets identical"
    );
    assert_eq!(
        fx.generation(),
        gen_after_del,
        "repeating a delete must not move the generation"
    );
    assert_eq!(
        fx.stored_counts(),
        counts_once,
        "repeating a delete must leave row counts identical"
    );
}

// ---- generation monotonicity ----

#[test]
fn generation_monotone_across_refresh_sequence_with_exact_single_steps() {
    let fx = Fx::new();
    let abs_a = fx.write("src/a.rs", "fn i3_seq_alpha() {}\n");
    set_mtime_checked(&abs_a, WHOLE_SECOND_T0);
    fx.write("src/b.rs", "fn i3_seq_beta() {}\n");
    assert_eq!(fx.reindex().files_indexed, 2);

    let mut series = vec![fx.generation()];

    // Noop full refresh: no mutation, generation unmoved.
    let noop = fx.reindex();
    assert_eq!(noop.files_indexed, 0);
    assert_eq!(noop.files_removed, 0);
    series.push(fx.generation());
    assert_eq!(series[1], series[0]);

    // Single-file ADD: exactly one generation step.
    let abs_c = fx.write("src/c.rs", "fn i3_seq_gamma() {}\n");
    assert_eq!(fx.update(std::slice::from_ref(&abs_c)).files_indexed, 1);
    series.push(fx.generation());
    assert_eq!(series[2], series[1] + 1);

    // Single-file MODIFY: exactly one generation step.
    fx.write("src/a.rs", "fn i3_seq_alpha_v2() {}\n");
    set_mtime_checked(&abs_a, WHOLE_SECOND_T1);
    assert_eq!(fx.update(std::slice::from_ref(&abs_a)).files_indexed, 1);
    series.push(fx.generation());
    assert_eq!(series[3], series[2] + 1);

    // Single-file DELETE: exactly one generation step.
    let abs_b = fx.root.join("src/b.rs");
    std::fs::remove_file(&abs_b).unwrap();
    assert_eq!(fx.update(std::slice::from_ref(&abs_b)).files_removed, 1);
    series.push(fx.generation());
    assert_eq!(series[4], series[3] + 1);

    // Noop incremental refresh: re-updating an unchanged file moves nothing.
    let repeat = fx.update(std::slice::from_ref(&abs_c));
    assert_eq!(repeat.files_indexed, 0);
    series.push(fx.generation());
    assert_eq!(series[5], series[4]);

    assert!(
        series.windows(2).all(|w| w[1] >= w[0]),
        "generation must never move backward: {series:?}"
    );
}

#[test]
fn bulk_refresh_generation_delta_equals_mutated_file_count() {
    let fx = Fx::new();
    let abs_a = fx.write("src/a.rs", "fn i3_bulk_a() {}\n");
    set_mtime_checked(&abs_a, WHOLE_SECOND_T0);
    let abs_b = fx.write("src/b.rs", "fn i3_bulk_b() {}\n");
    set_mtime_checked(&abs_b, WHOLE_SECOND_T0);
    fx.write("src/c.rs", "fn i3_bulk_c() {}\n");
    assert_eq!(fx.reindex().files_indexed, 3);
    let gen_before = fx.generation();

    fx.write("src/a.rs", "fn i3_bulk_a_v2() {}\n");
    set_mtime_checked(&abs_a, WHOLE_SECOND_T1);
    fx.write("src/b.rs", "fn i3_bulk_b_v2() {}\n");
    set_mtime_checked(&abs_b, WHOLE_SECOND_T1);
    std::fs::remove_file(fx.root.join("src/c.rs")).unwrap();
    fx.write("src/d.rs", "fn i3_bulk_d() {}\n");

    let stats = fx.reindex();
    assert_eq!(stats.files_indexed, 3);
    assert_eq!(stats.files_removed, 1);
    assert_eq!(stats.files_skipped, 0);
    assert_eq!(
        fx.generation(),
        gen_before + 4,
        "bulk refresh must bump once per mutated file (3 upserts + 1 removal)"
    );
}

// ---- untouched-file stability ----

#[test]
fn untouched_files_keep_byte_identical_stored_rows() {
    const CHURN_V1: &str = "fn i3_churn_v1() {}\n";
    const CHURN_V2: &str = "fn i3_churn_v2() {}\n";
    const KEEP1: &str = "fn i3_keep_one() { i3_keep_two(); let _t = \"tokkeep1zz\"; }\n";
    const KEEP2: &str = "fn i3_keep_two() {}\n";
    const NEW: &str = "fn i3_brand_new() {}\n";
    const BATTERY: &[&str] = &[
        "defs:i3_churn_v1",
        "defs:i3_churn_v2",
        "defs:i3_keep_one",
        "defs:i3_keep_two",
        "defs:i3_brand_new",
        "literal:tokkeep1zz",
        "callers:i3_keep_two",
    ];

    let fx = Fx::new();
    let abs_churn = fx.write("src/churn.rs", CHURN_V1);
    set_mtime_checked(&abs_churn, WHOLE_SECOND_T0);
    fx.write("src/keep1.rs", KEEP1);
    fx.write("src/keep2.rs", KEEP2);
    assert_eq!(fx.reindex().files_indexed, 3);
    let rows_keep1_before = fx.file_rows("src/keep1.rs");
    let rows_keep2_before = fx.file_rows("src/keep2.rs");
    assert!(!rows_keep1_before.is_empty() && !rows_keep2_before.is_empty());
    let battery_before = fx.battery(BATTERY);
    assert!(!only_file(&battery_before, "src/keep1.rs").is_empty());

    // Incremental churn elsewhere: modify + add via update_paths.
    fx.write("src/churn.rs", CHURN_V2);
    set_mtime_checked(&abs_churn, WHOLE_SECOND_T1);
    assert_eq!(fx.update(std::slice::from_ref(&abs_churn)).files_indexed, 1);
    let abs_new = fx.write("src/new.rs", NEW);
    assert_eq!(fx.update(std::slice::from_ref(&abs_new)).files_indexed, 1);

    assert_eq!(
        fx.file_rows("src/keep1.rs"),
        rows_keep1_before,
        "untouched file rows must be byte-identical after incremental churn"
    );
    assert_eq!(
        fx.file_rows("src/keep2.rs"),
        rows_keep2_before,
        "untouched file rows must be byte-identical after incremental churn"
    );
    let battery_after = fx.battery(BATTERY);
    for untouched in ["src/keep1.rs", "src/keep2.rs"] {
        assert_eq!(
            only_file(&battery_after, untouched),
            only_file(&battery_before, untouched),
            "untouched file {untouched} must keep identical hit subsets"
        );
    }
}

#[test]
fn untouched_file_hits_stable_under_full_refresh_churn() {
    const CHURN1_V1: &str = "fn i3_c1_old() {}\n";
    const CHURN1_V2: &str = "fn i3_c1_new() {}\n";
    const CHURN2: &str = "fn i3_c2_gone() {}\n";
    const KEEP: &str = "fn i3_anchor_keep() { let _t = \"tokanchorzz\"; }\n";
    const NEW: &str = "fn i3_added_late() {}\n";
    const BATTERY: &[&str] = &[
        "defs:i3_c1_old",
        "defs:i3_c1_new",
        "defs:i3_c2_gone",
        "defs:i3_anchor_keep",
        "defs:i3_added_late",
        "literal:tokanchorzz",
    ];

    let fx = Fx::new();
    let abs_c1 = fx.write("src/churn1.rs", CHURN1_V1);
    set_mtime_checked(&abs_c1, WHOLE_SECOND_T0);
    fx.write("src/churn2.rs", CHURN2);
    fx.write("src/keep.rs", KEEP);
    assert_eq!(fx.reindex().files_indexed, 3);
    let rows_before = fx.file_rows("src/keep.rs");
    let hash_before = IndexStore::open(&fx.root, Some(&fx.db))
        .unwrap()
        .file_hash("src/keep.rs")
        .unwrap();
    let battery_before = only_file(&fx.battery(BATTERY), "src/keep.rs");
    assert!(!battery_before.is_empty(), "anchor hits must be non-vacuous");

    // Full-refresh churn across every delta class at once.
    fx.write("src/churn1.rs", CHURN1_V2);
    set_mtime_checked(&abs_c1, WHOLE_SECOND_T1);
    std::fs::remove_file(fx.root.join("src/churn2.rs")).unwrap();
    fx.write("src/new.rs", NEW);
    let stats = fx.reindex();
    assert_eq!(stats.files_indexed, 2);
    assert_eq!(stats.files_removed, 1);

    assert_eq!(
        only_file(&fx.battery(BATTERY), "src/keep.rs"),
        battery_before,
        "untouched file must keep identical hits under full-refresh churn"
    );
    assert_eq!(
        fx.file_rows("src/keep.rs"),
        rows_before,
        "untouched file must keep byte-identical rows under full-refresh churn"
    );
    assert_eq!(
        IndexStore::open(&fx.root, Some(&fx.db))
            .unwrap()
            .file_hash("src/keep.rs")
            .unwrap(),
        hash_before,
        "untouched file must keep its stored content hash"
    );
}

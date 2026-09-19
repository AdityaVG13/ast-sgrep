//! Canonical core invalidation suite: FILE-DELTA MATRIX (I2 MERGEs + I3 M-untouched).
//!
//! Implements `tests/catalog/invalidation-core.md` matrix targets `M-defs`,
//! `M-literals`, `M-callers`, and `M-untouched` — one test per target, each a
//! per-delta-class matrix over ADD / MODIFY / DELETE / RENAME / NOOP. Every
//! absorbed test survives below as a named phase with its discriminants
//! intact; phases run against fresh fixtures so delta classes cannot leak
//! into each other.
//!
//! Absorption map:
//! - M-defs absorbs `add_file_makes_its_symbols_searchable`,
//!   `modify_file_retires_old_symbol_publishes_new`,
//!   `delete_file_defs_vanish_sibling_defs_remain`,
//!   `rename_file_moves_hits_to_new_path`,
//!   `noop_rewrite_same_bytes_search_results_identical`.
//! - M-literals absorbs `modify_file_literal_counts_exact_per_path`.
//! - M-callers absorbs `add_file_with_caller_edge_exposes_callers`,
//!   `delete_file_prunes_literal_and_caller_hits`,
//!   `noop_rewrite_same_bytes_caller_graph_stable`.
//! - M-untouched absorbs `add_file_leaves_sibling_hit_sets_untouched`,
//!   `modify_file_sibling_defs_and_literals_untouched`,
//!   `rename_file_preserves_sibling_hits_and_total_counts`,
//!   `untouched_files_keep_byte_identical_stored_rows`,
//!   `untouched_file_hits_stable_under_full_refresh_churn`.
//!
//! DELETE (not absorbed anywhere): `modify_file_via_update_paths_reflects_delta`
//! is strictly subsumed by I3 `incremental_add_modify_parity_with_fresh_rebuild`
//! plus I2 `modify_file_retires_old_symbol_publishes_new` (catalog verdict);
//! it has no unique discriminant and is intentionally absent.
//!
//! Overlap finding (catalog): I1 pins the detection MECHANISM, this file pins
//! the search-visible CONSEQUENCE — complementary halves, so the NOOP phases
//! below stand alongside (not inside) the mtime-gate tests in
//! `invalidation_staleness.rs`; nothing was dropped as a duplicate.
//!
//! Determinism: MODIFY/NOOP phases forge explicit whole-second mtimes with
//! read-back preconditions via testkit `set_mtime_secs`. No wall-clock sleeps.
//! Hermeticity: every open uses an explicit `index_path` outside the corpus
//! root, so no test depends on ambient `ASGREP_*` routing.

use ast_sgrep_testkit::{set_mtime_secs, HitTuple, InvalidationFixture as Fx};

/// Fixed whole-second stamps (nanos = 0 survives every filesystem timestamp
/// granularity). MODIFY/NOOP phases forge T0 at creation and T1 after the
/// rewrite so change detection is deterministic.
const WHOLE_SECOND_T0: u64 = 1_700_000_000;
const WHOLE_SECOND_T1: u64 = 1_700_003_600;

// Matrix phases ride the shared hermetic fixture from testkit
// (`core_invalidation::InvalidationFixture`, aliased to the file's `Fx`
// vocabulary); the 6-table row dump below is the only file-local surface.
// (The NOOP phase's `stored_counts` comparison now rides the promoted 5-tuple:
// strictly stronger than the former 3-tuple, same assertion shape.)
// WHY area-local: canonical content-row dump for the M-untouched
// byte-identity phases — single-suite projector over the shared fixture.
trait MatrixFixtureExt {
    /// Canonical content rows for one stored file, joined by path and ordered.
    /// Surrogate integer ids are excluded; everything else is byte-compared.
    fn file_rows(&self, rel: &str) -> Vec<String>;
}

impl MatrixFixtureExt for Fx {
    fn file_rows(&self, rel: &str) -> Vec<String> {
        let store = self.open_store();
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
// WHY area-local: battery restriction for the M-untouched phases only;
// single-suite companion to the promoted battery projector.
fn only_file(battery: &[HitTuple], file: &str) -> Vec<HitTuple> {
    battery.iter().filter(|t| t.2 == file).cloned().collect()
}

// WHY area-local: per-path literal-spot counter for the M-literals target
// only; single-suite companion to the promoted literal projector.
fn count_in(spots: &[(String, u32)], file: &str) -> usize {
    spots.iter().filter(|(f, _)| f == file).count()
}

// ---- M-defs: def hit-sets per delta class ----

/// Phase absorbed from `add_file_makes_its_symbols_searchable`: ADD publishes
/// the new file's defs searchable under exactly the new path while sibling
/// defs stay equal.
fn defs_phase_add() {
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

/// Phase absorbed from `modify_file_retires_old_symbol_publishes_new`: MODIFY
/// retires the old def and publishes the new def under the same path.
fn defs_phase_modify() {
    let fx = Fx::new();
    let abs = fx.write("src/a.rs", "fn i2_retire_old() {}\n");
    set_mtime_secs(&abs, WHOLE_SECOND_T0);
    assert_eq!(fx.reindex().files_indexed, 1);
    assert!(!fx.def_files("i2_retire_old").is_empty());

    fx.write("src/a.rs", "fn i2_fresh_new() {}\n");
    set_mtime_secs(&abs, WHOLE_SECOND_T1);
    assert_eq!(fx.reindex().files_indexed, 1);

    assert!(
        fx.def_files("i2_retire_old").is_empty(),
        "MODIFY must retire the old symbol"
    );
    let fresh = fx.def_files("i2_fresh_new");
    assert!(!fresh.is_empty(), "MODIFY must publish the new symbol");
    assert!(fresh.iter().all(|f| f == "src/a.rs"));
}

/// Phase absorbed from `delete_file_defs_vanish_sibling_defs_remain`: DELETE
/// vanishes the removed file's defs (files_removed=1) while sibling defs
/// stay equal.
fn defs_phase_delete() {
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

/// Phase absorbed from `rename_file_moves_hits_to_new_path`: RENAME keeps
/// def+literal hits searchable under exactly the new path (removed=1,
/// indexed=1).
fn defs_phase_rename() {
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

/// Phase absorbed from `noop_rewrite_same_bytes_search_results_identical`:
/// NOOP byte-identical rewrite under a newer forged mtime leaves def/literal
/// hits and stored counts identical. (Complements the I1 mtime-gate
/// mechanism: this phase pins the search-visible consequence.)
fn defs_phase_noop() {
    let fx = Fx::new();
    let abs = fx.write("src/a.rs", "fn i2_noop_one() { let _t = \"toknoopzz\"; }\n");
    set_mtime_secs(&abs, WHOLE_SECOND_T0);
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
    set_mtime_secs(&abs, WHOLE_SECOND_T1);
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

/// INTENT (M-defs): def hit-sets per delta class — ADD publishes under
/// exactly the new path, MODIFY retires old / publishes new, DELETE vanishes
/// the removed file's defs, RENAME moves hits to exactly the new path, NOOP
/// leaves def/literal hits and stored counts identical; path exclusivity
/// throughout.
/// KILLS: add-defs-invisible / hit-misattributed / stale-def-linger /
/// new-def-missing / delete-linger / rename-loses-hits / old-path-linger /
/// identity-rewrite-churn mutants.
/// ABSORBS: `add_file_makes_its_symbols_searchable`,
/// `modify_file_retires_old_symbol_publishes_new`,
/// `delete_file_defs_vanish_sibling_defs_remain`,
/// `rename_file_moves_hits_to_new_path`,
/// `noop_rewrite_same_bytes_search_results_identical` (5 → 1).
#[test]
fn defs_matrix_publish_retire_vanish_move_and_noop_identity() {
    defs_phase_add();
    defs_phase_modify();
    defs_phase_delete();
    defs_phase_rename();
    defs_phase_noop();
}

// ---- M-literals: literal spots + per-path counts ----

/// INTENT (M-literals): MODIFY drops the edited file's literal hits to 0
/// while preserving the sibling per-path count and the total.
/// KILLS: literal-under-prune / over-prune mutants.
/// ABSORBS: `modify_file_literal_counts_exact_per_path` (1 → 1; single-source
/// matrix target, kept as its own target per the catalog fan-in).
#[test]
fn literals_matrix_per_path_counts_exact() {
    let fx = Fx::new();
    let abs = fx.write(
        "src/a.rs",
        "fn i2_cnt_a() {}\n// tokcntzz alpha\n// tokcntzz beta\n",
    );
    set_mtime_secs(&abs, WHOLE_SECOND_T0);
    fx.write("src/b.rs", "fn i2_cnt_b() {}\n// tokcntzz gamma\n");
    assert_eq!(fx.reindex().files_indexed, 2);
    let before = fx.literal_spots("tokcntzz");
    assert!(count_in(&before, "src/a.rs") > 0);
    assert!(count_in(&before, "src/b.rs") > 0);

    fx.write("src/a.rs", "fn i2_cnt_a() {}\n// scrubbed line one\n// scrubbed line two\n");
    set_mtime_secs(&abs, WHOLE_SECOND_T1);
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

// ---- M-callers: caller-edge expose/prune/stability ----

/// Phase absorbed from `add_file_with_caller_edge_exposes_callers`: ADD
/// exposes the new file's caller edge under exactly the new path; callee
/// defs stay under the old path.
fn callers_phase_add() {
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

/// Phase absorbed from `delete_file_prunes_literal_and_caller_hits`: DELETE
/// prunes the removed file's caller+literal hits while the surviving callee's
/// defs stay.
fn callers_phase_delete() {
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

/// Phase absorbed from `noop_rewrite_same_bytes_caller_graph_stable`: NOOP
/// rewrite leaves caller hits and callee-def hits identical.
fn callers_phase_noop() {
    let fx = Fx::new();
    fx.write("src/a.rs", "fn i2_callee_zz() {}\n");
    let abs_b = fx.write("src/b.rs", "fn i2_caller_zz() { i2_callee_zz(); }\n");
    set_mtime_secs(&abs_b, WHOLE_SECOND_T0);
    assert_eq!(fx.reindex().files_indexed, 2);
    let callers_before = fx.caller_files("i2_callee_zz");
    let defs_before = fx.def_files("i2_callee_zz");
    assert!(!callers_before.is_empty(), "precondition: caller edge indexed");
    assert!(callers_before.iter().all(|f| f == "src/b.rs"));
    assert!(!defs_before.is_empty());

    fx.write("src/b.rs", "fn i2_caller_zz() { i2_callee_zz(); }\n");
    set_mtime_secs(&abs_b, WHOLE_SECOND_T1);
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

/// INTENT (M-callers): caller-edge expose/prune/stability — ADD exposes the
/// new file's caller edge under exactly the new path, DELETE prunes the
/// removed file's caller+literal hits while the surviving callee's defs stay,
/// NOOP leaves caller and callee-def hits identical.
/// KILLS: caller-edge-missing / caller-prune-missing / literal-prune-missing /
/// caller-churn-on-noop mutants.
/// ABSORBS: `add_file_with_caller_edge_exposes_callers`,
/// `delete_file_prunes_literal_and_caller_hits`,
/// `noop_rewrite_same_bytes_caller_graph_stable` (3 → 1).
#[test]
fn callers_matrix_expose_prune_and_noop_stability() {
    callers_phase_add();
    callers_phase_delete();
    callers_phase_noop();
}

// ---- M-untouched: sibling/untouched isolation ----

/// Phase absorbed from `add_file_leaves_sibling_hit_sets_untouched`: ADD
/// leaves sibling def+literal hit sets byte-equal while newcomer
/// defs/literals appear.
fn untouched_phase_add() {
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

/// Phase absorbed from `modify_file_sibling_defs_and_literals_untouched`:
/// MODIFY swaps the target's defs/literals exactly while sibling def+literal
/// sets stay equal.
fn untouched_phase_modify() {
    let fx = Fx::new();
    let abs = fx.write(
        "src/a.rs",
        "fn i2_churn_old() { let _t = \"tokchurnold\"; }\n",
    );
    set_mtime_secs(&abs, WHOLE_SECOND_T0);
    fx.write("src/b.rs", "fn i2_stable_sib() { let _t = \"tokstablezz\"; }\n");
    assert_eq!(fx.reindex().files_indexed, 2);
    let sib_defs = fx.def_files("i2_stable_sib");
    let sib_lits = fx.literal_spots("tokstablezz");
    assert!(!sib_defs.is_empty() && !sib_lits.is_empty());

    fx.write("src/a.rs", "fn i2_churn_new() { let _t = \"tokchurnnew\"; }\n");
    set_mtime_secs(&abs, WHOLE_SECOND_T1);
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

/// Phase absorbed from `rename_file_preserves_sibling_hits_and_total_counts`:
/// RENAME preserves sibling def/literal sets, the moved file's hit count +
/// line numbers, and defs under the new path.
fn untouched_phase_rename() {
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

/// Phase absorbed from `untouched_files_keep_byte_identical_stored_rows`:
/// incremental churn elsewhere leaves untouched files' 6-table stored rows
/// byte-identical and their hit subsets equal.
fn untouched_phase_incremental_rows() {
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
    set_mtime_secs(&abs_churn, WHOLE_SECOND_T0);
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
    set_mtime_secs(&abs_churn, WHOLE_SECOND_T1);
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

/// Phase absorbed from `untouched_file_hits_stable_under_full_refresh_churn`:
/// full-refresh all-class churn preserves the anchor file's hits, stored
/// rows, and content hash.
fn untouched_phase_full_refresh_rows() {
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
    set_mtime_secs(&abs_c1, WHOLE_SECOND_T0);
    fx.write("src/churn2.rs", CHURN2);
    fx.write("src/keep.rs", KEEP);
    assert_eq!(fx.reindex().files_indexed, 3);
    let rows_before = fx.file_rows("src/keep.rs");
    let hash_before = fx.open_store().file_hash("src/keep.rs").unwrap();
    let battery_before = only_file(&fx.battery(BATTERY), "src/keep.rs");
    assert!(!battery_before.is_empty(), "anchor hits must be non-vacuous");

    // Full-refresh churn across every delta class at once.
    fx.write("src/churn1.rs", CHURN1_V2);
    set_mtime_secs(&abs_c1, WHOLE_SECOND_T1);
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
        fx.open_store().file_hash("src/keep.rs").unwrap(),
        hash_before,
        "untouched file must keep its stored content hash"
    );
}

/// INTENT (M-untouched): sibling/untouched isolation — ADD/MODIFY/RENAME
/// leave sibling def+literal sets byte-equal; incremental churn elsewhere
/// leaves untouched files' 6-table stored rows byte-identical with equal hit
/// subsets; full-refresh all-class churn preserves the anchor file's hits,
/// rows, and content hash.
/// KILLS: sibling-hit-clobber / sibling-clobber / stale-literal /
/// line-shift / untouched-rewrite / row-churn / full-refresh-clobber mutants.
/// ABSORBS: `add_file_leaves_sibling_hit_sets_untouched`,
/// `modify_file_sibling_defs_and_literals_untouched`,
/// `rename_file_preserves_sibling_hits_and_total_counts`,
/// `untouched_files_keep_byte_identical_stored_rows`,
/// `untouched_file_hits_stable_under_full_refresh_churn` (5 → 1).
#[test]
fn untouched_matrix_sibling_isolation_under_all_churn() {
    untouched_phase_add();
    untouched_phase_modify();
    untouched_phase_rename();
    untouched_phase_incremental_rows();
    untouched_phase_full_refresh_rows();
}

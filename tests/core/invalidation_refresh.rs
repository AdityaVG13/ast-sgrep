//! Canonical core invalidation suite: REFRESH RELATIONS (I3 MERGEs).
//!
//! Implements `tests/catalog/invalidation-core.md` matrix targets
//! `M-generation`, `M-parity`, and `M-idempotence` — one test per target.
//! Every test asserts RELATIONS between refresh strategies (cross-strategy
//! equality of battery vectors, count tuples, generations), not single-delta
//! consequences; each absorbed test survives below as a named phase with its
//! discriminants intact.
//!
//! Absorption map:
//! - M-generation absorbs
//!   `generation_monotone_across_refresh_sequence_with_exact_single_steps`,
//!   `bulk_refresh_generation_delta_equals_mutated_file_count`.
//! - M-parity absorbs `incremental_add_modify_parity_with_fresh_rebuild`,
//!   `incremental_delete_rename_parity_with_fresh_rebuild`,
//!   `delta_order_independence_add_then_modify_vs_modify_then_add`,
//!   `delta_order_independence_delete_vs_modify`.
//! - M-idempotence absorbs `full_refresh_twice_identical_to_once`,
//!   `incremental_refresh_twice_identical_to_once`.
//! (The remaining I3 MERGEs — the two untouched-row tests — live in the
//! M-untouched target in `invalidation_delta_matrix.rs`.)
//!
//! Determinism: MODIFY deltas forge explicit whole-second mtimes with
//! read-back preconditions via testkit `set_mtime_secs`. No wall-clock sleeps.
//! Hermeticity: every open uses an explicit `index_path` outside the corpus
//! root, so no test depends on ambient `ASGREP_*` routing.

use ast_sgrep_testkit::{set_mtime_secs, InvalidationFixture as Fx};

/// Fixed whole-second stamps (nanos = 0 survives every filesystem timestamp
/// granularity). MODIFY phases forge T0 at creation and T1 after the rewrite.
const WHOLE_SECOND_T0: u64 = 1_700_000_000;
const WHOLE_SECOND_T1: u64 = 1_700_003_600;

// Refresh relations ride the shared hermetic fixture from testkit
// (`core_invalidation::InvalidationFixture`, aliased to the file's `Fx`
// vocabulary) with no other file-local surface.

// ---- M-generation: refresh-level generation accounting ----

/// Phase absorbed from
/// `generation_monotone_across_refresh_sequence_with_exact_single_steps`:
/// generation never regresses; noop refreshes leave it unmoved; each
/// single-file add/modify/delete steps exactly +1.
fn generation_phase_exact_single_steps() {
    let fx = Fx::new();
    let abs_a = fx.write("src/a.rs", "fn i3_seq_alpha() {}\n");
    set_mtime_secs(&abs_a, WHOLE_SECOND_T0);
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
    set_mtime_secs(&abs_a, WHOLE_SECOND_T1);
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

/// Phase absorbed from `bulk_refresh_generation_delta_equals_mutated_file_count`:
/// bulk refresh bumps generation by exactly the mutated-file count (3 upserts
/// + 1 removal = +4).
fn generation_phase_bulk_counts_mutations() {
    let fx = Fx::new();
    let abs_a = fx.write("src/a.rs", "fn i3_bulk_a() {}\n");
    set_mtime_secs(&abs_a, WHOLE_SECOND_T0);
    let abs_b = fx.write("src/b.rs", "fn i3_bulk_b() {}\n");
    set_mtime_secs(&abs_b, WHOLE_SECOND_T0);
    fx.write("src/c.rs", "fn i3_bulk_c() {}\n");
    assert_eq!(fx.reindex().files_indexed, 3);
    let gen_before = fx.generation();

    fx.write("src/a.rs", "fn i3_bulk_a_v2() {}\n");
    set_mtime_secs(&abs_a, WHOLE_SECOND_T1);
    fx.write("src/b.rs", "fn i3_bulk_b_v2() {}\n");
    set_mtime_secs(&abs_b, WHOLE_SECOND_T1);
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

/// INTENT (M-generation): refresh-level generation accounting — the
/// generation never regresses, noop full/incremental refreshes leave it
/// frozen, each single-file add/modify/delete steps exactly +1, and a bulk
/// refresh bumps by exactly the mutated-file count.
/// KILLS: backward / double / missed-bump / bulk-miscount mutants.
/// ABSORBS: `generation_monotone_across_refresh_sequence_with_exact_single_steps`,
/// `bulk_refresh_generation_delta_equals_mutated_file_count` (2 → 1).
#[test]
fn generation_matrix_monotone_exact_steps_and_bulk() {
    generation_phase_exact_single_steps();
    generation_phase_bulk_counts_mutations();
}

// ---- M-parity: incremental ≡ fresh + order independence ----

/// Phase absorbed from `incremental_add_modify_parity_with_fresh_rebuild`:
/// `update_paths` add+modify converge to the fresh-rebuild battery and row
/// counts.
fn parity_phase_add_modify() {
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
    set_mtime_secs(&abs_a, WHOLE_SECOND_T0);
    incr.write("src/b.rs", B);
    assert_eq!(incr.reindex().files_indexed, 2);
    assert!(!incr.battery(&["defs:i3_alpha_one"]).is_empty());

    incr.write("src/a.rs", A_V2);
    set_mtime_secs(&abs_a, WHOLE_SECOND_T1);
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

/// Phase absorbed from `incremental_delete_rename_parity_with_fresh_rebuild`:
/// `update_paths` delete + rename-as-remove/add converge to the
/// fresh-rebuild battery and row counts.
fn parity_phase_delete_rename() {
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

/// Phase absorbed from
/// `delta_order_independence_add_then_modify_vs_modify_then_add`: modify+add
/// applied in either order converge to identical battery, counts, and
/// generation.
fn parity_phase_order_add_modify() {
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
    set_mtime_secs(&abs_a, WHOLE_SECOND_T0);
    fx_x.write("src/b.rs", B);
    assert_eq!(fx_x.reindex().files_indexed, 2);
    fx_x.write("src/a.rs", A_V2);
    set_mtime_secs(&abs_a, WHOLE_SECOND_T1);
    assert_eq!(fx_x.update(std::slice::from_ref(&abs_a)).files_indexed, 1);
    let abs_c = fx_x.write("src/c.rs", C);
    assert_eq!(fx_x.update(std::slice::from_ref(&abs_c)).files_indexed, 1);

    // Order Y: the same deltas, add first, then modify.
    let fx_y = Fx::new();
    let abs_a = fx_y.write("src/a.rs", A_V1);
    set_mtime_secs(&abs_a, WHOLE_SECOND_T0);
    fx_y.write("src/b.rs", B);
    assert_eq!(fx_y.reindex().files_indexed, 2);
    let abs_c = fx_y.write("src/c.rs", C);
    assert_eq!(fx_y.update(std::slice::from_ref(&abs_c)).files_indexed, 1);
    fx_y.write("src/a.rs", A_V2);
    set_mtime_secs(&abs_a, WHOLE_SECOND_T1);
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

/// Phase absorbed from `delta_order_independence_delete_vs_modify`:
/// delete+modify applied in either order converge to identical battery,
/// counts, and generation.
fn parity_phase_order_delete_modify() {
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
    set_mtime_secs(&abs_a, WHOLE_SECOND_T0);
    let abs_b = fx_x.write("src/b.rs", B);
    fx_x.write("src/c.rs", C);
    assert_eq!(fx_x.reindex().files_indexed, 3);
    std::fs::remove_file(&abs_b).unwrap();
    assert_eq!(fx_x.update(std::slice::from_ref(&abs_b)).files_removed, 1);
    fx_x.write("src/a.rs", A_V2);
    set_mtime_secs(&abs_a, WHOLE_SECOND_T1);
    assert_eq!(fx_x.update(std::slice::from_ref(&abs_a)).files_indexed, 1);

    // Order Y: modify first, then delete.
    let fx_y = Fx::new();
    let abs_a = fx_y.write("src/a.rs", A_V1);
    set_mtime_secs(&abs_a, WHOLE_SECOND_T0);
    let abs_b = fx_y.write("src/b.rs", B);
    fx_y.write("src/c.rs", C);
    assert_eq!(fx_y.reindex().files_indexed, 3);
    fx_y.write("src/a.rs", A_V2);
    set_mtime_secs(&abs_a, WHOLE_SECOND_T1);
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

/// INTENT (M-parity): incremental ≡ fresh + order independence —
/// `update_paths` add+modify and delete+rename-as-remove/add converge to the
/// fresh-rebuild battery and row counts, and the same delta multiset applied
/// in different orders converges to identical hit sets, counts, and
/// generation.
/// KILLS: incremental-divergence / delete-divergence / rename-divergence /
/// order-dependent mutants.
/// ABSORBS: `incremental_add_modify_parity_with_fresh_rebuild`,
/// `incremental_delete_rename_parity_with_fresh_rebuild`,
/// `delta_order_independence_add_then_modify_vs_modify_then_add`,
/// `delta_order_independence_delete_vs_modify` (4 → 1).
#[test]
fn parity_matrix_incremental_equals_fresh_and_order_free() {
    parity_phase_add_modify();
    parity_phase_delete_rename();
    parity_phase_order_add_modify();
    parity_phase_order_delete_modify();
}

// ---- M-idempotence: full + incremental repeat-noop ----

/// Phase absorbed from `full_refresh_twice_identical_to_once`: a second full
/// refresh is mutation-free (0/0/0 stats, identical battery/generation/counts).
fn idempotence_phase_full_twice() {
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
    set_mtime_secs(&abs_a, WHOLE_SECOND_T0);
    fx.write("src/b.rs", B);
    assert_eq!(fx.reindex().files_indexed, 2);

    fx.write("src/a.rs", A_V2);
    set_mtime_secs(&abs_a, WHOLE_SECOND_T1);
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

/// Phase absorbed from `incremental_refresh_twice_identical_to_once`:
/// repeated `update_paths` add/modify/delete are skips (generation, battery,
/// and counts frozen).
fn idempotence_phase_incremental_twice() {
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
    set_mtime_secs(&abs_a, WHOLE_SECOND_T0);
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
    set_mtime_secs(&abs_a, WHOLE_SECOND_T1);
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

/// INTENT (M-idempotence): full + incremental repeat-noop — a second full
/// refresh is mutation-free (0/0/0 stats, identical battery/generation/counts)
/// and repeated `update_paths` add/modify/delete are skips with the
/// generation, battery, and counts frozen.
/// KILLS: refresh-churn / repeat-mutates mutants.
/// ABSORBS: `full_refresh_twice_identical_to_once`,
/// `incremental_refresh_twice_identical_to_once` (2 → 1).
#[test]
fn idempotence_matrix_repeat_refresh_is_noop() {
    idempotence_phase_full_twice();
    idempotence_phase_incremental_twice();
}

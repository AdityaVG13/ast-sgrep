# Invalidation core catalog (I1–I4)

Scope: `tests/core/invalidation_pass1.rs` (I1 staleness detection), `invalidation_pass2.rs` (I2 file-delta consequences), `invalidation_pass3.rs` (I3 rebuild-parity relations), `invalidation_pass4.rs` (I4 end-to-end drills). 43 `#[test]`s, one bullet each, strict calibration: KEEP only standalone intents; every per-delta-class (add/modify/delete/rename) test merges into a per-surface delta-matrix intent; cross-pass overlaps resolve to DELETE the weaker.

Matrix targets referenced by MERGE verdicts: `M-defs` = def hit-sets per delta class (publish/retire/vanish/move, path exclusivity); `M-literals` = literal spots + per-path counts; `M-callers` = caller-edge expose/prune/stability; `M-untouched` = sibling/untouched isolation (hit subsets, counts, byte-identical rows, hashes) under incremental + full churn; `M-generation` = refresh-level generation accounting (frozen-until-refresh, +1 per mutated file, bulk, no-move-on-noop); `M-parity` = incremental≡fresh + order independence; `M-idempotence` = full + incremental repeat-noop; `M-drill` = e2e serve→change→detect→refresh→serve loop per single delta class.

I1-vs-I2 overlap finding: no wholesale subsumption exists — apparent overlaps are complementary halves (I1 pins the detection mechanism, I2 pins the search-visible consequence; e.g. mtime-skip vs noop-identity go through different paths, status-stale-counts vs delete-hit-vanish assert different surfaces). The single DELETE is an I2-vs-I3 subsumption instead (see below).

## Pass 1 — staleness detection (`tests/core/invalidation_pass1.rs`)

- writer_generation_absent_reads_zero_and_bump_roundtrips: INTENT=Cold-start epoch reads 0 and bumped epoch round-trips including on-disk decimal format; CAT=staleness; KILLS=missing-default/bump-not-persisted/format mutants; VERDICT=KEEP
- writer_generation_bumps_are_unique_not_sequential: INTENT=Successive bumps publish distinct epochs (unique epoch, explicitly not a read+1 counter); CAT=staleness; KILLS=read+1-counter mutant; VERDICT=KEEP
- writer_generation_isolated_per_home_and_fail_open_on_corrupt_stamp: INTENT=Stamps isolate per root and per pinned DB parent; corrupt/empty stamp fails open to 0 without cross-root bleed; CAT=staleness; KILLS=shared-stamp/corrupt-errors mutants; VERDICT=KEEP
- index_data_version_bumps_exactly_one_per_upsert: INTENT=Each store upsert (including same-structure refresh_lines_only path) bumps index_data_version by exactly 1; index_generation aliases it; CAT=staleness; KILLS=missed/double-bump/alias-drift mutants; VERDICT=KEEP
- mtime_match_short_circuits_hash_check_within_certified_root: INTENT=Matching (secs,nanos) under certified root skips without consulting stored hash (tamper survives, generation unmoved, skipped=1); CAT=staleness; KILLS=always-hash/always-reextract mutants; VERDICT=KEEP
- mtime_gate_deletion_and_nanos_mismatch_force_hash_recheck: INTENT=Gate deletion or nanos-only stored/fresh mismatch defeats the mtime skip and forces hash-consult re-extraction; CAT=staleness; KILLS=gate-ignored/secs-only-identity mutants; VERDICT=KEEP
- cross_root_db_reuse_disables_mtime_trust: INTENT=Same mtime but different bytes across roots forces a content decision (no skip); within-root control with same forge does skip; CAT=staleness; KILLS=cross-root-mtime-trust mutant; VERDICT=KEEP
- newer_than_binary_schema_refused_on_both_open_modes: INTENT=Newer-than-binary schema fails closed on writable and readonly opens with machine-readable (on_disk, supported) pair; side-effect-free peek still reports the stamp; CAT=staleness; KILLS=newer-fail-open/peek-side-effect mutants; VERDICT=KEEP
- stale_schema_lifecycle_peek_refuse_migrate_idempotent: INTENT=Stale stamp lifecycle: peek reports without migrating, readonly refuses side-effect-free, writable migrates in place, second open is a noop; CAT=staleness; KILLS=skip-migration/readonly-mutates/non-idempotent mutants; VERDICT=KEEP
- cache_index_path_deterministic_and_root_isolated: INTENT=Cache path is deterministic per root, distinct across roots, rooted at XDG base with index.db leaf; CAT=staleness; KILLS=random-salt/shared-home/wrong-base mutants; VERDICT=KEEP
- cache_routing_local_wins_and_env_selects_base_fail_closed: INTENT=Routing: cache home when no local DB exists, present local DB wins over cache, HOME fallback, error when no base is resolvable; CAT=staleness; KILLS=precedence-inversion/missing-fallback mutants; VERDICT=KEEP
- status_reports_stored_counts_and_live_writer_epoch: INTENT=Status reports stored file/line/symbol counts + paths and reads the writer stamp live on every call; CAT=staleness; KILLS=cached-stamp/count mutants; VERDICT=KEEP
- status_distinguishes_empty_stored_stale_and_missing: INTENT=Missing DB errors on open+peek, fresh DB reports zeros, status shows stored (stale) counts until a reindex prunes them; CAT=staleness; KILLS=live-tree-status/missing-fail-open mutants; VERDICT=KEEP

## Pass 2 — file deltas (`tests/core/invalidation_pass2.rs`)

- add_file_makes_its_symbols_searchable: INTENT=ADD publishes the new file's defs searchable under exactly the new path while sibling defs stay equal; CAT=delta; KILLS=add-defs-invisible/hit-misattributed mutants; VERDICT=MERGE→M-defs
- add_file_leaves_sibling_hit_sets_untouched: INTENT=ADD leaves sibling def+literal hit sets byte-equal while newcomer defs/literals appear; CAT=delta; KILLS=sibling-hit-clobber mutants; VERDICT=MERGE→M-untouched
- add_file_with_caller_edge_exposes_callers: INTENT=ADD exposes the new file's caller edge under exactly the new path; callee defs stay under the old path; CAT=delta; KILLS=caller-edge-missing mutants; VERDICT=MERGE→M-callers
- modify_file_retires_old_symbol_publishes_new: INTENT=MODIFY retires the old def and publishes the new def under the same path; CAT=delta; KILLS=stale-def-linger/new-def-missing mutants; VERDICT=MERGE→M-defs
- modify_file_sibling_defs_and_literals_untouched: INTENT=MODIFY swaps the target's defs/literals exactly while sibling def+literal sets stay equal; CAT=delta; KILLS=sibling-clobber/stale-literal mutants; VERDICT=MERGE→M-untouched
- modify_file_literal_counts_exact_per_path: INTENT=MODIFY drops the edited file's literal hits to 0 while preserving the sibling per-path count and the total; CAT=delta; KILLS=literal-under-prune/over-prune mutants; VERDICT=MERGE→M-literals
- modify_file_via_update_paths_reflects_delta: INTENT=update_paths MODIFY retires old / publishes new def (incremental-path variant of the retire/publish intent); CAT=delta; KILLS=BEHAVIOR-ONLY; VERDICT=DELETE+reason: strictly subsumed by I3 incremental_add_modify_parity_with_fresh_rebuild (identical update_paths modify mechanism plus full-battery and row-count equality with a fresh rebuild) combined with I2 modify_file_retires_old_symbol_publishes_new (retire/publish consequence); weaker duplicate with no unique discriminant
- delete_file_defs_vanish_sibling_defs_remain: INTENT=DELETE vanishes the removed file's defs (files_removed=1) while sibling defs stay equal; CAT=delta; KILLS=delete-linger mutants; VERDICT=MERGE→M-defs
- delete_file_prunes_literal_and_caller_hits: INTENT=DELETE prunes the removed file's caller+literal hits while the surviving callee's defs stay; CAT=delta; KILLS=caller/literal-prune-missing mutants; VERDICT=MERGE→M-callers
- rename_file_moves_hits_to_new_path: INTENT=RENAME keeps def+literal hits searchable under exactly the new path (removed=1, indexed=1); CAT=delta; KILLS=rename-loses-hits/old-path-linger mutants; VERDICT=MERGE→M-defs
- rename_file_preserves_sibling_hits_and_total_counts: INTENT=RENAME preserves sibling def/literal sets, the moved file's hit count + line numbers, and defs under the new path; CAT=delta; KILLS=sibling-clobber/line-shift mutants; VERDICT=MERGE→M-untouched
- noop_rewrite_same_bytes_search_results_identical: INTENT=NOOP byte-identical rewrite under a newer forged mtime leaves def/literal hits and stored counts identical; CAT=delta; KILLS=identity-rewrite-churn mutants; VERDICT=MERGE→M-defs
- noop_rewrite_same_bytes_caller_graph_stable: INTENT=NOOP rewrite leaves caller hits and callee-def hits identical; CAT=delta; KILLS=caller-churn-on-noop mutants; VERDICT=MERGE→M-callers

## Pass 3 — rebuild parity (`tests/core/invalidation_pass3.rs`)

- incremental_add_modify_parity_with_fresh_rebuild: INTENT=update_paths add+modify converge to the fresh-rebuild battery and row counts; CAT=parity; KILLS=incremental-divergence mutants; VERDICT=MERGE→M-parity
- incremental_delete_rename_parity_with_fresh_rebuild: INTENT=update_paths delete + rename-as-remove/add converge to the fresh-rebuild battery and row counts; CAT=parity; KILLS=delete/rename-divergence mutants; VERDICT=MERGE→M-parity
- delta_order_independence_add_then_modify_vs_modify_then_add: INTENT=Modify+add applied in either order converge to identical battery, counts, and generation; CAT=parity; KILLS=order-dependent mutants; VERDICT=MERGE→M-parity
- delta_order_independence_delete_vs_modify: INTENT=Delete+modify applied in either order converge to identical battery, counts, and generation; CAT=parity; KILLS=order-dependent mutants; VERDICT=MERGE→M-parity
- full_refresh_twice_identical_to_once: INTENT=A second full refresh is mutation-free (0/0/0 stats, identical battery/generation/counts); CAT=parity; KILLS=refresh-churn mutants; VERDICT=MERGE→M-idempotence
- incremental_refresh_twice_identical_to_once: INTENT=Repeated update_paths add/modify/delete are skips (generation, battery, and counts frozen); CAT=parity; KILLS=repeat-mutates mutants; VERDICT=MERGE→M-idempotence
- generation_monotone_across_refresh_sequence_with_exact_single_steps: INTENT=Generation never regresses; noop refreshes leave it unmoved; each single-file add/modify/delete steps exactly +1; CAT=parity; KILLS=backward/double/missed-bump mutants; VERDICT=MERGE→M-generation
- bulk_refresh_generation_delta_equals_mutated_file_count: INTENT=Bulk refresh bumps generation by exactly the mutated-file count (3 upserts + 1 removal = +4); CAT=parity; KILLS=bulk-miscount mutants; VERDICT=MERGE→M-generation
- untouched_files_keep_byte_identical_stored_rows: INTENT=Incremental churn elsewhere leaves untouched files' 6-table stored rows byte-identical and their hit subsets equal; CAT=parity; KILLS=untouched-rewrite/row-churn mutants; VERDICT=MERGE→M-untouched
- untouched_file_hits_stable_under_full_refresh_churn: INTENT=Full-refresh all-class churn preserves the anchor file's hits, stored rows, and content hash; CAT=parity; KILLS=full-refresh-clobber mutants; VERDICT=MERGE→M-untouched

## Pass 4 — end-to-end drills (`tests/core/invalidation_pass4.rs`)

- drill_add_new_file_end_to_end: INTENT=E2E ADD loop: exact baseline serve → frozen detect (live tree outruns stored count, new symbol invisible) → +1 refresh → exact new serve; CAT=drill; KILLS=add-loop-break mutants; VERDICT=MERGE→M-drill
- drill_modify_file_end_to_end: INTENT=E2E MODIFY loop including same-count undetectability (retired symbol lingers, new symbol invisible pre-refresh); CAT=drill; KILLS=modify-loop-break mutants; VERDICT=MERGE→M-drill
- drill_delete_file_end_to_end: INTENT=E2E DELETE loop: dead-file hits linger pre-refresh, vanish post-refresh with generation +1; CAT=drill; KILLS=delete-loop-break mutants; VERDICT=MERGE→M-drill
- drill_rename_file_end_to_end: INTENT=E2E RENAME loop: stale serve under the dead path, refresh removes+upserts, generation +2 (rename = two mutations); CAT=drill; KILLS=rename-loop/rename-counted-once mutants; VERDICT=MERGE→M-drill
- drill_chained_multi_change_single_refresh: INTENT=One refresh absorbs a modify+delete+rename+add chain (+5) with exact converged sets including caller-edge carry to the renamed path; CAT=drill; KILLS=multi-class-interaction mutants; VERDICT=KEEP
- drill_rapid_succession_change_refresh_change_refresh: INTENT=Back-to-back modify cycles each detect/refresh/serve and converge on the latest content only; CAT=drill; KILLS=cycle-linger/convergence mutants; VERDICT=KEEP
- drill_delete_then_readd_same_path_end_to_end: INTENT=Path resurrection: delete→refresh→re-add of the same path with new content serves exactly the reborn hits (+1 per cycle); CAT=drill; KILLS=resurrection-stale-row mutants; VERDICT=KEEP

## Helper patterns

- `Fx` fixture (I2/I3/I4, near-identical): separate corpus + external-index `TempDir`s, `write` (mkdir -p + write), `reindex` (explicit `index_path`, no tantivy/embed), `searcher` (limit 64/256, no embed); I1 instead uses a bare `indexer_at(root, db)` helper plus direct `IndexStore` opens.
- Whole-second mtime forge with read-back precondition (`set_mtime_checked`, T0/T1/T2 = fixed epoch constants, nanos 0): every MODIFY/NOOP test in all four files; change detection never depends on filesystem timestamp granularity, no wall-clock sleeps.
- Sorted hit projectors (I2/I4, identical): `def_files` (Def hits filtered by exact symbol → sorted files), `literal_spots` (all hits → sorted (file, line) pairs), `caller_files` (Caller hits filtered by exact callee → sorted files); I3 generalizes to `battery()` fixed-query structural `HitTuple`s (scores/excerpts excluded) plus `only_file` restriction.
- I3 `file_rows()`: canonical byte-dump of one file's stored content across 6 tables (files/lines/symbols/callers/imports/pattern_nodes, surrogate ids excluded) for untouched-row equality.
- I4 `DetectionSnap` + `assert_frozen`: snapshots (generation, 5-tuple counts, writer epoch) and asserts the whole detection surface is frozen pre-refresh; `live_rs_files` walks the tree as stale-against ground truth.
- I1-only helpers: `plain_input` (`UpsertFileInput` builder for store-level upserts), `stored_mtime` (direct `(secs, nanos)` row read), `env_lock` + `EnvRestore` RAII guard serializing the two process-env cache-routing tests.
- Universal hermeticity: every open uses an explicit `index_path` outside the corpus root (no ambient `ASGREP_*` dependence); assertions are discriminants only (is_ok/is_err, counts, equality/emptiness, path exclusivity), never message text.

## Counts

- Tests: 43 total = pass1 13 + pass2 13 + pass3 10 + pass4 7.
- Verdicts: KEEP 16 (pass1 13, pass2 0, pass3 0, pass4 3) / MERGE 26 (pass1 0, pass2 12, pass3 10, pass4 4) / DELETE 1 (pass2 1: `modify_file_via_update_paths_reflects_delta`, subsumed by I3 add/modify parity + I2 retire/publish).
- CAT: staleness 13 / delta 13 / parity 10 / drill 7 / other 0.
- MERGE targets: M-defs 5 / M-literals 1 / M-callers 3 / M-untouched 5 (I2 3 + I3 2) / M-generation 2 / M-parity 4 / M-idempotence 2 / M-drill 4.
- KILLS: 42 tests kill a named mutant class; 1 BEHAVIOR-ONLY (the DELETE); 0 TAUTOLOGY-RISK.

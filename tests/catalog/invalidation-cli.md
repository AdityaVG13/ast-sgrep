# Invalidation CLI catalog (I1–I4)

Source files: `tests/cli/invalidation_pass1.rs` (I1, staleness contracts),
`tests/cli/invalidation_pass2.rs` (I2, per-delta served output),
`tests/cli/invalidation_pass3.rs` (I3, rebuild-parity relations),
`tests/cli/invalidation_pass4.rs` (I4, end-to-end drills).
Read fully 2026-09-18. One bullet per `#[test]`.

Merge-target legend (proposed matrix intents, none exist yet):
- `matrix/staleness-counts` — one freshness-count matrix: reuse / rebuild-stale /
  prune / targeted-noop count rows (I1).
- `matrix/targeted-refresh` — one targeted-driver matrix: noop counts (I1) +
  edit-served (I2) + targeted==full bytes (I3).
- `matrix/search-delta` + `matrix/outline-delta` — per-surface delta matrices:
  every delta class × {search hit sets | outline symbol sets} (I2).
- `matrix/parity-incremental-vs-clean`, `matrix/parity-order-independence`,
  `matrix/parity-reindex` — one test per relation family (I3).
- `matrix/drill-lifecycle` — one parameterized drill:
  baseline→stale-middle→reindex→serve per delta kind (I4).

I1-vs-I2 overlap check: no exact duplicates found. I1 pins counts/status
envelopes, I2 pins served search/outline bytes after refresh — complementary
surfaces by design (see file headers). The single DELETE is an intra-I3
strict subsumption (counts implied by byte identity).

## I1 — invalidation_pass1.rs (staleness contracts)

- index_second_run_reuses_fresh_rows: INTENT=no-change second index reuses all rows (0 indexed / 2 skipped); CAT=staleness; KILLS=always-rebuild (freshness-check deletion); VERDICT=MERGE→matrix/staleness-counts
- index_rebuilds_only_stale_file_after_edit: INTENT=post-edit refresh rebuilds only the stale file (1 indexed / 1 skipped); CAT=staleness; KILLS=stale-row-reuse|rebuild-all; VERDICT=MERGE→matrix/staleness-counts
- index_refresh_prunes_deleted_file: INTENT=refresh prunes deleted file (removed=1) and status file_count drops to 1; CAT=staleness; KILLS=prune-omission; VERDICT=MERGE→matrix/staleness-counts
- reindex_rewrites_all_rows_without_edits: INTENT=reindex ignores freshness and rewrites all rows with zero edits; CAT=staleness; KILLS=reindex-degrades-to-index (freshness-reuse-in-reindex); VERDICT=KEEP
- index_path_on_unchanged_file_is_noop_reuse: INTENT=targeted --path on unchanged file is a counted noop (0/1/0); CAT=staleness; KILLS=targeted-always-rewrite; VERDICT=MERGE→matrix/targeted-refresh
- reindex_dry_run_reports_without_writing: INTENT=reindex --dry-run reports counts and creates no DB file; CAT=other; KILLS=write-despite-dry-run; VERDICT=KEEP
- status_discriminants_fresh_vs_missing_index: INTENT=status refuses missing index without creating DB, types discriminants when fresh; CAT=staleness; KILLS=refusal-inversion|create-on-status; VERDICT=KEEP
- search_auto_index_refreshes_stale_index_after_edit: INTENT=--auto-index refreshes stale index before serving the new symbol; CAT=staleness; KILLS=auto-index-no-refresh (serves-stale); VERDICT=KEEP
- auto_index_plus_no_auto_index_refuses: INTENT=--no-auto-index wins over --auto-index (refuses on empty, silent stderr); CAT=other; KILLS=precedence-inversion; VERDICT=KEEP
- mutated_predicate_truth_table: INTENT=IndexStats::mutated true iff indexed|removed nonzero, false on skipped|failed (lib unit); CAT=other; KILLS=predicate-arm-deletion (||→&&, dropped-removed-arm); VERDICT=KEEP
- watch_serves_only_after_initial_index_completes: INTENT=watch first-boot servable only after full initial index (status-gated, keyword-served); CAT=staleness; KILLS=serve-before-ready (gate-omission); VERDICT=KEEP
- watch_restart_resumes_to_servable_index: INTENT=watch restart over existing DB resumes to servable index across two restarts; CAT=staleness; KILLS=resume-regression; VERDICT=KEEP

## I2 — invalidation_pass2.rs (per-delta served output)

- i2_delta_add_single_file_search_and_outline_serve_new_symbol: INTENT=ADD: new file symbol served by search+outline, prior hit sets intact; CAT=delta; KILLS=add-omission (untracked-skipped); VERDICT=MERGE→matrix/search-delta + matrix/outline-delta
- i2_delta_add_two_files_hit_sets_partitioned_per_path: INTENT=ADD×2: hit sets partitioned per path, zero cross-file leak; CAT=delta; KILLS=cross-file-hit-leak (path-confusion); VERDICT=MERGE→matrix/search-delta + matrix/outline-delta
- i2_delta_modify_added_symbol_served_old_symbols_retained: INTENT=MODIFY-add: new symbol served, old and other-file symbols retained; CAT=delta; KILLS=modify-rewrite-skipped; VERDICT=MERGE→matrix/search-delta + matrix/outline-delta
- i2_delta_modify_removed_symbol_stale_hit_never_served: INTENT=MODIFY-del: removed symbol served nowhere post-refresh; CAT=delta; KILLS=stale-token-retention; VERDICT=MERGE→matrix/search-delta + matrix/outline-delta
- i2_delta_modify_renamed_symbol_old_token_gone_new_token_served: INTENT=MODIFY-rename: old token gone and new token served atomically; CAT=delta; KILLS=token-swap-incompleteness; VERDICT=MERGE→matrix/search-delta + matrix/outline-delta
- i2_delta_modify_line_shift_hit_lines_follow_new_rows: INTENT=MODIFY-shift: search and outline line_starts follow symbol to new lines; CAT=delta; KILLS=position-staleness; VERDICT=MERGE→matrix/search-delta + matrix/outline-delta
- i2_delta_delete_file_prunes_search_and_outline: INTENT=DELETE: file pruned from search, outline refuses, survivor intact; CAT=delta; KILLS=prune-omission (serves-deleted, refusal-missing); VERDICT=MERGE→matrix/search-delta + matrix/outline-delta
- i2_delta_delete_then_readd_restores_exact_hit_set: INTENT=DELETE→ADD roundtrip: empty after delete, exact hit set+outline after re-add; CAT=delta; KILLS=tombstone-persistence (resurrection-failure); VERDICT=MERGE→matrix/search-delta + matrix/outline-delta
- i2_delta_rename_file_hits_follow_new_path_only: INTENT=RENAME: hits follow new path only, old outline refuses; CAT=delta; KILLS=old-path-retention|new-path-omission; VERDICT=MERGE→matrix/search-delta + matrix/outline-delta
- i2_delta_rename_with_edit_new_symbol_at_new_path_only: INTENT=RENAME+EDIT: new symbol at new path only, old token gone everywhere; CAT=delta; KILLS=rename/edit-confusion; VERDICT=MERGE→matrix/search-delta + matrix/outline-delta
- i2_delta_noop_rewrite_same_bytes_hit_sets_identical: INTENT=NOOP same-bytes rewrite serves identical hit sets and outline; CAT=delta; KILLS=hash-fallback-corruption (rewrite-changes-output); VERDICT=MERGE→matrix/search-delta + matrix/outline-delta
- i2_delta_targeted_path_refresh_serves_edit_without_full_rescan: INTENT=TARGETED --path edit lands in served output, untouched file intact; CAT=delta; KILLS=targeted-flag-ignored; VERDICT=MERGE→matrix/targeted-refresh

## I3 — invalidation_pass3.rs (rebuild-parity relations)

- i3_incremental_refresh_matches_clean_rebuild_search_bytes: INTENT=incremental refresh == clean rebuild over normalized search bytes per probe query; CAT=parity; KILLS=incremental-divergence (stale-row|missing-prune); VERDICT=MERGE→matrix/parity-incremental-vs-clean
- i3_incremental_refresh_matches_clean_rebuild_outline_bytes: INTENT=same relation over outline bytes incl. deleted-path refusal codes; CAT=parity; KILLS=incremental-outline-divergence; VERDICT=MERGE→matrix/parity-incremental-vs-clean
- i3_incremental_refresh_matches_clean_rebuild_counts: INTENT=incremental == clean over status counts plus per-query hit counts; CAT=parity; KILLS=TAUTOLOGY-RISK+counts-implied-by-sibling-byte-identity; VERDICT=DELETE+strictly subsumed by search-bytes + outline-bytes tests
- i3_targeted_path_refreshes_match_full_refresh_bytes: INTENT=two targeted --path refreshes == one full refresh over search+outline bytes; CAT=parity; KILLS=targeted/full-divergence; VERDICT=MERGE→matrix/targeted-refresh
- i3_delta_order_add_then_modify_matches_modify_then_add: INTENT=delta order unobservable: add→modify == modify→add over search+outline bytes; CAT=parity; KILLS=order-dependent-state; VERDICT=MERGE→matrix/parity-order-independence
- i3_delta_order_delete_then_add_matches_add_then_delete: INTENT=same order-independence for the delete/add pair; CAT=parity; KILLS=order-dependent-prune/add; VERDICT=MERGE→matrix/parity-order-independence
- i3_reindex_twice_serves_byte_identical_search: INTENT=reindex idempotent over search bytes (twice == once); CAT=parity; KILLS=reindex-non-idempotence; VERDICT=MERGE→matrix/parity-reindex
- i3_reindex_twice_outline_and_status_counts_stable: INTENT=reindex idempotent over outline bytes plus status counts; CAT=parity; KILLS=reindex-outline/count-drift; VERDICT=MERGE→matrix/parity-reindex
- i3_reindex_matches_clean_rebuild_bytes: INTENT=in-place reindex == clean rebuild over search+outline bytes; CAT=parity; KILLS=rewrite-path-row-skip; VERDICT=MERGE→matrix/parity-reindex
- i3_converged_refresh_changes_no_observable_bytes: INTENT=post-convergence refresh is a counted noop with zero search/outline/status byte drift; CAT=parity; KILLS=converged-refresh-mutation; VERDICT=KEEP

## I4 — invalidation_pass4.rs (end-to-end drills)

- i4_drill_modify_add_symbol_full_cycle: INTENT=MODIFY-add lifecycle: baseline→stale-invisible→reindex→exact serve; CAT=drill; KILLS=implicit-refresh|reindex-non-convergence; VERDICT=MERGE→matrix/drill-lifecycle
- i4_drill_add_file_full_cycle: INTENT=ADD lifecycle incl. stale outline-refusal of the unindexed path; CAT=drill; KILLS=implicit-refresh|reindex-non-convergence; VERDICT=MERGE→matrix/drill-lifecycle
- i4_drill_delete_file_full_cycle: INTENT=DELETE lifecycle incl. stale-still-served no-implicit-refresh proof; CAT=drill; KILLS=implicit-refresh|prune-non-convergence; VERDICT=MERGE→matrix/drill-lifecycle
- i4_drill_rename_file_full_cycle: INTENT=RENAME lifecycle: stale old-path-only → new-path-only after reindex; CAT=drill; KILLS=implicit-refresh|rename-non-convergence; VERDICT=MERGE→matrix/drill-lifecycle
- i4_drill_modify_remove_symbol_full_cycle: INTENT=MODIFY-remove lifecycle: stale old-served/new-absent → swapped after reindex; CAT=drill; KILLS=implicit-refresh|swap-non-convergence; VERDICT=MERGE→matrix/drill-lifecycle
- i4_drill_chained_add_modify_delete_rename_single_reindex: INTENT=chained modify+delete+add+rename converges exactly in one reindex; CAT=drill; KILLS=chained-delta-interference (rename-loses-edit, tombstone-blocks-add); VERDICT=KEEP
- i4_drill_watch_mode_serves_modify_and_delete_without_manual_reindex: INTENT=live watch daemon converges MODIFY+DELETE with zero manual refresh invocations; CAT=drill; KILLS=watch-no-detect|no-serve-without-manual; VERDICT=KEEP

## Helper patterns

Every file re-declares the same core harness (no shared test module; ~60 lines
copy-pasted 4×, plus `rewrite_with_mtime_bump` 4× and `WatchChild` +
`wait_until_servable` duplicated in I1/I4):
- `asgrep_bin` (CARGO_BIN_EXE_asgrep) + `run` (NO_COLOR=1, captured output).
- `parse_stdout` (panic with stderr on non-JSON) + `assert_success` (exit 0,
  ok==true, command echo).
- `fixture_two_files` (proj/{alpha,beta}.rs + idx/index.db tempdir; I1 returns
  the index PathBuf too).
- `rewrite_with_mtime_bump` (content rewrite + mtime +2s so mtime fast path
  and hash fallback agree).
- Refresh drivers: I1 `run_index`; I2 `run_refresh`; I3
  `run_index`/`run_reindex`/`run_targeted`; I4 `run_index`/`run_reindex`.
- Deterministic served reads: `run_search` with `--no-auto-index` (I2/I3-raw/
  I4); `run_outline` returns raw Output so refusal exit codes are assertable.
- Served-set assertions: `hits_in` / `total_hits` / `assert_served_only_at` /
  `assert_served_nowhere` / `outline_names` (identical in I2/I4).
- I3-only: `PROBE_QUERIES` × `snapshot_search` (strips `snapshot.generation`)
  + `snapshot_outline` (raw bytes) + `normalized_status_bytes` (strips random
  `writer_generation`); `mutate_to_final_tree` canonical fixture; fixed-mtime
  `stamp`/`reset_to_v0`/`add_gamma`/`modify_alpha` order harness.
- Watch drills (I1/I4): `WatchChild` (50ms debounce, null stdio, kill-on-drop)
  + `wait_until_servable` (poll `status` file_count, 30s cap, never log text);
  I4 adds `poll_search_until` for daemon convergence.

## Counts

- By file: I1 12, I2 12, I3 10, I4 7. Total 41.
- By CAT: staleness 9, delta 12, parity 10, drill 7, other 3. Total 41.
- By VERDICT: KEEP 11, MERGE 29, DELETE 1. Total 41.
- KEEP (11): I1 reindex-selection, dry-run, status-discriminants, auto-index
  refresh, flag precedence, mutated-predicate, 2× watch gates (8); I3
  converged-fixpoint (1); I4 chained + watch-mode drills (2).
- MERGE fan-in (29): staleness-counts 3 (I1), targeted-refresh 3 (I1+I2+I3),
  search-delta + outline-delta 11 (I2), parity-incremental-vs-clean 2 (I3),
  parity-order-independence 2 (I3), parity-reindex 3 (I3), drill-lifecycle 5
  (I4).
- DELETE (1): I3 counts test, subsumed by sibling byte-identity tests.

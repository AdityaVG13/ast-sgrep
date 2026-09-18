# Invalidation codemode catalog (I1–I4)

Scope: `tests/codemode/invalidation_pass1.rs` (I1 session freshness contract), `invalidation_pass2.rs` (I2 per-delta-class consequences), `invalidation_pass3.rs` (I3 rebuild-parity relations), `invalidation_pass4.rs` (I4 end-to-end drills). 41 `#[test]`s, one bullet each, strict calibration: KEEP only standalone intents; every per-delta-class (add/modify/delete/rename) test merges into a per-surface delta-matrix intent; per-relation and per-kind drill tests merge into per-family matrices.

Matrix targets referenced by MERGE verdicts: `M-stale` = pinned-stale-until-stamp contract (bare-edit invisible, external epoch bump, reopen-fresh, status tracks live epoch); `M-render` = render-cache invalidation across triggers (external/delete/modify), never-serve-stale, repopulate; `M-writer` = writer refresh paths (targeted shape, force shape, edit-tool reindex); `M-pure` = non-writer stability (noop edit, pure tools leave epoch+cache untouched); `M-catalog` = catalog stability across writes and convergence across histories; `M-find` / `M-search` / `M-defs` = per-surface delta matrices (ADD/MODIFY/DELETE/RENAME legs); `M-census` = file_count census tracking deltas; `M-parity` = targeted≡rebuild≡fresh agreement; `M-order` = path/delta/rename order independence; `M-idem` = repeat-refresh idempotence; `M-conv` = transient/add-delete cycle convergence; `M-drill` = e2e serve→change→detect→refresh→serve loop per single change kind.

Overlap finding: no DELETEs — every apparent cross-pass overlap differs in trigger, surface, or asserted dimension (e.g. I1 `stale_render_cache_*` triggers on external write while I2 `delta_delete_refresh_drops_*` triggers on delete-refresh and I4 `drill_render_cache_*` on modify-refresh; I1 `edit_tool_*` pins the writer path via find only while I4 `drill_edit_tool_*` runs the full e2e loop across all surfaces). The two KEEPs are the only tests with no matrix siblings.

## Pass 1 — freshness contract (`tests/codemode/invalidation_pass1.rs`)

- bare_file_change_without_reindex_serves_stale_hits: INTENT=Bare disk edit without a writer leaves the epoch unmoved; pinned index serves stale ALPHA hits while BETA stays invisible; CAT=staleness; KILLS=auto-refresh-on-read/reopen-every-call mutants; VERDICT=MERGE→M-stale
- external_indexer_write_bumps_writer_generation: INTENT=Out-of-band Indexer write on the same DB stamps a new writer_generation epoch; CAT=staleness; KILLS=no-stamp-on-external-write mutants; VERDICT=MERGE→M-stale
- search_after_external_reindex_serves_fresh_hits: INTENT=Post-stamp search reopens the index and serves BETA hits while ALPHA evicts; CAT=staleness; KILLS=never-reopen-cached-Searcher mutants; VERDICT=MERGE→M-stale
- stale_render_cache_is_never_served_after_writer_change: INTENT=peek_cached_search answers None once the epoch moves, until a fresh search repopulates the cache; CAT=staleness; KILLS=serve-cache-regardless-of-epoch mutants; VERDICT=MERGE→M-render
- index_repo_targeted_refresh_returns_fresh_results: INTENT=Targeted paths refresh reports targeted shape plus stats and find serves the new token; CAT=delta; KILLS=targeted-refresh-skipped mutants; VERDICT=MERGE→M-writer
- index_repo_force_rebuild_refreshes_and_reports_full_shape: INTENT=Force rebuild reports full shape and find swaps the old token for the new; CAT=delta; KILLS=force-flag-ignored mutants; VERDICT=MERGE→M-writer
- index_repo_arg_conflicts_fail_with_other_discriminant: INTENT=force+paths, empty paths, and traversal fail as CallError::Other with epoch and warm cache untouched; CAT=other; KILLS=conflict-silent-ok/wrong-discriminant/failure-drops-cache mutants; VERDICT=KEEP
- edit_tool_reindexes_touched_paths: INTENT=Session edit mutates disk and reindexes so find swaps the old token for the new; CAT=delta; KILLS=edit-without-reindex mutants; VERDICT=MERGE→M-writer
- noop_edit_leaves_generation_and_cache_untouched: INTENT=Zero-change edit reports changed=0 with epoch and render cache untouched; CAT=staleness; KILLS=noop-bumps-epoch/noop-drops-cache mutants; VERDICT=MERGE→M-pure
- catalog_is_stable_across_index_writes: INTENT=Catalog names and read_only flags identical across all four writer paths; unknown name fails InvalidArgs; CAT=other; KILLS=read_only-flag-flip/unknown-not-InvalidArgs mutants (name-equality leg BEHAVIOR-ONLY: same static fn both sides); VERDICT=MERGE→M-catalog
- pure_tools_neither_bump_generation_nor_drop_cache: INTENT=catalog_search/describe, filter_hits, and select leave the epoch and warm cache untouched; CAT=staleness; KILLS=pure-tool-invalidates mutants; VERDICT=MERGE→M-pure
- index_status_reports_current_writer_generation: INTENT=index_status writer_generation tracks the live epoch across an external bump; CAT=staleness; KILLS=cached-or-stale-status-epoch mutants; VERDICT=MERGE→M-stale

## Pass 2 — delta classes (`tests/codemode/invalidation_pass2.rs`)

- delta_add_find_surfaces_new_file_token_only: INTENT=ADD plus refresh surfaces the new token in exactly the new file while the sibling token stays intact; CAT=delta; KILLS=add-invisible/spillover mutants; VERDICT=MERGE→M-find
- delta_add_search_word_query_surfaces_new_file: INTENT=ADD through hybrid word search resolves exactly the new file; CAT=delta; KILLS=search-misses-add mutants; VERDICT=MERGE→M-search
- delta_add_defs_lookup_finds_new_symbol: INTENT=ADD through defs resolves the new symbol while an unknown symbol resolves empty; CAT=delta; KILLS=defs-misses-add/phantom-def mutants; VERDICT=MERGE→M-defs
- delta_modify_find_swaps_token_exactly: INTENT=MODIFY swaps the find token exactly (new present, old gone); CAT=delta; KILLS=stale-linger/new-missing mutants; VERDICT=MERGE→M-find
- delta_modify_defs_lookup_tracks_renamed_symbol: INTENT=MODIFY renaming a definition moves defs lookup to the new name only; CAT=delta; KILLS=defs-stale-name mutants; VERDICT=MERGE→M-defs
- delta_delete_find_and_search_evict_token: INTENT=DELETE evicts the removed token from both find and search; CAT=delta; KILLS=delete-linger mutants; VERDICT=MERGE→M-find + M-search (DELETE legs)
- delta_delete_defs_lookup_empties: INTENT=DELETE of the defining file empties defs lookup; CAT=delta; KILLS=defs-delete-linger mutants; VERDICT=MERGE→M-defs
- delta_rename_refresh_moves_hits_to_new_path: INTENT=RENAME moves the token to the new path without duplication while the census stays at 1; CAT=delta; KILLS=rename-loses-hits/old-path-linger mutants; VERDICT=MERGE→M-find
- delta_catalog_file_count_tracks_add_and_delete: INTENT=file_count census moves 1→2→1 with an epoch bump per delta and reads agreeing with the census; CAT=delta; KILLS=census-not-tracked mutants; VERDICT=MERGE→M-census
- delta_delete_refresh_drops_stale_render_and_repopulates_fresh: INTENT=Delete refresh drops the warm render, post-delete search serves empty, and re-add repopulates the cache; CAT=staleness; KILLS=render-survives-delete mutants; VERDICT=MERGE→M-render

## Pass 3 — parity relations (`tests/codemode/invalidation_pass3.rs`)

- targeted_refresh_matches_force_rebuild_find: INTENT=Targeted refresh and force rebuild of the same final state agree on find bytes, counts, and census; CAT=parity; KILLS=targeted-vs-rebuild-divergence mutants; VERDICT=MERGE→M-parity
- incremental_vs_fresh_index_search_parity: INTENT=Incremental add+modify and a fresh index of the same tree agree on search and find bytes; CAT=parity; KILLS=incremental-divergence mutants; VERDICT=MERGE→M-parity
- incremental_vs_fresh_index_defs_and_census_parity: INTENT=Incremental vs fresh agree on defs bytes/counts and file census; CAT=parity; KILLS=defs-or-census-divergence mutants; VERDICT=MERGE→M-parity
- refresh_path_order_does_not_change_results: INTENT=Multi-path refresh in opposite orders converges on find results and census; CAT=parity; KILLS=path-order-dependent mutants; VERDICT=MERGE→M-order
- delta_application_order_does_not_change_results: INTENT=Modify-then-add vs add-then-modify serve identical find/search results and census; CAT=parity; KILLS=delta-order-dependent mutants; VERDICT=MERGE→M-order
- transient_add_delete_cycle_is_unobservable_in_final_state: INTENT=A churned session converges to one that never saw the transient file; CAT=parity; KILLS=transient-residue mutants; VERDICT=MERGE→M-conv
- repeated_targeted_refresh_is_idempotent_for_reads: INTENT=Repeating a multi-path refresh leaves find bytes/counts and census fixed; CAT=parity; KILLS=repeat-mutates mutants; VERDICT=MERGE→M-idem
- repeated_delete_refresh_is_idempotent: INTENT=Refreshing an already-evicted path keeps reads empty and the census at 0; CAT=parity; KILLS=repeat-delete-churn mutants; VERDICT=MERGE→M-idem
- rename_refresh_path_order_converges: INTENT=Rename old+new refresh in either order moves the token identically to exactly beta.py; CAT=parity; KILLS=rename-order-dependent mutants; VERDICT=MERGE→M-order
- catalog_converges_across_divergent_refresh_histories: INTENT=Divergent histories over the same tree expose identical catalog names/flags and census; CAT=parity; KILLS=history-dependent-catalog mutants; VERDICT=MERGE→M-catalog
- add_then_delete_cycle_returns_to_baseline_reads: INTENT=An add/delete cycle returns find/search bytes and census to baseline; CAT=parity; KILLS=cycle-residue mutants; VERDICT=MERGE→M-conv

## Pass 4 — end-to-end drills (`tests/codemode/invalidation_pass4.rs`)

- drill_add_change_detect_refresh_serve: INTENT=E2E ADD: baseline serve, stale detect (epoch frozen, new token invisible), targeted refresh, exact multi-surface serve with census 2; CAT=drill; KILLS=add-loop-break mutants; VERDICT=MERGE→M-drill
- drill_modify_change_detect_refresh_serve: INTENT=E2E MODIFY: pinned old-token serve pre-refresh, refresh, exact all-surface swap with census 1; CAT=drill; KILLS=modify-loop-break mutants; VERDICT=MERGE→M-drill
- drill_delete_change_detect_refresh_serve: INTENT=E2E DELETE: deleted token lingers pre-refresh, refresh evicts it from every tool with census 0; CAT=drill; KILLS=delete-loop-break mutants; VERDICT=MERGE→M-drill
- drill_rename_change_detect_refresh_serve: INTENT=E2E RENAME: stale old-path serve, two-sided refresh, token moved exactly to the new path via every tool; CAT=drill; KILLS=rename-loop-break mutants; VERDICT=MERGE→M-drill
- drill_edit_tool_change_refresh_serve: INTENT=E2E session-edit loop: edit mutates and refreshes inline with an epoch move, exact all-surface swap; CAT=drill; KILLS=edit-loop-break mutants; VERDICT=MERGE→M-drill
- drill_force_rebuild_change_detect_refresh_serve: INTENT=E2E force rebuild absorbs unrefreshed modify+add into the exact final multi-surface state with census 2; CAT=drill; KILLS=rebuild-loop-break mutants; VERDICT=MERGE→M-drill
- drill_render_cache_freshness_across_refresh: INTENT=E2E render-cache loop: warm cache answers, goes stale post-change, refresh drops it, fresh empty search repopulates; CAT=drill; KILLS=render-loop-break mutants; VERDICT=MERGE→M-render
- drill_chained_multi_change_add_modify_delete_rename: INTENT=Four-link add→modify→delete→rename chain with stale-detect plus epoch-move per link and exact final cross-tool serve; CAT=drill; KILLS=multi-link-interaction/sequence mutants; VERDICT=KEEP

## Helper patterns

- `setup()` (+ `session_for`): fresh TempDir root and index DB, one `alpha.py` fixture indexed, session with `limit: 8, use_embed: false` — identical shape in all four files (pass1 returns the token-parametrized variant).
- `write_py` / `write_fixture`: single-`def` Python fixture keyed by unique `snorkel_*` tokens (ALPHA/BETA/GAMMA/DELTA/NEWDEF/OLDDEF) so every delta is token-addressable.
- `refresh(session, paths)`: one-call `index_repo` with `paths` (pass2–4); pass1 additionally uses `external_reindex` (second `Indexer` on canonicalized root + `update_paths` + `flush_deferred_rebuilds`) to simulate out-of-band writers.
- Read wrappers `find` / `search` / `defs` (pass3–4) plus `file_count` / `writer_generation` via `index_status` (pass1 reads the epoch via `read_writer_generation` instead).
- Hit assertions: `hit_file_set` (pass2) / `hit_name_set` basenames for cross-tempdir comparison (pass3–4), `hit_bytes` byte-stable set encoding for exact parity, `hit_count`, and `hits_file` suffix matching (pass1–2).
- Cache probes: `matches!(session.peek_cached_search(&args), Some/None)` pins render-cache presence without serving stale content.
- Discriminant discipline: `matches!` on `CallError`, typed JSON accessors, counts, and hit-set bytes only — no error-message text matched anywhere.

## Counts

- Total: 41 tests (pass1: 12, pass2: 10, pass3: 11, pass4: 8).
- By CAT: staleness 8, delta 12, parity 11, drill 8, other 2.
- By VERDICT: KEEP 2 (`index_repo_arg_conflicts_fail_with_other_discriminant`, `drill_chained_multi_change_add_modify_delete_rename`), MERGE 39, DELETE 0.
- MERGE targets: M-stale 4, M-render 3, M-writer 3, M-pure 2, M-catalog 2, M-find 4 (incl. shared DELETE leg), M-search 2 (incl. shared DELETE leg), M-defs 3, M-census 1, M-parity 3, M-order 3, M-idem 2, M-conv 2, M-drill 6.

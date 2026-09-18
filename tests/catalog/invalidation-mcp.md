# Invalidation MCP catalog (I1–I4)

Scope: `tests/mcp/invalidation_pass1.rs` (I1 broad invalidation contract), `invalidation_pass2.rs` (I2 per-delta-class oracles), `invalidation_pass3.rs` (I3 rebuild-parity relations), `invalidation_pass4.rs` (I4 end-to-end drills). 37 `#[test]`s, one bullet each, strict calibration: KEEP only standalone intents; every per-delta-class (add/modify/delete/rename) test merges into a per-surface delta-matrix intent.

Matrix targets referenced by MERGE verdicts: `M1-search` = keyword_search hit-path sets × {add, modify, modify-move, delete, rename}; `M2-status` = index_status transitions + index_repo refresh stats × {add, modify, delete, rename}; `M3-multi` = multi/mixed single-refresh exact sets + stats; `M4-parity` = incremental ≡ fresh ≡ force byte/count agreement; `M5-order` = delta-order/interleave independence; `M6-idempotent` = redundant-refresh noop + status convergence; `M7-drill` = full change→detect→refresh→serve arc × {add, modify, delete, rename} in one live session.

## Pass 1 — broad contract (`tests/mcp/invalidation_pass1.rs`)

- index_status_discriminates_missing_unindexed_and_fresh: INTENT=index_status three-way discriminant: missing-root tool error, unindexed zeros, fresh counts+nonzero epoch; CAT=other; KILLS=status-discriminant mutant (missing→success, unindexed-nonzero, fresh-zero-epoch); VERDICT=KEEP
- search_on_stale_index_serves_pre_change_hits_without_flag: INTENT=stale index serves pre-change hits byte-identically with success shape, no staleness keys, frozen epoch; CAT=staleness; KILLS=stale-visibility mutant (flag keys added, fresh served early, epoch bumped on bare edit); VERDICT=KEEP
- in_session_reindex_then_search_reflects_changes: INTENT=in-session index_repo flips a modify delta (old symbol misses, new hits); CAT=delta; KILLS=in-session-refresh-noop mutant; VERDICT=MERGE→M7-drill
- external_reindex_invalidates_warm_session_searcher: INTENT=out-of-band reindex invalidates a warm same-session Searcher (serves fresh rows); CAT=staleness; KILLS=warm-Searcher-snapshot mutant (generation advance ignored); VERDICT=KEEP
- restart_picks_up_fresh_state: INTENT=restarted process observes post-mutation epoch+rows, deterministic across restarts; CAT=staleness; KILLS=restart-stale-generation mutant; VERDICT=KEEP
- session_and_fresh_process_agree_after_reindex: INTENT=post-reindex in-session answers agree byte-identically with a fresh process; CAT=parity; KILLS=session-vs-process divergence mutant; VERDICT=MERGE→M4-parity
- index_repo_heals_empty_index_miss_within_session: INTENT=in-session index_repo heals an empty_index miss (miss→stats→hit→count=1); CAT=staleness; KILLS=empty-heal mutant (index_repo noop on empty, wrong miss code); VERDICT=KEEP
- reindex_after_deletion_prunes_counts_and_hits: INTENT=reindex after delete prunes rows+counts (removed=1, indexed=0, deleted misses, kept hits); CAT=delta; KILLS=delete-row-leak mutant; VERDICT=MERGE→M2-status

## Pass 2 — delta classes (`tests/mcp/invalidation_pass2.rs`)

- add_delta_new_file_hits_appear_only_after_refresh: INTENT=ADD: new file invisible until refresh, then exact new+old path sets; CAT=delta; KILLS=unindexed-serve/old-path-drop mutant; VERDICT=MERGE→M1-search
- add_delta_status_counts_and_refresh_stats: INTENT=ADD: bare add moves no discriminant; refresh stats (1,0,0), count+1, epoch advances; CAT=delta; KILLS=add-stats/count mutant; VERDICT=MERGE→M2-status
- modify_delta_symbol_swap_exact_path_sets: INTENT=MODIFY: old symbol misses, new hits edited path, untouched exact; stats (1,0,0); CAT=delta; KILLS=modify-swap mutant (stale old hit, new miss, wrong path); VERDICT=MERGE→M1-search
- modify_delta_symbol_move_stale_path_set_then_fresh: INTENT=MODIFY-move: stale serves pre-move path with frozen discriminants, fresh serves post-move path only; CAT=delta; KILLS=modify-move path-set mutant; VERDICT=MERGE→M1-search
- delete_delta_pruned_hits_exact_paths: INTENT=DELETE: stale row serves old path until refresh, then misses; kept symbol exact; CAT=delta; KILLS=delete-row-leak mutant; VERDICT=MERGE→M1-search
- delete_delta_status_transition_and_refresh_stats: INTENT=DELETE: bare delete moves nothing; stats (0,1,0), count-1, epoch advances; CAT=delta; KILLS=delete-stats/count mutant; VERDICT=MERGE→M2-status
- rename_delta_old_path_gone_new_path_hit: INTENT=RENAME: stale old path until refresh; fresh hits new path only, old leaves p-table; CAT=delta; KILLS=rename-path mutant (old lingers, new missing); VERDICT=MERGE→M1-search
- rename_delta_status_file_count_stable: INTENT=RENAME: bare rename moves nothing; stats (1,1,0), count+symbols stable, epoch advances; CAT=delta; KILLS=rename-stats mutant (count/symbol drift); VERDICT=MERGE→M2-status
- add_multiple_delta_single_refresh_exact_hit_sets: INTENT=MULTI-ADD: one refresh absorbs two adds with exact per-path sets, stats (2,0,0); CAT=delta; KILLS=multi-add cross-contamination mutant; VERDICT=MERGE→M3-multi
- mixed_add_delete_delta_single_refresh_exact_hit_sets: INTENT=MIXED add+delete: one refresh, stats (1,1,0), exact miss/hit/kept sets; CAT=delta; KILLS=mixed-delta misattribution mutant; VERDICT=MERGE→M3-multi

## Pass 3 — parity relations (`tests/mcp/invalidation_pass3.rs`)

- incremental_refresh_matches_fresh_build_after_modify: INTENT=incremental refresh vs fresh build agree byte-identically after modify; CAT=parity; KILLS=delta-history-leak mutant (incremental≠fresh rows/counts); VERDICT=MERGE→M4-parity
- incremental_refresh_matches_fresh_build_after_mixed_deltas: INTENT=incremental vs fresh agree byte-identically after mixed modify+delete+add; CAT=parity; KILLS=delta-history-leak mutant; VERDICT=MERGE→M4-parity
- force_rebuild_preserves_search_bytes_and_counts: INTENT=force rebuild over the same tree preserves search bytes and counts exactly; CAT=parity; KILLS=force-rebuild content mutant; VERDICT=MERGE→M4-parity
- force_refresh_from_stale_matches_incremental_refresh: INTENT=force-from-stale vs incremental refresh agree byte-identically; CAT=parity; KILLS=force-vs-incremental divergence mutant; VERDICT=MERGE→M4-parity
- add_order_independent_under_single_refresh: INTENT=two adds in opposite order converge to identical bytes+counts after one refresh each; CAT=parity; KILLS=order-dependence mutant; VERDICT=MERGE→M5-order
- mixed_add_delete_order_independent: INTENT=delete+add in opposite order converge to identical bytes+counts; CAT=parity; KILLS=order-dependence mutant; VERDICT=MERGE→M5-order
- modify_add_interleave_order_independent: INTENT=modify+add in opposite interleave converge to identical bytes+counts; CAT=parity; KILLS=order-dependence mutant; VERDICT=MERGE→M5-order
- refresh_twice_search_bytes_identical: INTENT=second refresh with no tree change leaves hit+miss bytes identical; CAT=parity; KILLS=redundant-refresh-content mutant; TAUTOLOGY-RISK+tail assert compares search_text to itself; VERDICT=MERGE→M6-idempotent
- second_refresh_without_changes_is_content_noop: INTENT=redundant refresh yields zero-mutation stats and converged counts; CAT=parity; KILLS=redundant-refresh-stats mutant (nonzero stats on noop); VERDICT=MERGE→M6-idempotent
- index_status_byte_stable_across_repeated_calls: INTENT=repeated index_status calls with no refresh are byte-identical; CAT=parity; KILLS=nondeterministic-status-serialization mutant; VERDICT=MERGE→M6-idempotent
- index_status_counts_converge_across_repeated_refresh: INTENT=three refreshes over one delta converge counts+bytes at once and stay converged; CAT=parity; KILLS=non-convergence mutant; VERDICT=MERGE→M6-idempotent

## Pass 4 — drills (`tests/mcp/invalidation_pass4.rs`)

- drill_add_change_detect_refresh_serve: INTENT=full ADD arc in one session: miss-detect, byte-stable status, stats, gen advance, exact fresh sets; CAT=drill; KILLS=arc-phase-skip mutant (add column); VERDICT=MERGE→M7-drill
- drill_modify_change_detect_refresh_serve: INTENT=full MODIFY arc in one session: stale-bytes detect, refresh, old-miss/new-hit; CAT=drill; KILLS=arc-phase-skip mutant (modify column); VERDICT=MERGE→M7-drill
- drill_delete_change_detect_refresh_serve: INTENT=full DELETE arc in one session: stale-bytes detect, stats (0,1,0), kept byte-identical; CAT=drill; KILLS=arc-phase-skip mutant (delete column); VERDICT=MERGE→M7-drill
- drill_rename_change_detect_refresh_serve: INTENT=full RENAME arc in one session: old-path stale detect, stats (1,1,0), new-path serve; CAT=drill; KILLS=arc-phase-skip mutant (rename column); VERDICT=MERGE→M7-drill
- drill_modify_twice_single_refresh_serves_final_only: INTENT=two successive modifies: one refresh serves the final revision only, intermediate never visible; CAT=drill; KILLS=intermediate-revision-serve mutant; VERDICT=KEEP
- drill_add_then_delete_before_refresh_is_net_zero: INTENT=add-then-delete before refresh: zero-mutation stats, serve identical to baseline; CAT=drill; KILLS=cancel-delta mutation mutant (nonzero stats, changed serve); VERDICT=KEEP
- drill_rename_chain_single_refresh_serves_final_path: INTENT=rename chain (a→b→c): one refresh collapses to single remove+add serving the final path; CAT=drill; KILLS=chain-collapse mutant (intermediate path served, wrong stats); VERDICT=KEEP
- drill_chained_add_modify_delete_rename_in_one_session: INTENT=four chained arcs in one session with per-link gen advance and exact final serve; CAT=drill; KILLS=chained-link state-bleed mutant; VERDICT=KEEP

## Helper patterns

All four files duplicate the same stdio harness (~230 lines each, no shared crate; clientInfo name differs per pass: asgrep-mcp-i1..i4). Shared helpers: `mcp_bin()` locates the asgrep-mcp binary (CARGO_BIN_EXE_asgrep-mcp or target profile dir); `init_payload()` builds the JSON-RPC initialize handshake; `rpc_session(payloads, root)` drives sequential send-one/read-one through one server process with ASGREP_ROOT set; `rpc_at(payload, root)` is the single-call fresh-process wrapper; `LiveSession` (pass1/2/4 only; pass3 uses fresh processes throughout) holds a persistent stdio session with `call`/`finish` plus `search_text`/`status_text`/`refresh` convenience methods in pass4; `tool_call`/`tool_text`/`tool_body` build and parse tool envelopes; `assert_tool_success` and (pass1 only) `assert_tool_error_shape` pin envelope discriminants; `assert_hit_envelope`/`assert_miss_envelope(why)` pin hit/miss shapes (no-why + zn≥1 + nonempty h vs why + zn=0 + empty h); `index_tree` runs an out-of-band Indexer::index_all; `search_call` builds keyword_search with resend_seen. Pass2/4 add `hit_path_set`/`assert_hit_path_set`, which resolve hits through the compact p table via resolve_compact_paths and pin zn==row-count plus table==hit-paths (the old-path-leaves-table discriminant). Pass3 adds `refresh`/`force_refresh`/`status_body`/`status_text` plus `assert_same_counts` (file/line/symbol only) and `assert_nonzero_generation`, encoding the generation-is-liveness-not-digest rule. Pass4 adds `parse` (text→Value).

## Counts

Total 37 `#[test]`s: KEEP 9, MERGE 28, DELETE 0. By file: pass1 8 (5 KEEP, 3 MERGE), pass2 10 (0 KEEP, 10 MERGE), pass3 11 (0 KEEP, 11 MERGE), pass4 8 (4 KEEP, 4 MERGE). By CAT: staleness 4, delta 12, parity 12, drill 8, other 1. By merge target: M1-search 5, M2-status 4, M3-multi 2, M4-parity 5, M5-order 3, M6-idempotent 4, M7-drill 5. One TAUTOLOGY-RISK flag (single self-compare tail assert in refresh_twice_search_bytes_identical; test intent still merges).

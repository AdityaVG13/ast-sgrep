# Tokio catalog (37 tests, agent: orchestrator — capacity slot unavailable)

## tests/mcp/tokio_pass1.rs (K1 concurrency, 10 tests)

- pipelined_tool_batch_all_answered_with_matching_ids: INTENT=pipelined requests are all answered with matching ids; CAT=concurrency; KILLS=drop-reorder mutants (id mismatch, lost response); VERDICT=MERGE→pipeline-integrity (facet: tool-only batch)
- pipelined_distinct_searches_have_no_crosstalk: INTENT=concurrent distinct searches never exchange results; CAT=concurrency; KILLS=shared-buffer/crosstalk mutants; VERDICT=KEEP
- pipelined_error_does_not_poison_batch_or_session: INTENT=one bad call fails alone, batch and session continue; CAT=concurrency; KILLS=abort-on-first-error/session-poison mutants; VERDICT=KEEP
- rapid_fire_identical_calls_answered_identically: INTENT=repeat determinism under rapid fire; CAT=concurrency; KILLS=nondeterminism/state-leak mutants; VERDICT=MERGE→repeat-determinism (with K3 same-script tests)
- ping_answered_while_slow_tool_runs: INTENT=reader serves ping while a tool holds SQLite (the core tokio contract); CAT=concurrency; KILLS=lock-removal/reader-block mutants; RACY-RESILIENT (generous timeout, 4x green); VERDICT=KEEP
- tools_list_succeeds_during_active_tool_call: INTENT=list_tools served during active tool; CAT=concurrency; KILLS=same class as ping-during-tool; VERDICT=MERGE→served-during-activity (facet: tools/list; with ping test)
- mixed_method_pipeline_all_matched_by_id: INTENT=mixed-method pipelines match by id; CAT=concurrency; KILLS=drop-reorder mutants; VERDICT=MERGE→pipeline-integrity (facet: mixed methods)
- concurrent_writer_threads_all_answered: INTENT=server correct under threaded concurrent stdin writers; CAT=concurrency; KILLS=write-interleave/framing mutants; RACY-RESILIENT; VERDICT=KEEP
- session_survives_back_to_back_batches: INTENT=multi-batch sessions stay correct; CAT=concurrency; KILLS=session-state-rot mutants; VERDICT=MERGE→session-durability (facet: back-to-back batches)
- session_survives_initialize_calls_more_calls: INTENT=extended sessions stay correct; CAT=concurrency; KILLS=session-state-rot mutants; VERDICT=MERGE→session-durability (facet: extended script)

## tests/mcp/tokio_pass2.rs (K2 cancellation, 10 tests)

- eof_before_initialize_exits_ok: INTENT=EOF before init is clean Ok, not failure; CAT=cancel; KILLS=exit-code mutants; VERDICT=MERGE→eof-abort-handling (facet: pre-init)
- eof_mid_session_idle_exits_ok: INTENT=idle EOF exits Ok; CAT=cancel; KILLS=hang-on-eof mutants; VERDICT=MERGE→eof-abort-handling (facet: idle)
- client_abort_mid_call_exits_ok: INTENT=stdin close mid-call terminates cleanly; CAT=cancel; KILLS=hang/poison mutants; VERDICT=MERGE→eof-abort-handling (facet: mid-call abort)
- eof_immediately_after_cancel_exits_ok: INTENT=EOF right after cancel exits Ok; CAT=cancel; KILLS=cancel-then-hang mutants; VERDICT=MERGE→eof-abort-handling (facet: post-cancel)
- cancel_mid_call_suppresses_response: INTENT=in-flight cancel suppresses the response; CAT=cancel; KILLS=cancel-ignored mutants; RACY-RESILIENT; VERDICT=KEEP (anchor of cancel-suppression intent)
- cancel_queued_call_behind_slow_tool: INTENT=queued (not yet running) call cancelled; CAT=cancel; KILLS=queue-cancel-ignored mutants; VERDICT=MERGE→cancel-suppression (facet: queued)
- cancel_one_queued_call_leaves_sibling_intact: INTENT=cancel is selective, siblings unaffected; CAT=cancel; KILLS=cancel-all/cancel-wrong-id mutants; VERDICT=MERGE→cancel-suppression (facet: selectivity)
- session_usable_after_cancel_reindex_and_search: INTENT=post-cancel session fully usable; CAT=cancel; KILLS=cancel-poisons-session mutants; VERDICT=KEEP
- ping_and_list_served_during_cancel_window: INTENT=non-tool requests served during cancel; CAT=cancel; KILLS=same as K1 served-during-activity; VERDICT=MERGE→served-during-activity (facet: cancel window)
- stray_cancels_are_silent_and_harmless: INTENT=unknown-id cancels ignored silently; CAT=cancel; KILLS=error-on-unknown-cancel mutants; VERDICT=KEEP

## tests/mcp/tokio_pass3.rs (K3 metamorphic, 10 tests)

- sequential_vs_pipelined_tools_equivalent: INTENT=delivery order never changes results (tools); CAT=metamorphic; KILLS=order-dependent mutants; VERDICT=MERGE→seq-vs-pipe-equivalence (facet: tools)
- sequential_vs_pipelined_mixed_methods_equivalent: INTENT=same for mixed methods; CAT=metamorphic; KILLS=order-dependent mutants; VERDICT=MERGE→seq-vs-pipe-equivalence (facet: mixed)
- sequential_vs_pipelined_errors_equivalent: INTENT=same for error calls; CAT=metamorphic; KILLS=order-dependent error mutants; VERDICT=MERGE→seq-vs-pipe-equivalence (facet: errors)
- fresh_servers_same_transcript_byte_identical: INTENT=restart determinism; CAT=metamorphic; KILLS=state-leak-across-restart mutants; VERDICT=MERGE→restart-equivalence (facet: fresh servers)
- same_script_twice_in_one_session_identical: INTENT=in-session repeat determinism; CAT=metamorphic; KILLS=in-session state-rot mutants; VERDICT=MERGE→repeat-determinism (facet: same session)
- same_pipelined_batch_twice_across_restart_identical: INTENT=restart determinism for batches; CAT=metamorphic; KILLS=state-leak mutants; VERDICT=MERGE→restart-equivalence (facet: batch across restart)
- soak_sixty_mixed_calls_all_correct: INTENT=60-call soak stays correct; CAT=metamorphic; KILLS=leak/degradation mutants; VERDICT=KEEP (anchor of soak intent)
- soak_repeated_search_byte_stable: INTENT=soak byte stability; CAT=metamorphic; KILLS=drift mutants; VERDICT=MERGE→soak (facet: byte stability)
- ping_interleaved_never_perturbs_tools: INTENT=interleaved ping never perturbs; CAT=metamorphic; KILLS=interleave-corruption mutants; VERDICT=MERGE→interleave-stability (facet: ping)
- tools_list_interleaved_never_perturbs_tools: INTENT=interleaved list never perturbs; CAT=metamorphic; KILLS=interleave-corruption mutants; VERDICT=MERGE→interleave-stability (facet: list)

## tests/mcp/tokio_pass4.rs (K4 drills, 7 tests)

- burst_24_mixed_calls_all_correct_id_ordered: INTENT=24-call burst correct and id-ordered; CAT=runtime-drill; KILLS=burst-drop/reorder mutants; VERDICT=KEEP (anchor of burst intent)
- interleave_ping_list_cancel_leaves_tools_unperturbed: INTENT=interleave incl cancel unperturbing (overlaps K3.9/10 + cancel facet); CAT=runtime-drill; KILLS=interleave-corruption mutants; VERDICT=MERGE→interleave-stability (facet: with cancel)
- abort_stdin_slammed_mid_burst_exits_ok: INTENT=mid-burst abort exits Ok (stronger K2.3); CAT=runtime-drill; KILLS=abort-hang mutants; VERDICT=MERGE→eof-abort-handling (facet: mid-burst slam)
- restart_kill9_fresh_server_serves_identical_baseline: INTENT=SIGKILL recovery serves identical baseline; CAT=runtime-drill; KILLS=unclean-restart mutants; VERDICT=KEEP
- slow_client_byte_trickle_assembles_correct_requests: INTENT=framing robust to byte-trickle writes; CAT=runtime-drill; KILLS=framing-assumption mutants; VERDICT=KEEP
- mixed_chaos_burst_interleave_cancel_restart_final_state_correct: INTENT=chaos composition converges to correct state; CAT=runtime-drill; KILLS=composition mutants; VERDICT=KEEP
- burst_errors_and_stray_cancels_exact_count: INTENT=burst error accounting exact; CAT=runtime-drill; KILLS=error-miscount mutants; VERDICT=MERGE→burst (facet: error accounting)

## Helper patterns (testkit candidates)

- LiveSession stdio harness + payload builders (mcp_bin/init_payload/tool_call/tools_list/ping/cancelled) + response asserts (tool_text/tool_body/assert_tool_success/assert_ping_ok/assert_tools_list_ok/expected_tool_names) + tree builders (small_tree/big_tree) duplicated ~300 lines in EACH of the 4 files (also in oracle/recovery/errorapi/invalidation/numerical mcp suites) → testkit::mcp_session module (biggest win in the repo).
- collect/by_id/run_sequential/run_pipelined/canon comparators (pass3/4) → testkit::mcp_session script-comparison helpers.
- status_counts/assert_status_counts_eq (pass3/4) → testkit helper.

## Proposed structure

- tests/mcp/tokio_concurrency.rs: pipeline-integrity, no-crosstalk, error-isolation, served-during-activity, writer-threads, session-durability (6 tests)
- tests/mcp/tokio_cancellation.rs: cancel-suppression (in-flight+queued+selective), post-cancel-usability, stray-cancels, eof-abort-handling (4 tests)
- tests/mcp/tokio_equivalence_chaos.rs: seq-vs-pipe, repeat-determinism, restart-equivalence, soak, interleave-stability, burst, kill9-restart, trickle, chaos (9 tests)
- Total: ~19 intent tests replacing 37; harness collapses into testkit.

## Counts

- Total 37 / KEEP 12 / MERGE 25 / DELETE 0. No pure tautologies found; fluff is fragmentation, not emptiness.

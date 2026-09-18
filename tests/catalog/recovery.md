# Recovery catalog (155 #[test] fns: 153 tests + 2 child-entry harness fns)

## tests/cli/durable_recovery_pass1.rs (R1 static contracts, 11 tests)

- readers_fail_closed_on_corrupt_index_db: INTENT=readers (status/outline/passive search) refuse corrupt db with exit-2/operational, never silent-empty; CAT=fail-closed; KILLS=silent-empty/walk-downgrade mutants; VERDICT=KEEP (anchor: reader fail-closed)
- doctor_reports_corrupt_and_missing_index_as_index_open: INTENT=doctor reports corrupt/missing db as index_open, healthy:false, null status, exit 2; CAT=fail-closed; KILLS=ok-true/wrong-kind mutants; VERDICT=KEEP (anchor: doctor envelope)
- incremental_index_refuses_corrupt_db_while_reindex_heals: INTENT=incremental writer refuses corrupt db (no quarantine) while reindex heals + quarantines + serves; CAT=fail-closed; KILLS=refuse-skip/quarantine-skip mutants; VERDICT=KEEP (anchor: writer boundary; absorbs R2 torn facet)
- stale_lock_and_temp_crash_debris_do_not_block_reindex: INTENT=garbage lock + stale tmp debris never block reindex; CAT=fail-closed; KILLS=lock-content-honoring mutants; VERDICT=MERGE→stale-lock-never-blocks (facet: static debris; with R2 dead-pid arm)
- writer_generation_stamp_missing_or_corrupt_is_cold_start_not_error: INTENT=corrupt/missing stamp cold-starts, recreates numeric nonzero epoch; CAT=fail-closed; KILLS=stamp-gating mutants; VERDICT=KEEP (anchor: stamp cold-start)
- corrupt_semantic_ivf_degrades_search_and_rebuilds_on_mutation: INTENT=garbage IVF degrades search (exit 0, hits intact), mutation rebuilds valid sidecar; CAT=fail-closed; KILLS=refuse-on-cache-corrupt mutants; VERDICT=MERGE→ivf-degrade-rebuild (facet: garbage bytes; with R2 torn arm) [R1-vs-R2 duplicate]
- corrupt_lexical_cache_fails_writer_but_not_reader: INTENT=corrupt lexical.db fails writer (exit 2) while status/search degrade past it; CAT=fail-closed; KILLS=split-contract mutants; VERDICT=MERGE→lexical-split-contract (facet: garbage; with R2 torn arm) [R1-vs-R2 duplicate]
- bench_history_missing_file_and_dir_are_rebuilt: INTENT=missing history file/dirs rebuild with establish_baseline, schema v1; CAT=fail-closed; KILLS=refuse-on-missing mutants; VERDICT=KEEP (anchor: history rebuild)
- unwritable_and_missing_dirs_error_cleanly: INTENT=bench into read-only history dir + status on missing root exit 2; CAT=fail-closed; KILLS=panic/partial-write mutants; VERDICT=MERGE→fault_readonly_index_home (facet: history-dir + missing-root arms)
- corrupt_committed_prior_fails_open_to_baseline: INTENT=corrupt keep-gate prior fails open to establish_baseline (control arm proves consultation), bytes preserved; CAT=fail-closed; KILLS=fail-closed-on-prior mutants; VERDICT=KEEP (anchor: fail-open contrast)
- install_config_missing_rebuilt_corrupt_refused: INTENT=missing agent config created, corrupt config refused with bytes kept; CAT=fail-closed; KILLS=overwrite-corrupt mutants; VERDICT=KEEP (only install-config test)

## tests/cli/durable_recovery_pass2.rs (R2 fault injection, 12 tests)

- fault_sigkill_mid_cache_write_next_run_recovers_or_fails_closed: INTENT=SIGKILL mid-cache-rewrite → next run serves intact or refuses (exit 0/2 only), reindex converges; CAT=fault-injection; KILLS=lying-envelope/exit-1 mutants; VERDICT=KEEP (anchor: live SIGKILL)
- fault_torn_truncated_index_db_refused_then_healed: INTENT=torn db refused (inode untouched) then healed with torn bytes quarantined; CAT=fault-injection; KILLS=same class as R1 writer-boundary; VERDICT=MERGE→incremental_index_refuses_corrupt_db_while_reindex_heals (facet: torn bytes) [R1-vs-R2 duplicate]
- fault_zero_length_index_db_rebuilt_as_cold_start: INTENT=zero-length db cold-starts via index (no quarantine), converges; CAT=fault-injection; KILLS=quarantine-empty/refuse-empty mutants; VERDICT=MERGE→drill_deleted_index_db_serve_parity (facet: zero-length arm; same cold-start family)
- fault_torn_truncated_lexical_cache_split_contract: INTENT=torn lexical.db keeps split contract (writer 2, readers 0), delete+index rebuilds; CAT=fault-injection; KILLS=split-contract mutants; VERDICT=MERGE→lexical-split-contract (facet: torn; with R1 garbage arm) [R1-vs-R2 duplicate]
- fault_torn_truncated_semantic_ivf_degrades_then_rebuilds: INTENT=torn IVF degrades (bytes untouched), mutation rebuilds valid sidecar; CAT=fault-injection; KILLS=refuse-on-cache-corrupt mutants; VERDICT=MERGE→ivf-degrade-rebuild (facet: torn; with R1 garbage arm) [R1-vs-R2 duplicate]
- fault_torn_truncated_bench_history_fails_loud_preserves_bytes: INTENT=torn aggregate fails loud, bytes preserved, removal re-establishes baseline; CAT=fault-injection; KILLS=reset-evidence mutants; VERDICT=MERGE→relation_corrupt_recover_verify_roundtrip_bench_history (R3 is a strict superset)
- fault_stale_dead_pid_lock_does_not_block_reindex: INTENT=dead-pid lock record never blocks reindex (flock, not pid); CAT=fault-injection; KILLS=pid-honoring mutants; VERDICT=MERGE→stale-lock-never-blocks (facet: dead-pid arm; with R1 debris arm) [R1-vs-R2 duplicate]
- fault_live_lock_holder_blocks_or_refuses_never_corrupts: INTENT=live flock holder blocks reindex, release heals without corruption; CAT=fault-injection; KILLS=lock-ignoring mutants; VERDICT=KEEP (only live-flock test)
- fault_lock_path_occupied_by_directory_fails_closed: INTENT=lock path as directory fails closed, inode untouched, removal heals; CAT=fault-injection; KILLS=proceed-without-lock mutants; VERDICT=KEEP (unique obstruction shape; no merge target)
- fault_readonly_index_home_fails_closed_exit_2: INTENT=read-only home refuses writes+reads, read-only db file still serves, writability restores fully; CAT=fault-injection; KILLS=file/dir-split mutants; VERDICT=KEEP (anchor: readonly file/dir split, 3 arms)
- fault_killed_watch_resumes_via_next_index: INTENT=killed watch loop resumes via plain index (no reindex), raced edit searchable; CAT=fault-injection; KILLS=resume-requires-reindex mutants; VERDICT=KEEP (only watch-resume test)
- fault_occupied_quarantine_slot_allocates_unique_quarantine: INTENT=pre-occupied quarantine slot forces unique .corrupt.1, sentinel intact, rebuild converges; CAT=fault-injection; KILLS=overwrite-evidence mutants; VERDICT=KEEP (anchor: quarantine uniqueness; distinct from R3 chain-growth)

## tests/cli/durable_recovery_pass3.rs (R3 relations, 11 tests)

- relation_noop_double_index_caches_byte_identical: INTENT=no-op index leaves db+lexical+IVF byte-identical (stamp excepted by shape); CAT=metamorphic; KILLS=gratuitous-rewrite mutants; VERDICT=KEEP (anchor: byte-idempotence)
- relation_double_reindex_observable_idempotence: INTENT=double reindex identical over status/files_indexed/answers; CAT=metamorphic; KILLS=non-idempotent-rebuild mutants; VERDICT=KEEP (anchor: observable idempotence)
- relation_quarantine_bytes_stable_no_new_slot_on_healthy_rebuild: INTENT=healthy rebuild neither rewrites quarantine nor takes a new slot; CAT=metamorphic; KILLS=rewrite-evidence mutants; VERDICT=KEEP (anchor: evidence stability)
- relation_corrupt_recover_verify_roundtrip_index: INTENT=corrupt→refuse→heal restores status snapshot + answers, quarantine preserves bytes; CAT=metamorphic; KILLS=lossy-recovery mutants; VERDICT=MERGE→drill_corrupt_index_db_serve_parity (R4 is a strict superset: adds outline)
- relation_corrupt_recover_verify_roundtrip_lexical_cache: INTENT=lexical corrupt→split→delete→rebuild restores status + answers; CAT=metamorphic; KILLS=lossy-cache-recovery mutants; VERDICT=MERGE→drill_corrupt_lexical_cache_serve_parity (R4 superset: adds outline)
- relation_corrupt_recover_verify_roundtrip_bench_history: INTENT=torn history fails loud, removal re-establishes IDENTICAL verdict + schema; CAT=metamorphic; KILLS=verdict-drift mutants; VERDICT=KEEP (anchor: bench roundtrip; absorbs R2 torn-history)
- relation_repeated_db_fault_cycles_deterministic: INTENT=3 corrupt→reindex cycles converge identically, quarantine chain pairs slot-i/garbage-i; CAT=metamorphic; KILLS=cycle-drift/slot-mixup mutants; VERDICT=KEEP (anchor: cycle determinism)
- relation_repeated_cache_fault_cycles_deterministic: INTENT=3 cache corrupt→delete→rebuild cycles converge identically; CAT=metamorphic; KILLS=same class as db cycles; VERDICT=MERGE→relation_repeated_db_fault_cycles_deterministic (facet: cache arm)
- relation_degraded_ivf_search_answer_parity: INTENT=degraded-IVF search serves same count + answer keys as healthy; CAT=metamorphic; KILLS=degraded-answer-loss mutants; VERDICT=MERGE→drill_corrupt_ivf_degraded_and_rebuilt_serve_parity (R4 serve-#1 is this + outline)
- relation_sigkill_interrupted_resume_equals_uninterrupted: INTENT=killed+resumed reindex equals uninterrupted twin (status + answers); CAT=metamorphic; KILLS=resume-divergence mutants; VERDICT=MERGE→drill_sigkill_mid_reindex_serve_parity (facet: twin comparator arm)
- relation_recovered_state_cli_parity_vs_clean: INTENT=corrupted-then-healed twin serves identical status/answers/doctor-health as clean twin; CAT=metamorphic; KILLS=recovery-residue mutants; VERDICT=KEEP (anchor: recovered-vs-clean parity; unique doctor-healthy arm)

## tests/cli/durable_recovery_pass4.rs (R4 drills, 8 tests)

- drill_sigkill_mid_reindex_serve_parity: INTENT=SIGKILL mid-reindex → reindex → search+outline serve identical to baseline; CAT=crash-drill; KILLS=kill-residue mutants; VERDICT=KEEP (anchor: kill drill; absorbs R3 twin arm)
- drill_corrupt_index_db_serve_parity: INTENT=corrupt db → refuse → reindex+quarantine → serve identical; CAT=crash-drill; KILLS=lossy-recovery mutants; VERDICT=KEEP (anchor: corrupt drill; absorbs R3 roundtrip + torn arm)
- drill_deleted_index_db_serve_parity: INTENT=deleted db → refuse → index cold-start (no quarantine) → serve identical; CAT=crash-drill; KILLS=cold-start mutants; VERDICT=KEEP (anchor: delete drill; absorbs R2 zero-length arm)
- drill_torn_truncated_db_serve_parity: INTENT=torn db → refuse → reindex+quarantine → serve identical; CAT=crash-drill; KILLS=same class as corrupt drill; VERDICT=MERGE→drill_corrupt_index_db_serve_parity (facet: torn crash arm; body otherwise identical)
- drill_corrupt_lexical_cache_serve_parity: INTENT=corrupt lexical → writer refuses → delete+index rebuilds → serve identical; CAT=crash-drill; KILLS=cache-recovery mutants; VERDICT=KEEP (anchor: lexical drill; absorbs R3 roundtrip)
- drill_deleted_caches_and_stamp_serve_parity: INTENT=combined wipe of lexical+IVF+stamp still converges via index, reindex restores all, serve identical; CAT=crash-drill; KILLS=derived-state-load-bearing mutants; VERDICT=KEEP (only combined-wipe test)
- drill_corrupt_ivf_degraded_and_rebuilt_serve_parity: INTENT=corrupt IVF serves THROUGH degradation identically, then delete+reindex rebuilds valid sidecar, serve identical again; CAT=crash-drill; KILLS=degraded-answer-loss mutants; VERDICT=KEEP (anchor: IVF drill; absorbs R3 parity)
- drill_chained_double_crash_serve_parity: INTENT=corrupt→heal→delete→heal serves ORIGINAL baseline, crash-1 evidence survives; CAT=crash-drill; KILLS=chain-residue mutants; VERDICT=KEEP (only chained drill)

## tests/codemode/durable_recovery_pass1.rs (R1 static contracts, 10 tests)

- missing_plan_and_batch_files_fail_closed_as_other: INTENT=missing plan/batch files fail as Other (io), never silent-empty; CAT=fail-closed; KILLS=silent-empty mutants; VERDICT=MERGE→corrupt_truncated_and_empty_files_fail_closed_as_json (facet: missing/Other arm of file-refusal matrix)
- corrupt_truncated_and_empty_files_fail_closed_as_json: INTENT=garbage/truncated/empty plan+batch files fail as Json (syntax layer); CAT=fail-closed; KILLS=wrong-layer mutants; VERDICT=KEEP (anchor: file-refusal matrix; absorbs missing arm)
- valid_plan_and_batch_files_resume_deterministically: INTENT=same plan/batch bytes load twice and execute identically with hand-computed values; CAT=fail-closed; KILLS=load-nondeterminism mutants; VERDICT=KEEP (anchor: file roundtrip)
- absent_index_reads_fail_closed_then_empty_index_serves_deterministic_empty_results: INTENT=missing index fails reads as Other, writer creates empty index, empty serves deterministic zero hits; CAT=fail-closed; KILLS=serve-without-index mutants; VERDICT=KEEP (anchor: missing-index)
- corrupt_and_truncated_index_files_fail_closed_without_quarantine: INTENT=garbage/truncated index fails read+write as Other with NO quarantine, total refusal; CAT=fail-closed; KILLS=silent-rebuild/quarantine mutants; VERDICT=KEEP (anchor: total refusal; absorbs R2 mid-flow truncate arm)
- future_schema_version_is_refused_loudly_and_nonmutating: INTENT=user_version 9999 refused as Other on both paths, stamp untouched, peek survives; CAT=fail-closed; KILLS=migrate-down/serve-future mutants; VERDICT=KEEP (anchor: future-schema)
- stale_schema_version_readers_refuse_writers_migrate_then_reads_resume: INTENT=stale readers refuse, writer migrates in place to 16, reads resume on migrated rows; CAT=fail-closed; KILLS=serve-stale mutants; VERDICT=KEEP (anchor: stale-schema migrate)
- serve_stream_recovers_after_corrupt_records: INTENT=garbage + truncated-JSON lines fail as id-less per-record Errors, stream resumes, ends Bye; CAT=fail-closed; KILLS=stream-abort mutants; VERDICT=MERGE→mr_serve_valid_answers_unaffected_by_corrupt_lines (R3 is a superset: equivalence + no-id + Bye)
- corrupt_writer_generation_stamp_fails_open_per_contract: INTENT=corrupt/missing stamp reads 0, session keeps serving; CAT=fail-closed; KILLS=stamp-gating mutants; VERDICT=KEEP (anchor: static stamp fail-open; R2 owns mid-flow tear)
- index_repo_rebuilds_after_corrupt_index_is_removed: INTENT=corrupt fails closed, operator removal + index_repo rebuilds, search deterministic; CAT=fail-closed; KILLS=rebuild-failure mutants; VERDICT=MERGE→mr_corrupt_remove_rebuild_restores_identical_outputs (R3 superset: adds read/status equality)

## tests/codemode/durable_recovery_pass2.rs (R2 fault injection, 10 tests)

- fault_torn_plan_file_midflow_refused_as_json: INTENT=plan torn mid-flow refused as Json, live in-memory plan unaffected; CAT=fault-injection; KILLS=durable/live-confusion mutants; VERDICT=MERGE→mr_torn_plan_file_repair_restores_identical_result (facet: mid-flow + live-isolation arm) [R1-vs-R2 duplicate of static Json refusal]
- fault_truncated_batch_file_midflow_refused_as_json: INTENT=batch truncated mid-flow refused as Json, live request still executes; CAT=fault-injection; KILLS=same class as torn-plan; VERDICT=MERGE→mr_truncated_batch_file_repair_restores_identical_response (facet: mid-flow + live-isolation arm) [R1-vs-R2 duplicate]
- fault_truncated_index_midflow_refused_loudly_without_quarantine: INTENT=index truncated mid-flow fails read/write/plan as Other, no quarantine; CAT=fault-injection; KILLS=same class as R1 total-refusal; VERDICT=MERGE→corrupt_and_truncated_index_files_fail_closed_without_quarantine (facet: mid-flow arm incl plan path) [R1-vs-R2 duplicate]
- fault_session_dir_deleted_midrun_fails_closed: INTENT=root deleted mid-run fails warm + fresh sessions as Other on every path; CAT=fault-injection; KILLS=serve-without-root mutants; VERDICT=KEEP (only root-deletion test)
- fault_readonly_session_dir_writes_fail_cleanly_as_other: INTENT=read-only dir fails writes as Other (no panic/partial), bytes unchanged; CAT=fault-injection; KILLS=panic/partial-write mutants; VERDICT=KEEP (only readonly test; root-skip guard)
- fault_interrupted_batch_failed_call_atomic_siblings_preserved: INTENT=failing middle call commits nothing, wave continues, siblings keep ids/order/values; CAT=fault-injection; KILLS=partial-commit/abort-wave mutants; VERDICT=KEEP (anchor: intra-call atomicity)
- fault_interrupted_batch_commits_prefix_resumes_from_failed_id: INTENT=[edit-ok,edit-bad] commits prefix, resume of failed id alone converges, final bytes ordered; CAT=fault-injection; KILLS=rollback-prefix mutants; VERDICT=MERGE→mr_interrupted_batch_resume_equals_uninterrupted_wave (R3 superset: adds uninterrupted-twin equality)
- fault_concurrent_readers_and_parallel_agree_exactly: INTENT=two live sessions serve byte-identical capsules; serial/parallel waves agree per-call; CAT=fault-injection; KILLS=session-drift/mode-divergence mutants; VERDICT=KEEP (anchor: concurrent-reader agreement; strip serial/parallel arm — covered by R3 MR-MODE)
- fault_writer_while_reader_no_stale_error: INTENT=warm reader sees writer's mutation via generation-bump invalidation, agrees with fresh session; CAT=fault-injection; KILLS=stale-cache mutants; VERDICT=KEEP (only cache-invalidation test)
- fault_corrupt_stamp_midflow_failopen_writer_restores: INTENT=stamp torn mid-flow reads 0 and serves, next writer restores nonzero stamp; CAT=fault-injection; KILLS=stamp-poison mutants; VERDICT=KEEP (anchor: mid-flow stamp tear + writer restore)

## tests/codemode/durable_recovery_pass3.rs (R3 relations, 11 tests)

- mr_plan_reexecution_on_warm_session_is_identical: INTENT=same plan twice on warm session identical except call-count doubling; CAT=metamorphic; KILLS=warm-state-rot mutants; VERDICT=MERGE→mr_plan_reexecution_across_fresh_sessions_is_identical (facet: warm-session arm)
- mr_plan_reexecution_across_fresh_sessions_is_identical: INTENT=same plan on two fresh sessions yields fully identical PlanResults; CAT=metamorphic; KILLS=session-nondeterminism mutants; VERDICT=KEEP (anchor: plan determinism; absorbs warm arm)
- mr_corrupt_remove_rebuild_restores_identical_outputs: INTENT=corrupt→refuse→remove→rebuild restores identical search/read/status; CAT=metamorphic; KILLS=lossy-repair mutants; VERDICT=KEEP (anchor: index roundtrip; absorbs R1 rebuild + R3 twin arm)
- mr_torn_plan_file_repair_restores_identical_result: INTENT=torn plan refused, rewrite-repair reruns to identical full PlanResult; CAT=metamorphic; KILLS=lossy-repair mutants; VERDICT=KEEP (anchor: plan roundtrip; absorbs R2 mid-flow arm)
- mr_truncated_batch_file_repair_restores_identical_response: INTENT=truncated batch refused, rewrite-repair reruns to identical response; CAT=metamorphic; KILLS=lossy-repair mutants; VERDICT=KEEP (anchor: batch roundtrip; absorbs R2 mid-flow arm)
- mr_interrupted_batch_resume_equals_uninterrupted_wave: INTENT=interrupted+resumed twin equals uninterrupted twin (bytes + per-call values); CAT=metamorphic; KILLS=resume-divergence mutants; VERDICT=KEEP (anchor: batch resume; absorbs R2 prefix-resume)
- mr_session_reopen_preserves_served_state_exactly: INTENT=drop+reopen preserves search/read/status, bytes, stamp; budget resets; CAT=metamorphic; KILLS=reopen-drift mutants; VERDICT=KEEP (only reopen test)
- mr_recovered_repo_matches_never_faulted_twin: INTENT=truncated→removed→rebuilt twin serves identically to pristine twin; CAT=metamorphic; KILLS=same class as roundtrip; VERDICT=MERGE→mr_corrupt_remove_rebuild_restores_identical_outputs (facet: truncate + pristine-twin arm)
- mr_stale_schema_migration_preserves_search_output_exactly: INTENT=stale refuse→migrate→resume preserves search+read outputs byte-identically; CAT=metamorphic; KILLS=lossy-migration mutants; VERDICT=KEEP (anchor: migration parity; complements R1 resume check)
- mr_serve_valid_answers_unaffected_by_corrupt_lines: INTENT=corrupt-interleaved stream answers valid calls identically to clean stream + 2 id-less Errors + Bye; CAT=metamorphic; KILLS=perturbation mutants; VERDICT=KEEP (anchor: stream equivalence; absorbs R1 resumption)
- mr_serial_parallel_agreement_survives_index_rebuild: INTENT=serial/parallel agree before fault, agree after rebuild, values preserved; CAT=metamorphic; KILLS=mode-divergence mutants; VERDICT=KEEP (anchor: mode equivalence; absorbs R2 parallel arm)

## tests/codemode/durable_recovery_pass4.rs (R4 drills, 8 tests)

- drill_corrupt_index_crash_recovers_to_identical_serve: INTENT=corrupt index → refuse → remove+rebuild → plan/batch/serve byte-identical; CAT=crash-drill; KILLS=crash-residue mutants; VERDICT=KEEP (anchor: index-crash drill; absorbs truncate + delete arms)
- drill_truncated_index_crash_recovers_to_identical_serve: INTENT=truncated index → refuse → remove+rebuild → identical serve; CAT=crash-drill; KILLS=same class as corrupt drill (weaker: search-only refusal); VERDICT=MERGE→drill_corrupt_index_crash_recovers_to_identical_serve (facet: truncate crash arm)
- drill_deleted_index_crash_recovers_to_identical_serve: INTENT=deleted index → refuse → rebuild → identical serve; CAT=crash-drill; KILLS=same class as corrupt drill; VERDICT=MERGE→drill_corrupt_index_crash_recovers_to_identical_serve (facet: delete crash arm)
- drill_torn_plan_file_crash_recovers_to_identical_serve: INTENT=torn plan → Json refuse → rewrite-repair → plan/batch/serve identical; CAT=crash-drill; KILLS=record-recovery mutants; VERDICT=MERGE→drill_deleted_plan_and_batch_files_crash_recovers_to_identical_serve (facet: torn-plan/Json arm)
- drill_corrupt_batch_file_crash_recovers_to_identical_serve: INTENT=garbage batch → Json refuse → rewrite-repair → identical; CAT=crash-drill; KILLS=same class as torn-plan drill; VERDICT=MERGE→drill_deleted_plan_and_batch_files_crash_recovers_to_identical_serve (facet: corrupt-batch/Json arm)
- drill_deleted_plan_and_batch_files_crash_recovers_to_identical_serve: INTENT=both records deleted → Other refuse → rewrite-repair → identical; CAT=crash-drill; KILLS=record-recovery mutants; VERDICT=KEEP (anchor: records drill; absorbs torn-plan + corrupt-batch arms)
- drill_stale_schema_crash_recovers_to_identical_serve: INTENT=stale stamp → refuse → writer migrates → identical serve; CAT=crash-drill; KILLS=migration-residue mutants; VERDICT=KEEP (only schema drill)
- drill_double_crash_truncated_index_then_torn_plan_recovers_to_identical_serve: INTENT=truncate→recover→tear-plan→recover serves ORIGINAL baseline, no residue; CAT=crash-drill; KILLS=chain-residue mutants; VERDICT=KEEP (only chained drill)

## tests/core/durable_recovery_pass1.rs (R1 static contracts, 12 tests)

- truncated_index_db_fails_closed_as_database_without_side_effects: INTENT=torn store fails as Database on both opens, bytes intact, no quarantine/sidecars; CAT=fail-closed; KILLS=silent-repair/quarantine mutants; VERDICT=KEEP (anchor: torn fail-closed)
- empty_store_readonly_refuses_writable_initializes: INTENT=zero-byte store refuses read-only (Other), peeks 0 without migrating, writable initializes in place; CAT=fail-closed; KILLS=serve-empty mutants; VERDICT=KEEP (anchor: zero-byte)
- schema_newer_than_binary_refuses_both_opens_with_structured_pair: INTENT=newer schema refuses both opens as Other with hand-computed pair via parse_schema_mismatch, stamp untouched; CAT=fail-closed; KILLS=migrate-down mutants; VERDICT=KEEP (anchor: future-schema)
- missing_index_peek_and_readonly_open_error_without_creating: INTENT=missing peek/read-only open errors as Other, creates no dirs/files; CAT=fail-closed; KILLS=auto-create mutants; VERDICT=KEEP (anchor: missing-index)
- garbage_lexical_sidecar_fails_closed_as_database: INTENT=garbage or truncated lexical.db fails search-open as Database, never served; CAT=fail-closed; KILLS=serve-garbage-sidecar mutants; VERDICT=KEEP (anchor: lexical fail-closed; 2-shape matrix)
- unservable_ivf_sidecars_read_as_none_without_error: INTENT=missing/empty/garbage/truncated/mismatched IVF all read Ok(None), peek forges nothing; CAT=fail-closed; KILLS=serve-invalid-sidecar mutants; VERDICT=KEEP (anchor: IVF None; 5-shape matrix)
- save_semantic_ivf_rejects_misaligned_vectors_as_other: INTENT=zero-dim/empty/ragged IVF saves rejected as Other, no files left; CAT=fail-closed; KILLS=partial-write mutants; VERDICT=KEEP (anchor: misaligned-save; 3-shape matrix)
- searcher_new_on_missing_root_fails_closed_as_other: INTENT=search on missing root fails as Other, no panic, no side effects; CAT=fail-closed; KILLS=panic/auto-create mutants; VERDICT=KEEP (unique missing-root check; tiny but load-bearing)
- writable_open_creates_missing_parents_deterministically: INTENT=writable open creates deep missing parents, lands current-schema store; CAT=fail-closed; KILLS=caller-must-mkdir mutants; VERDICT=KEEP (anchor: parent creation)
- read_only_dir_writable_open_errors_without_panic: INTENT=writable open under unwritable dir errors before any partial db appears; CAT=fail-closed; KILLS=panic/partial-db mutants; VERDICT=KEEP (missing-db arm; R2 owns existing-store-frozen — complementary, not duplicate)
- empty_index_serves_zero_hits_deterministically: INTENT=empty corpus serves 0 hits twice, 0 files/0 lines; CAT=fail-closed; KILLS=phantom-hit mutants; VERDICT=KEEP (anchor: empty-serve)
- writer_generation_absent_or_corrupt_reads_zero_fail_open: INTENT=missing/empty/corrupt stamp reads epoch 0; CAT=fail-closed; KILLS=stamp-erroring mutants; VERDICT=KEEP (anchor: stamp fail-open; 3-shape matrix)

## tests/core/durable_recovery_pass2.rs (R2 fault injection, 10 tests + 1 harness fn)

- fault_crash_sigkill_child_writer_mid_build_reopen_recovers: INTENT=SIGKILLed child in bulk tx → reopen recovers committed state, no quarantine, force-reindex serves; CAT=fault-injection; KILLS=phantom-row/quarantine mutants; VERDICT=KEEP (only SIGKILL core test)
- r2_child_writer_entry: INTENT=child-process entry for the SIGKILL test (no-op without spec env); CAT=other; KILLS=n/a (harness); VERDICT=KEEP (harness, not a test — excluded from test counts)
- fault_torn_splice_prefix_suffix_never_silently_healthy: INTENT=A-prefix/B-suffix splice refuses or detects deterministically, no quarantine/sidecars; CAT=fault-injection; KILLS=silent-healthy mutants; VERDICT=KEEP (anchor: splice)
- fault_truncation_offsets_refuse_or_detect_loudly: INTENT=0/1/header/mid/near-end tears refuse or detect loudly with exact discriminants where strict; CAT=fault-injection; KILLS=silent-healthy mutants; VERDICT=KEEP (anchor: truncation sweep; 5-offset matrix)
- fault_stale_lockfile_ignored_live_lock_blocks_then_proceeds: INTENT=stray *.lock files ignored unconsumed; live SQLite lock blocks then proceeds after rollback; CAT=fault-injection; KILLS=lockfile-honoring/wedge mutants; VERDICT=KEEP (anchor: locks; stale+live arms)
- fault_readonly_store_first_write_fails_database: INTENT=frozen read-only store fails every path as Database, bytes unaltered; CAT=fault-injection; KILLS=serve-frozen mutants; VERDICT=KEEP (anchor: frozen store; complements R1 missing-db arm)
- fault_concurrent_second_opener_bidirectional_visibility: INTENT=two writable handles see each other's commits both directions; CAT=fault-injection; KILLS=fork/wedge mutants; VERDICT=KEEP (anchor: concurrency)
- fault_interrupted_rename_old_or_new_never_mixed: INTENT=pre-rename crash serves old, post-rename serves new, never mixed; CAT=fault-injection; KILLS=torn-publish mutants; VERDICT=KEEP (anchor: atomic publish; absorbs orphan-tmp arm)
- fault_orphan_ivf_tmp_next_save_unaffected: INTENT=stale .tmp never poisons next save; CAT=fault-injection; KILLS=tmp-poison mutants; VERDICT=MERGE→fault_interrupted_rename_old_or_new_never_mixed (facet: orphan-tmp arm; same crash-safety family)
- fault_read_during_uncommitted_write_serves_last_committed: INTENT=read-only open during open bulk tx serves last committed, post-commit sees new rows; CAT=fault-injection; KILLS=torn-read mutants; VERDICT=KEEP (anchor: snapshot isolation)
- fault_index_path_is_directory_fails_closed: INTENT=directory-as-db refuses (writable Database, read-only Other), dir untouched; CAT=fault-injection; KILLS=write-into-dir mutants; VERDICT=KEEP (anchor: path confusion)

## tests/core/durable_recovery_pass3.rs (R3 relations, 11 tests)

- rebuild_incremental_second_pass_is_byte_identical_noop: INTENT=second incremental pass byte-identical, 0 files re-indexed, no quarantine; CAT=metamorphic; KILLS=gratuitous-rewrite mutants; VERDICT=KEEP (anchor: byte-idempotence)
- rebuild_force_twice_is_logically_identical: INTENT=two forced rebuilds converge to same snapshot + keys (bytes excluded by design); CAT=metamorphic; KILLS=non-idempotent-rebuild mutants; VERDICT=KEEP (anchor: logical idempotence)
- rebuild_ivf_save_twice_is_byte_identical: INTENT=same IVF payload saved twice (sibling + overwrite) byte-identical, loads same vectors; CAT=metamorphic; KILLS=nondeterministic-serialize mutants; VERDICT=KEEP (anchor: IVF determinism)
- repair_page_flip_then_verify_clean_and_restored: INTENT=page-flip → forced rebuild → clean verify + pre-fault snapshot/keys restored; CAT=metamorphic; KILLS=lossy-repair mutants; VERDICT=KEEP (anchor: repair-then-verify; absorbs truncation arm)
- repair_truncation_then_verify_clean_and_restored: INTENT=half-truncate → forced rebuild → same restored state as page-flip path; CAT=metamorphic; KILLS=same class as page-flip repair; VERDICT=MERGE→repair_page_flip_then_verify_clean_and_restored (facet: truncation arm; identical recovery path + asserts)
- recovery_same_fault_twice_is_deterministic: INTENT=same fault on two copies tears/quarantines/recovers identically; CAT=metamorphic; KILLS=nondeterministic-recovery mutants; VERDICT=KEEP (anchor: recovery determinism; absorbs order-independence arm)
- recovery_cancelled_build_resumed_equals_fresh: INTENT=cancelled build commits nothing, resume equals fresh build; CAT=metamorphic; KILLS=residue mutants; VERDICT=KEEP (anchor: partial-progress monotonicity; absorbs abandoned-tx arm)
- recovery_abandoned_bulk_tx_resumed_equals_fresh: INTENT=abandoned bulk tx rolls back, subsequent build equals fresh; CAT=metamorphic; KILLS=same class as cancelled-build; VERDICT=MERGE→recovery_cancelled_build_resumed_equals_fresh (facet: abandoned-tx arm)
- recovered_index_search_parity_with_never_crashed: INTENT=corrupt→recovered serves identical hits as never-crashed incl unknown token, pinned non-empty; CAT=metamorphic; KILLS=recovery-residue mutants; VERDICT=KEEP (anchor: search parity; unique unknown-token arm)
- recovery_disjoint_fault_order_commutes: INTENT=F1;F2 and F2;F1 tear identically and recover to identical quarantine/snapshot/keys; CAT=metamorphic; KILLS=order-dependent mutants; TAUTOLOGY-RISK on byte-layer commutativity assert (XOR at disjoint offsets commutes by construction — tests the fixture, not the product); VERDICT=MERGE→recovery_same_fault_twice_is_deterministic (facet: order-independence arm; drop byte-commutativity assert)
- recovery_second_fault_preserves_first_quarantine_and_reconverges: INTENT=second fault allocates fresh quarantine, first intact, reconverges; CAT=metamorphic; KILLS=overwrite-evidence mutants; VERDICT=KEEP (anchor: quarantine monotonicity)

## tests/core/durable_recovery_pass4.rs (R4 drills, 7 tests + 1 harness fn)

- drill_sigkill_child_writer_reopen_serves_baseline: INTENT=SIGKILL child in bulk tx → plain reopen (no quarantine) serves pre-crash baseline; CAT=crash-drill; KILLS=phantom-row mutants; VERDICT=KEEP (anchor: kill drill)
- r4_child_writer_entry: INTENT=child-process entry for the SIGKILL drill (no-op without spec env); CAT=other; KILLS=n/a (harness); VERDICT=KEEP (harness, not a test — excluded from test counts)
- drill_page_corrupt_force_rebuild_serves_baseline: INTENT=page-flip → forced rebuild + quarantine → serves baseline; CAT=crash-drill; KILLS=lossy-repair mutants; VERDICT=KEEP (anchor: corrupt drill; absorbs truncate arm)
- drill_truncate_force_rebuild_serves_baseline: INTENT=half-truncate → forced rebuild + quarantine → serves baseline; CAT=crash-drill; KILLS=same class as page-corrupt drill; VERDICT=MERGE→drill_page_corrupt_force_rebuild_serves_baseline (facet: truncate crash arm; body otherwise identical)
- drill_deleted_db_rebuild_serves_baseline: INTENT=deleted db+WAL → plain rebuild (no quarantine) → serves baseline; CAT=crash-drill; KILLS=cold-start mutants; VERDICT=KEEP (anchor: delete drill; absorbs zeroed arm)
- drill_deleted_lexical_sidecar_rebuild_serves_baseline: INTENT=deleted lexical.db → incremental build rebuilds it → sidecar search serves baseline; CAT=crash-drill; KILLS=missing-sidecar-blind mutants; VERDICT=KEEP (only sidecar drill)
- drill_zeroed_db_reopen_rebuild_serves_baseline: INTENT=zeroed db → writable reopen initializes + build repopulates → serves baseline; CAT=crash-drill; KILLS=same cold-start family as delete drill; VERDICT=MERGE→drill_deleted_db_rebuild_serves_baseline (facet: zeroed arm)
- drill_chained_truncate_then_corrupt_serves_baseline: INTENT=truncate→recover→corrupt→recover serves ORIGINAL baseline, both quarantines distinct; CAT=crash-drill; KILLS=chain-residue mutants; VERDICT=KEEP (anchor: chained drill)

## tests/mcp/durable_recovery_pass1.rs (R1 static contracts, 8 tests)

- startup_with_missing_asgrep_root_exits_without_json: INTENT=missing ASGREP_ROOT fails process nonzero with zero JSON-RPC on stdout; CAT=fail-closed; KILLS=half-initialized-server mutants; VERDICT=KEEP (anchor: startup)
- workspace_root_removed_mid_session_fails_tools_closed_server_survives: INTENT=root removed mid-session fails pipelined tools closed, ping answers, exit 0; CAT=fail-closed; KILLS=hang-on-missing-root mutants; VERDICT=KEEP (pipelined arm; R2 owns sequential+heal — complementary, not duplicate)
- per_call_root_pointing_at_file_fails_closed: INTENT=per-call root as regular file fails all tools closed; CAT=fail-closed; KILLS=root-type-confusion mutants; VERDICT=KEEP (anchor: file-root)
- corrupt_index_db_refused_loudly_across_tools_while_reads_survive: INTENT=garbage index.db refused by status/search/reindex±force, code_read unaffected; CAT=fail-closed; KILLS=silent-empty mutants; VERDICT=KEEP (anchor: corrupt-index)
- deleting_corrupt_db_then_reindex_heals_search_and_status: INTENT=delete corrupt db + index_repo heals, status+search serve; CAT=fail-closed; KILLS=heal-failure mutants; VERDICT=KEEP (anchor: heal path)
- empty_root_chain_reproduces_identically_across_restarts: INTENT=empty-root tools/list+status+miss reproduce byte-identically across restarts; CAT=fail-closed; KILLS=restart-drift mutants; VERDICT=KEEP (anchor: empty restart)
- session_restart_clears_snippet_elision_state: INTENT=restart restores full snippets byte-identical to first session, no elision markers; CAT=fail-closed; KILLS=durable-elision mutants; VERDICT=KEEP (anchor: elision reset)
- pinned_index_path_garbage_refused_loudly_default_root_unaffected: INTENT=garbage pinned index refused loudly, unpinning serves healthy default; CAT=fail-closed; KILLS=pin-poison mutants; VERDICT=KEEP (anchor: index pin)

## tests/mcp/durable_recovery_pass2.rs (R2 fault injection, 8 tests)

- fault_root_deleted_mid_session_next_call_fails_closed_and_live_heal: INTENT=next sequential call after root deletion fails closed, ping answers, recreating root heals LIVE session; CAT=fault-injection; KILLS=no-live-heal mutants; VERDICT=KEEP (sequential+heal arm; complements R1 pipelined)
- fault_index_truncated_to_stub_mid_session_refused_loudly_reads_survive: INTENT=7-byte stub tear mid-session refused loudly (fresh limit defeats warm cache), code_read survives; CAT=fault-injection; KILLS=warm-cache-masking mutants; VERDICT=KEEP (anchor: index-tear refusal; absorbs half-length arm)
- fault_index_torn_half_length_mid_session_refused_loudly_without_silent_empty: INTENT=half-length tear mid-session is tool errors, never silent-empty/fabricated; CAT=fault-injection; KILLS=same class as stub tear; VERDICT=MERGE→fault_index_truncated_to_stub_mid_session_refused_loudly_reads_survive (facet: half-length tear arm)
- fault_stdin_closed_mid_session_exits_cleanly: INTENT=EOF after healthy call exits 0 in budget, never hangs; CAT=fault-injection; KILLS=hang-on-eof mutants; VERDICT=KEEP (anchor: stdin EOF; absorbs partial-line arm)
- fault_stdin_closed_mid_request_partial_line_exits_cleanly_without_response: INTENT=EOF with torn request in flight exits 0, no torn-id response, stdout stays JSON; CAT=fault-injection; KILLS=torn-id-answered mutants; VERDICT=MERGE→fault_stdin_closed_mid_session_exits_cleanly (facet: partial-line arm)
- fault_garbage_line_mid_stream_ignored_session_continues: INTENT=unparsable mid-stream line ignored (no echo), next calls answer; CAT=fault-injection; KILLS=hang/echo mutants; VERDICT=KEEP (anchor: garbage line)
- fault_invalid_envelope_mid_stream_yields_error_shape_session_survives: INTENT=unknown-method envelope yields JSON-RPC error shape (echoed id, numeric code), session survives; CAT=fault-injection; KILLS=shape mutants; VERDICT=KEEP (anchor: invalid envelope; distinct from ignore-garbage)
- fault_restart_after_root_deletion_reproduces_clean_state: INTENT=indexed chain reproduces byte-identically after fault+restore+restart cycle; CAT=fault-injection; KILLS=fault-trace mutants; VERDICT=KEEP (anchor: fault-restore restart; complements R3 pure-restart)

## tests/mcp/durable_recovery_pass3.rs (R3 relations, 8 tests)

- restart_indexed_chain_reproduces_byte_identical_responses: INTENT=indexed chain (list/status/search×2/read) byte-identical across 2 restarts + shape checks; CAT=metamorphic; KILLS=restart-drift mutants; VERDICT=MERGE→repeated_identical_sessions_produce_identical_transcripts (3-session full-Value transcript is a superset; fold in tuple-width/node-id spot checks)
- fault_source_corruption_restart_roundtrip_restores_baseline: INTENT=source-byte fault tracked by reads, reproduces across restarts, restore+restart restores baseline; CAT=metamorphic; KILLS=heal-or-mask mutants; VERDICT=KEEP (only source-byte fault test)
- tools_list_stable_across_stream_index_and_root_fault_cycles: INTENT=tools/list bytes invariant under stream/index/root faults + restarts; CAT=metamorphic; KILLS=catalog-drift mutants; VERDICT=KEEP (anchor: catalog stability)
- search_then_read_consistency_preserved_across_restarts: INTENT=search→compact-id→read link resolves to same node+bytes in every fresh session; CAT=metamorphic; KILLS=link-rot mutants; VERDICT=KEEP (anchor: link consistency)
- repeated_identical_sessions_produce_identical_transcripts: INTENT=3 identical sessions produce fully identical transcripts; CAT=metamorphic; KILLS=session-memory mutants; VERDICT=KEEP (anchor: transcript identity; absorbs indexed-chain test)
- resend_seen_encoding_invariant_under_position_and_restart: INTENT=resend_seen encoding identical at every position, interleaved, across restarts, never elides; CAT=metamorphic; KILLS=position-drift mutants; VERDICT=KEEP (anchor: stateless encoding)
- error_envelopes_reproduce_identically_across_restarts: INTENT=4 invalid calls fail with byte-identical tool errors across restarts; CAT=metamorphic; KILLS=error-drift mutants; VERDICT=KEEP (anchor: error determinism)
- explicit_root_equivalent_to_default_across_restarts: INTENT=omitted vs explicit-default root byte-identical within and across sessions; CAT=metamorphic; KILLS=root-equivalence mutants; VERDICT=KEEP (anchor: root equivalence)

## tests/mcp/durable_recovery_pass4.rs (R4 drills, 8 tests)

- drill_kill_idle_server_recovers_identical_serve: INTENT=SIGKILL idle session → fresh session serves chain byte-identically; CAT=crash-drill; KILLS=crash-trace mutants; VERDICT=KEEP (anchor: kill drill; absorbs in-flight arm)
- drill_kill_with_pipelined_requests_in_flight_recovers: INTENT=SIGKILL with 3 pipelined sends unread → fresh session serves baseline (pending output asserted nothing about); CAT=crash-drill; KILLS=same class as idle-kill (only delta is 3 sends); VERDICT=MERGE→drill_kill_idle_server_recovers_identical_serve (facet: in-flight arm)
- drill_stdin_eof_mid_session_fresh_session_serves_identical: INTENT=clean EOF mid-session → fresh session reproduces chain; CAT=crash-drill; KILLS=eof-trace mutants; VERDICT=MERGE→drill_stdin_aborted_mid_request_fresh_session_serves (facet: clean-EOF arm; aborted drill is strictly richer)
- drill_stdin_aborted_mid_request_fresh_session_serves: INTENT=torn-request EOF exits 0 with no torn-id response + JSON-only stdout → fresh session serves baseline; CAT=crash-drill; KILLS=torn-id-answered mutants; VERDICT=KEEP (anchor: EOF drill; absorbs clean-EOF arm)
- drill_root_deleted_then_killed_restore_and_reindex_recovers: INTENT=root deleted + killed → out-of-band restore + index_repo → search/read identical, status across stamp; CAT=crash-drill; KILLS=restore-divergence mutants; VERDICT=KEEP (anchor: root drill)
- drill_index_corrupted_then_killed_delete_and_reindex_heals: INTENT=corrupt db + killed → delete + index_repo → identical serve; CAT=crash-drill; KILLS=heal-divergence mutants; VERDICT=KEEP (anchor: index drill; absorbs deleted-index arm)
- drill_index_deleted_then_killed_reindex_recovers: INTENT=deleted db + killed → index_repo → identical serve; CAT=crash-drill; KILLS=same class as corrupt drill (recovery body identical); VERDICT=MERGE→drill_index_corrupted_then_killed_delete_and_reindex_heals (facet: deleted-index arm)
- drill_double_crash_kill_then_tear_then_kill_recovers: INTENT=kill → interim serve proves recovery → tear+kill → delete+rebuild → ORIGINAL baseline; CAT=crash-drill; KILLS=chain-residue mutants; VERDICT=KEEP (anchor: chained drill)

## Helper patterns (testkit candidates)

- CLI harness (~230 lines × 4 files): asgrep/seed_project/seed_big_project/run_in/run_timeout/stdout_json/assert_operational_failure/assert_success/run_index/corrupt_db/KillOnDrop/kill9 → testkit::cli_recovery module.
- CLI comparators (pass3/4): status_snapshot/run_status/search_answer_keys/run_search/capture_baseline/assert_serve_parity/run_outline_snapshot/run_file_count → same module.
- Codemode loaders+fixtures (all 4 files): load_plan_file/load_batch_file/config_for/indexed_repo/batch_request/db_user_version/set_db_user_version/remove_db_with_sidecars/status_counts/search_capsule/read_window + drill_plan_json/drill_batch_request/baseline_flow/prove_full_function/serve_transcript → testkit::codemode_recovery module.
- Core fixtures (pass2/3/4 overlap): err_of/write_corpus/index_options/build_and_quiet/remove_wal_sidecars/db_bytes/snapshot/integrity/hits_key/parity_keys/assert_torn/flip_range/flip_mid/quarantine_path/home_names/upsert_file/fixture_store/probe_open → testkit::core_recovery module.
- MCP harness (~200 lines × 4 files): mcp_bin/init_payload/rpc_session/rpc_session_env/tool_call/tools_list/ping/tool_text/tool_body/assert_tool_error_shape/index_tree/indexed_tree/corrupt_index_db/truncate_index_db/LiveSession (+serve_chain/assert_serve_valid/status-scrub in p4) → testkit::mcp_session extension (same win as tokio catalog).
- R1-vs-R2 duplicate pattern (flagged 8×): same contract planted statically (garbage) vs injected as torn bytes — consolidate as two fault arms in one test rather than two tests.

## Proposed structure

- tests/cli/recovery_contracts.rs: reader fail-closed, doctor kinds, writer boundary (garbage+torn), stale-lock (debris+dead-pid), stamp cold-start, IVF degrade (garbage+torn), lexical split (garbage+torn), history rebuild, readonly (home+history-dir+missing-root), live-flock, lock-obstruction, killed-watch, quarantine uniqueness, bench torn-roundtrip, prior fail-open, install config (17 tests, absorbs R2 single-fault singles)
- tests/cli/recovery_relations.rs: noop byte-idempotence, reindex observable idempotence, quarantine stability, bench roundtrip, db+cache cycle determinism, recovered-vs-clean parity (6 tests)
- tests/cli/recovery_drills.rs: sigkill (+twin arm), corrupt db (+torn arm), deleted db (+zero-length arm), lexical, wiped-caches, IVF two-beat, chained (7 tests)
- tests/codemode/recovery_contracts.rs: file-refusal matrix (missing+corrupt), file roundtrip, missing index, total refusal (+mid-flow arm), future/stale schema, static+midflow stamp, root deletion, readonly, batch atomicity, concurrent readers, writer-reader invalidation (12 tests)
- tests/codemode/recovery_relations.rs: plan determinism (warm+fresh), index/plan/batch roundtrips (+twin arm), batch resume, reopen, migration parity, stream equivalence, mode equivalence (8 tests)
- tests/codemode/recovery_drills.rs: index crash (corrupt+truncate+delete), records crash (torn+corrupt+deleted), schema, chained (4 tests)
- tests/core/recovery_contracts.rs: all 12 R1 + sigkill, splice, truncation sweep, locks, frozen store, concurrency, atomic publish (+orphan-tmp), snapshot isolation, path confusion (21 tests + 1 harness)
- tests/core/recovery_relations.rs: 3 idempotence, repair-then-verify (flip+truncate), determinism (+order arm, byte-assert dropped), cancel resume (+abandoned-tx), search parity, quarantine monotonicity (8 tests)
- tests/core/recovery_drills.rs: sigkill, corrupt (+truncate), deleted (+zeroed), sidecar, chained (5 tests + 1 harness)
- tests/mcp/recovery_contracts.rs: all 8 R1 + root live-heal, index tear (stub+half), stdin EOF (clean+partial), garbage line, invalid envelope, fault-restore restart (14 tests)
- tests/mcp/recovery_relations.rs: transcript identity (+chain checks), source roundtrip, catalog stability, link consistency, resend_seen, error determinism, root equivalence (7 tests)
- tests/mcp/recovery_drills.rs: kill (+in-flight), EOF (+clean), root, index (+deleted), chained (5 tests)
- Total: ~112 intent tests (+2 harness fns) replacing 153; harnesses collapse into testkit.

## Counts

- Total 155 #[test] fns = 153 real tests + 2 harness fns (r2/r4_child_writer_entry, KEEP as harness).
- Real tests: KEEP 112 / MERGE 41 / DELETE 0. No pure tautologies found (one TAUTOLOGY-RISK assert flagged inside an otherwise-KEEP test); fluff is fragmentation (single-fault singles + R1-vs-R2/R3-vs-R4 subset pairs), not emptiness.
- By area: cli 42 (26/16/0), codemode 39 (26/13/0), core 40 real (34/6/0), mcp 32 (26/6/0).

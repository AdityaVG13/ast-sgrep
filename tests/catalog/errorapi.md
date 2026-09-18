# errorapi catalog — every error-API test by intent

Scope: all 20 `tests/*/error_api_pass*.rs` files, 213 `#[test]`s. One line per test.
CAT ∈ {taxonomy, propagation, negative-metamorphic, e2e-drill, other}.
KILLS ∈ {mutant class | BEHAVIOR-ONLY | TAUTOLOGY-RISK + reason}.
OVERLAP flags pins already covered by oracle/recovery/protocol suites (kept only when the discriminant/relation is new).

## tests/cli/error_api_pass1.rs (E1 taxonomy inventory, 14)

- clap_rejection_is_exit_1_with_usage_envelope: INTENT=clap parse rejection exits 1, human stderr + machine usage envelope; CAT=taxonomy; KILLS=exit-code-swap(1↔2), envelope-kind-swap; VERDICT=KEEP (usage-family merge anchor)
- missing_query_is_usage_exit_1: INTENT=bare invocation without query exits 1; CAT=taxonomy; KILLS=exit-code-swap, missing-arg-default-Ok; VERDICT=MERGE→usage-exit-1 taxonomy
- unknown_lang_is_usage_exit_1: INTENT=unknown --lang fails closed exit 1; CAT=taxonomy; KILLS=lang-fallback-to-default-swallow, exit-swap; VERDICT=MERGE→usage-exit-1 taxonomy
- bad_format_is_usage_exit_1: INTENT=unknown --format exits 1; CAT=taxonomy; KILLS=format-fallback-swallow, exit-swap; VERDICT=MERGE→usage-exit-1 taxonomy
- query_and_pattern_conflict_is_usage_exit_1: INTENT=QUERY positional + --pattern conflict exits 1; CAT=taxonomy; KILLS=conflict-takes-first-silently, exit-swap; VERDICT=MERGE→usage-exit-1 taxonomy
- index_dry_run_and_path_conflict_is_usage_exit_1: INTENT=index --dry-run + --path conflict exits 1; CAT=taxonomy; KILLS=conflict-swallow, exit-swap; VERDICT=MERGE→usage-exit-1 taxonomy
- codemod_without_yes_is_usage_exit_1: INTENT=codemod without --yes exits 1 before root IO; CAT=taxonomy; KILLS=guard-drop(implicit-yes), exit-swap; VERDICT=MERGE→usage-exit-1 taxonomy
- ambiguous_root_is_usage_exit_1: INTENT=--root + positional ROOT ambiguity exits 1; CAT=taxonomy; KILLS=first-wins-silently, exit-swap; VERDICT=MERGE→usage-exit-1 taxonomy
- missing_root_search_is_operational_exit_2: INTENT=missing root search exits 2 operational; CAT=taxonomy; KILLS=exit-swap(2→1), envelope-kind-swap; VERDICT=KEEP (operational-family merge anchor)
- unindexed_root_search_is_operational_exit_2: INTENT=unindexed root fails closed exit 2, no auto-index; CAT=taxonomy; KILLS=auto-index-swallow-to-Ok, exit-swap; VERDICT=MERGE→operational-exit-2 taxonomy
- eval_gold_failures_are_operational_exit_2: INTENT=unreadable or query-less eval gold exits 2; CAT=taxonomy; KILLS=gold-default-empty-swallow, exit-swap; VERDICT=MERGE→operational-exit-2 taxonomy
- outline_unindexed_file_is_operational_exit_2: INTENT=outline of unindexed file exits 2; CAT=taxonomy; KILLS=empty-outline-Ok-swallow, exit-swap; VERDICT=MERGE→operational-exit-2 taxonomy
- codemode_batch_malformed_is_operational_exit_2: INTENT=malformed codemode-batch JSON exits 2; CAT=taxonomy; KILLS=json-error-to-usage-swap, exit-swap; VERDICT=MERGE→operational-exit-2 taxonomy
- zero_hit_search_is_success_exit_0: INTENT=zero-hit search is ok:true with empty hit list (success control); CAT=other; KILLS=empty-to-error-swap; VERDICT=MERGE→success-shape control row (not error taxonomy)

## tests/cli/error_api_pass2.rs (E2 propagation, 12)

- query_too_long_search_propagates_operational: INTENT=oversize query rejected by lib reaches CLI as exit 2 both modes; CAT=propagation; KILLS=swallow-to-Ok, exit-swap; VERDICT=KEEP
- invalid_regex_search_propagates_operational: INTENT=invalid regex rejected by lib reaches CLI as exit 2 both modes; CAT=propagation; KILLS=regex-fallback-to-literal, exit-swap; VERDICT=KEEP
- file_filter_too_long_propagates_operational: INTENT=oversize file_filter rejected at Searcher::new reaches CLI as exit 2; CAT=propagation; KILLS=filter-length-gate-drop, exit-swap; VERDICT=KEEP (file-filter merge anchor)
- file_filter_control_chars_propagate_operational: INTENT=control-char file_filter rejected at finish reaches CLI as exit 2; CAT=propagation; KILLS=filter-content-gate-drop, exit-swap; VERDICT=MERGE→file-filter rejection propagation
- in_scope_missing_propagates_operational: INTENT=in: scope matching nothing rejected by lib reaches CLI as exit 2; CAT=propagation; KILLS=scope-miss-to-empty-Ok, exit-swap; VERDICT=KEEP (in-scope merge anchor)
- in_scope_escape_propagates_operational: INTENT=escaping in: scope rejected by lib reaches CLI as exit 2; CAT=propagation; KILLS=scope-jail-drop, exit-swap; VERDICT=MERGE→in-scope rejection propagation
- feature_gates_propagate_between_lib_and_cli: INTENT=neural-embed/rerank lib-vs-CLI agree (both err or both ok per build); CAT=propagation; KILLS=feature-gate-drop, gate-only-on-one-side; VERDICT=KEEP (only cross-build agreement pin; success arm is BEHAVIOR-ONLY exit-0 check)
- corrupt_index_path_propagates_operational: INTENT=non-DB --index-path rejected by lib reaches CLI as exit 2; CAT=propagation; KILLS=Database-kind-drop, exit-swap; VERDICT=KEEP; OVERLAP=recovery corrupt pins (adds lib+CLI agreement)
- codemod_empty_pattern_propagates_operational: INTENT=empty-pattern plan_codemod rejection reaches preview+apply as exit 2; CAT=propagation; KILLS=preview-apply-divergence, exit-swap; VERDICT=KEEP
- call_path_oversize_propagates_operational: INTENT=oversize call-path endpoint rejected by lib reaches CLI as exit 2; CAT=propagation; KILLS=endpoint-length-gate-drop, exit-swap; VERDICT=KEEP
- bench_unknown_suite_propagates_operational: INTENT=unknown bench suite (lib None) reaches CLI as exit 2; CAT=propagation; KILLS=suite-fallback-to-default, exit-swap; VERDICT=KEEP
- durability_unknown_propagates_usage: INTENT=unknown durability (lib None) reaches CLI as exit 1; CAT=propagation; KILLS=durability-fallback-to-default, exit-swap; VERDICT=KEEP (only usage-propagation row)

## tests/cli/error_api_pass3.rs (E3 relations, 11)

- missing_root_human_and_json_agree_on_operational: INTENT=same root fault fails in both modes with same exit; CAT=negative-metamorphic; KILLS=mode-divergence(success-in-one-mode); VERDICT=KEEP (agreement-relation merge anchor)
- invalid_regex_human_and_json_agree_on_operational: INTENT=same query fault fails in both modes with same exit; CAT=negative-metamorphic; KILLS=mode-divergence; VERDICT=MERGE→human/machine agreement relation
- unknown_lang_human_and_json_agree_on_usage: INTENT=same usage fault fails in both modes with same exit; CAT=negative-metamorphic; KILLS=mode-divergence; VERDICT=MERGE→human/machine agreement relation
- missing_root_same_operational_family_across_subcommands: INTENT=one root fault, exit 2 + operational kind on 4 subcommands; CAT=negative-metamorphic; KILLS=subcommand-exit-divergence, kind-divergence; VERDICT=KEEP
- unindexed_root_same_operational_family_across_subcommands: INTENT=one unindexed fault, exit 2 + operational kind on 4 readers; CAT=negative-metamorphic; KILLS=subcommand-exit-divergence; VERDICT=MERGE→cross-subcommand operational family (parameterize fault × subcommand)
- ambiguous_root_same_usage_family_across_subcommands: INTENT=one ambiguity fault, exit 1 + usage kind on 3 subcommands; CAT=negative-metamorphic; KILLS=subcommand-exit-divergence; VERDICT=KEEP (usage family; distinct from operational)
- operational_envelope_shape_deterministic_across_reruns: INTENT=same fault rerun 3× yields identical exit + envelope shape; CAT=negative-metamorphic; KILLS=nondeterministic-envelope, flaky-exit; VERDICT=KEEP
- human_failure_deterministic_across_reruns: INTENT=human failure rerun yields identical exit; CAT=negative-metamorphic; KILLS=flaky-exit; VERDICT=MERGE→failure-determinism relation (fold human leg into envelope determinism)
- failed_codemod_leaves_tree_untouched: INTENT=rejected codemod rewrites nothing, adds/drops nothing; CAT=negative-metamorphic; KILLS=partial-write-on-failure; VERDICT=KEEP (write-path fail-closed)
- failed_root_fault_creates_no_state: INTENT=root fault conjures no root, no partial index, no parent entries; CAT=negative-metamorphic; KILLS=state-creation-on-failure; VERDICT=KEEP (create-path fail-closed; distinct mechanism)
- malformed_batch_deterministic_and_fail_closed: INTENT=malformed batch deterministic, no partial results, no success shape; CAT=negative-metamorphic; KILLS=half-ok-batch, nondeterministic-envelope; VERDICT=KEEP

## tests/cli/error_api_pass4.rs (E4 drills, 8)

- corrupt_index_db_search_drill_reindex_heals: INTENT=corrupt db fails 2 both modes, reindex heals, hits return; CAT=e2e-drill; KILLS=heal-path-regression, stale-hits-after-corrupt; VERDICT=KEEP; OVERLAP=recovery (adds both-modes + hit-list proof)
- deleted_index_state_search_drill_index_heals: INTENT=deleted .asgrep fails 2 both modes, fresh index heals; CAT=e2e-drill; KILLS=heal-path-regression; VERDICT=KEEP (distinct heal path from reindex)
- bad_lang_arg_search_drill_fix_arg_recovers: INTENT=unknown --lang fails 1 both modes, valid args recover; CAT=e2e-drill; KILLS=arg-fault-sticks-after-fix; VERDICT=MERGE→arg-fault recovery drill
- query_pattern_conflict_drill_single_query_recovers: INTENT=QUERY+--pattern fails 1 both modes, single query recovers; CAT=e2e-drill; KILLS=arg-fault-sticks-after-fix; VERDICT=KEEP (arg-drill anchor)
- invalid_regex_query_drill_fix_query_recovers: INTENT=invalid regex fails 2 both modes, valid query recovers; CAT=e2e-drill; KILLS=query-fault-poisons-handle; VERDICT=KEEP (operational query-fix drill)
- unreadable_gold_eval_drill_valid_gold_recovers: INTENT=directory-as-gold fails 2 both modes, valid gold recovers with scored query; CAT=e2e-drill; KILLS=gold-fault-sticks-after-fix; VERDICT=KEEP (only eval drill)
- outline_missing_file_drill_indexed_file_recovers: INTENT=outline missing file fails 2 both modes, indexed file recovers with count; CAT=e2e-drill; KILLS=outline-fault-sticks-after-fix; VERDICT=KEEP (only outline drill)
- chained_corrupt_plus_bad_arg_ordered_recovery: INTENT=usage fault wins over corrupt state, then operational, then success; CAT=e2e-drill; KILLS=validation-order-swap(arg-after-state); VERDICT=KEEP (only precedence + ordered-recovery pin)

## tests/codemode/error_api_pass1.rs (E1 taxonomy, 12)

- e1_unknown_tool_row_direct_plan_batch_serve: INTENT=UnknownTool row on direct/plan/batch/serve surfaces; CAT=taxonomy; KILLS=variant-swap(UnknownTool→InvalidArgs), serve-Err-instead-of-Result; VERDICT=KEEP
- e1_invalid_args_pure_tool_guard_gaps: INTENT=7 pure-tool guard arms yield InvalidArgs; CAT=taxonomy; KILLS=guard-drop per tool; VERDICT=KEEP
- e1_invalid_args_plan_ref_and_shape_gaps: INTENT=plan $ref index-required arms + null-args guard yield InvalidArgs; CAT=taxonomy; KILLS=ref-arm-swap, null-args-panic; VERDICT=KEEP; OVERLAP=oracle ref matrix (pins gaps only)
- e1_invalid_args_serve_envelope_not_err: INTENT=serve maps bad identity/batch shape to Error envelopes, keeps serving; CAT=taxonomy; KILLS=serve-abort-on-validation, Error-vs-Result-swap; VERDICT=KEEP
- e1_budget_exhausted_serve_returns_discriminant: INTENT=10_001st serve call yields Result{ok:false} + BudgetExhausted(10_000); CAT=taxonomy; KILLS=budget-count-off-by-one, fail-open-past-budget; VERDICT=MERGE→serve budget fail-once (e2_serve_budget_answers_once_ignores_trailing runs the same 10k-call fault and pins the same discriminant + trailing silence; fold discriminant pin there, delete this run)
- e1_json_row_from_conversion: INTENT=From<serde_json::Error> (direct + via ?) lands on CallError::Json; CAT=taxonomy; KILLS=BEHAVIOR-ONLY (pins a constructor; production to_value sites admitted infallible so no production path exercised); VERDICT=MERGE→json-cause contract (fold the via-? arm into e2_json_cause_preserved_via_downcast as the single Json pin)
- e1_other_search_find_chain_query_validation: INTENT=missing/overlong query on search/find/chain yields Other pre-IO; CAT=taxonomy; KILLS=variant-swap(Other→InvalidArgs), validation-after-IO; VERDICT=KEEP
- e1_other_unknown_lang_rejected_pre_io: INTENT=unknown lang on search/find yields Other before index work; CAT=taxonomy; KILLS=lang-fallback-swallow; VERDICT=KEEP; OVERLAP=text pinned elsewhere (discriminant new)
- e1_other_read_ref_shape_and_jail: INTENT=read ref-shape + jail arms yield Other; CAT=taxonomy; KILLS=guard-drop, jail-drop; VERDICT=KEEP
- e1_other_edit_shape_and_uniqueness: INTENT=edit shape/uniqueness/jail arms yield Other, file untouched; CAT=taxonomy; KILLS=guard-drop, write-before-validate; VERDICT=KEEP
- e1_other_index_repo_targeted_shape: INTENT=index_repo targeted-shape arms yield Other; CAT=taxonomy; KILLS=guard-drop, traversal-accept; VERDICT=KEEP
- e1_batch_per_call_mirrors_direct_discriminants: INTENT=batch envelope stays Ok, per-call ok/value/error mirror direct outcomes; CAT=taxonomy; KILLS=envelope-Err-instead-of-per-call, ok/error-coexist; VERDICT=MERGE→batch-mirrors-direct (e3_batch_mirrors_direct_outcomes asserts the same mirror over the same 4-row shape + all_ok conjunction; fold exclusivity checks there)

## tests/codemode/error_api_pass2.rs (E2 propagation, 14)

- e2_session_other_preserves_io_cause_chain: INTENT=Other keeps std source + io::Error reachable in anyhow chain; CAT=propagation; KILLS=cause-flatten-to-string, source-drop; VERDICT=KEEP
- e2_plan_invalid_args_short_circuits: INTENT=mid-plan InvalidArgs keeps variant, later steps never run; CAT=propagation; KILLS=variant-rewrap, no-short-circuit; VERDICT=KEEP (short-circuit merge anchor)
- e2_plan_other_short_circuits: INTENT=mid-plan Other keeps variant, step three never runs, Err not Ok; CAT=propagation; KILLS=variant-rewrap, no-short-circuit; VERDICT=MERGE→plan short-circuit propagation (same contract, second variant; parameterize)
- e2_plan_parse_and_shape_never_touch_budget: INTENT=malformed/empty plans fail InvalidArgs with zero budget consumed; CAT=propagation; KILLS=parse-failure-charges-budget, variant-swap; VERDICT=KEEP
- e2_batch_envelope_validation_is_err: INTENT=envelope-shape violations are Err InvalidArgs, never Ok+all_ok:false; CAT=propagation; KILLS=envelope-violation-as-per-call-row, Ok-carried-failure; VERDICT=KEEP
- e2_batch_parallel_readonly_isolates_failures: INTENT=rayon path keeps serial contract: Ok envelope, isolated failures, input order; CAT=propagation; KILLS=parallel-crosstalk, reorder, mode-misreport; VERDICT=MERGE→batch-mode equivalence (e3_batch_mode_equivalence runs the same 4-row mixed batch under both modes; fold the mode=="parallel" pin there)
- e2_budget_sticks_on_session: INTENT=spent budget sticky: same payload, frozen count, exhausted; CAT=propagation; KILLS=budget-degrade-to-Other, counter-drift; VERDICT=KEEP
- e2_plan_budget_aborts_with_discriminant: INTENT=budget exhaustion inside plan aborts BudgetExhausted, unstarted step uncharged; CAT=propagation; KILLS=budget-wrap-as-Other, overcharge; VERDICT=KEEP
- e2_serve_tool_failures_are_result_and_continues: INTENT=serve maps tool failures to Result{ok:false}, keeps serving to Bye; CAT=propagation; KILLS=Result-vs-Error-swap, worker-abort; VERDICT=KEEP
- e2_serve_batch_mixed_propagates_percall: INTENT=serve BatchResult mirrors per-call ok/fail, worker continues to Bye; CAT=propagation; KILLS=batch-abort-on-first-failure, percall-flatten; VERDICT=KEEP; OVERLAP=e4 serve-batch drill (E2 is the cheap field-level pin)
- e2_serve_budget_answers_once_ignores_trailing: INTENT=serve fail-once: one Result{ok:false}, BudgetExhausted return, trailing input silent; CAT=propagation; KILLS=error-flood-past-budget, trailing-Bye; VERDICT=KEEP (fail-once merge anchor)
- e2_json_cause_preserved_via_downcast: INTENT=Json keeps inner serde error: downcast, syntax/eof discriminants, transparent Display; CAT=propagation; KILLS=BEHAVIOR-ONLY (synthetic: constructs From over synthetic serde errors; no production call path yields Json); VERDICT=KEEP (sole Json pin after folding e1_json_row; documents the variant contract)
- e2_unknown_tool_vs_invalid_args_precedence: INTENT=unknown name beats garbage args; known tool + bad args (incl. unknown catalog entry) is InvalidArgs; CAT=propagation; KILLS=dispatch-order-swap, catalog-entry-as-UnknownTool; VERDICT=KEEP
- e2_ok_channel_never_carries_failure: INTENT=ok plan/batch attest success; failing plan is Err; CAT=other; KILLS=BEHAVIOR-ONLY (success arms belong to happy-path suites; failure arm duplicates e2_plan_invalid_args_short_circuits); VERDICT=DELETE (no error mutant uniquely killed)

## tests/codemode/error_api_pass3.rs (E3 relations, 10)

- e3_dispatch_equivalence_unknown_tool: INTENT=session.call == call_tool on UnknownTool (+ budget-bump divergence); CAT=negative-metamorphic; KILLS=dispatch-surface-divergence; VERDICT=MERGE→dispatch equivalence (fold 3 variant matrices into one parameterized test)
- e3_dispatch_equivalence_invalid_args: INTENT=session.call == call_tool on 8 InvalidArgs faults; CAT=negative-metamorphic; KILLS=dispatch-surface-divergence; VERDICT=KEEP (equivalence merge anchor)
- e3_dispatch_equivalence_other: INTENT=session.call == call_tool on 5 Other faults; CAT=negative-metamorphic; KILLS=dispatch-surface-divergence; VERDICT=MERGE→dispatch equivalence
- e3_repeat_bad_call_deterministic: INTENT=repeat bad call same discriminant same+fresh session; budget bypass pinned; CAT=negative-metamorphic; KILLS=nondeterministic-discriminant, bypass-regression; VERDICT=KEEP
- e3_batch_position_independence: INTENT=bad call at index 0/1/2: same envelope, good values bit-identical; CAT=negative-metamorphic; KILLS=position-dependent-routing, good-row-taint; VERDICT=KEEP
- e3_batch_mode_equivalence: INTENT=serial vs parallel mixed batch: same per-id pattern, values, order; CAT=negative-metamorphic; KILLS=mode-divergence; VERDICT=KEEP (mode-equivalence merge anchor)
- e3_plan_position_independence: INTENT=failing step at any index: same discriminant as direct, count pins prefix; CAT=negative-metamorphic; KILLS=position-dependent-discriminant, count-drift; VERDICT=KEEP
- e3_batch_mirrors_direct_outcomes: INTENT=per-call ok == direct is_ok, exclusivity per row, all_ok conjunction; CAT=negative-metamorphic; KILLS=mirror-divergence, partial-value-smuggle; VERDICT=KEEP (batch-mirror merge anchor)
- e3_plan_fail_closed_never_ok: INTENT=failed plan is Err with direct discriminant; prefix applied vs failing-edit untouched; CAT=negative-metamorphic; KILLS=Ok-carried-plan-failure, prefix-rollback, partial-edit-write; VERDICT=KEEP (fail-closed merge anchor)
- e3_serve_stream_position_independence: INTENT=bad request at stream index 0/1/2: Result{ok:false}, neighbors identical, Bye reached; CAT=negative-metamorphic; KILLS=stream-position-dependence, worker-abort; VERDICT=KEEP

## tests/codemode/error_api_pass4.rs (E4 drills, 8)

- e4_serve_batch_fault_mid_stream_isolates_and_resumes: INTENT=mid-stream batch fault isolated per-call, stream reaches Bye; CAT=e2e-drill; KILLS=isolation-breach, stream-abort; VERDICT=KEEP
- e4_unknown_tool_in_plan_resumes_with_corrected_plan: INTENT=UnknownTool mid-plan fails, same session resumes corrected plan; CAT=e2e-drill; KILLS=session-poison-after-UnknownTool; VERDICT=KEEP
- e4_budget_exhaustion_mid_plan_sticks_terminal: INTENT=budget aborts mid-plan then sticks terminal (non-resumable); CAT=e2e-drill; KILLS=budget-recovery-after-exhaustion; VERDICT=KEEP; OVERLAP=e2 budget pins (adds mid-plan flow)
- e4_deleted_root_fault_scoped_and_recovers_after_repair: INTENT=root deletion fails bound calls as Other, pure tools green, reindex resumes; CAT=e2e-drill; KILLS=fault-scope-breach, stale-searcher-after-repair; VERDICT=KEEP
- e4_atomic_edit_batch_zero_writes_then_resumes: INTENT=failing edits[] entry fails whole call, zero writes, retry applies; CAT=e2e-drill; KILLS=partial-batch-write; VERDICT=KEEP
- e4_plan_prefix_applied_then_remainder_resumes: INTENT=mutate-then-bad-args plan fails InvalidArgs with prefix kept, remainder resumes; CAT=e2e-drill; KILLS=prefix-rollback-on-resume; VERDICT=MERGE→plan fail-closed (first half duplicates e3_plan_fail_closed_never_ok line-for-line incl. hello-world/mars fixture; append the remainder-resume half to E3)
- e4_chained_double_fault_unknown_then_budget: INTENT=UnknownTool then, after resume spends budget, terminal BudgetExhausted in order; CAT=e2e-drill; KILLS=fault-order-swap, non-sticky-terminal; VERDICT=KEEP
- e4_oversized_response_fault_then_resumes: INTENT=oversized response fails Other closed (never truncated-ok), session resumes; CAT=e2e-drill; KILLS=truncated-ok, session-poison; VERDICT=KEEP; OVERLAP=oracle oversized pin (adds resume + budget-charge)

## tests/core/error_api_pass1.rs (E1 taxonomy, 14)

- store_error_database_from_garbage_index_file: INTENT=garbage bytes at index path open as Database; CAT=taxonomy; KILLS=variant-swap(Database→Other), garbage-tolerated; VERDICT=KEEP (sole Database row); OVERLAP=durable_recovery_pass1 torn db (this pins the minimal trigger)
- store_error_io_from_missing_file_read: INTENT=missing file read surfaces Io; CAT=taxonomy; KILLS=variant-swap(Io→Other); VERDICT=KEEP (sole Io row; NEW)
- store_error_other_from_empty_pattern_ingress: INTENT=empty pattern ingress refuses as Other; CAT=taxonomy; KILLS=empty-pattern-accepted; VERDICT=MERGE→search-ingress Other rows; OVERLAP=pattern_routing message-level (discriminant new)
- schema_mismatch_helper_roundtrip_is_format_contract: INTENT=schema_newer message round-trips through parse_schema_mismatch; CAT=other; KILLS=BEHAVIOR-ONLY (self-roundtrip; kills format drift); VERDICT=KEEP (only format-contract pin); OVERLAP=oracle_foundry_pass1/2
- io_bounds_over_cap_rejects_as_other: INTENT=over-cap file read refuses as Other; CAT=taxonomy; KILLS=cap-drop; VERDICT=MERGE→io-bounds Other rows; OVERLAP=oracle_foundry_pass3 is_err-only (discriminant new)
- io_bounds_binary_and_directory_reject_as_other: INTENT=binary + directory reads refuse as Other; CAT=taxonomy; KILLS=binary-sniff-drop, dir-read-accepted; VERDICT=MERGE→io-bounds Other rows; OVERLAP=oracle_foundry_pass3 is_err-only
- search_rejects_oversize_query: INTENT=search ingress wires validate_query_len, oversize refuses Other; CAT=taxonomy; KILLS=ingress-wiring-drop; VERDICT=KEEP (Other-ingress merge anchor); OVERLAP=oracle unit bounds (search wiring NEW)
- search_regex_rejects_invalid_pattern: INTENT=uncompilable regex: refuses Other; CAT=taxonomy; KILLS=regex-compile-error-swallow; VERDICT=MERGE→search-ingress Other rows
- regex_pass_rejects_oversize_pattern: INTENT=regex-length gate refuses when hit directly; CAT=taxonomy; KILLS=gate-drop; VERDICT=MERGE→search-ingress Other rows
- searcher_new_rejects_oversize_file_filter: INTENT=Searcher::new rejects oversize file_filter pre-open; CAT=taxonomy; KILLS=filter-length-gate-drop; VERDICT=MERGE→search-ingress Other rows
- search_rejects_empty_file_filter: INTENT=empty file_filter refuses at finish; CAT=taxonomy; KILLS=empty-filter-accepted; VERDICT=MERGE→search-ingress Other rows
- update_paths_rejects_over_max_batch: INTENT=incremental batch over MAX_INCREMENTAL_PATHS refuses Other; CAT=taxonomy; KILLS=max-batch-gate-drop; VERDICT=MERGE→batch/shape Other rows
- search_multi_pattern_rejects_empty_batch: INTENT=zero-pattern multi-pattern ingress refuses Other; CAT=taxonomy; KILLS=empty-batch-accepted; VERDICT=MERGE→batch/shape Other rows
- call_path_rejects_oversize_endpoints: INTENT=find_call_path validates both endpoint lengths; CAT=taxonomy; KILLS=endpoint-gate-drop(one-side); VERDICT=MERGE→batch/shape Other rows

## tests/core/error_api_pass2.rs (E2 propagation, 13)

- indexer_new_missing_root_surfaces_io: INTENT=RootDir::open ENOENT surfaces Io(NotFound), db not created; CAT=propagation; KILLS=Io→Other-swap, side-effect-before-check; VERDICT=KEEP (Io-surfacing merge anchor)
- indexer_new_file_as_root_surfaces_io: INTENT=file-as-root (O_DIRECTORY) surfaces Io; CAT=propagation; KILLS=variant-swap; VERDICT=MERGE→missing-root Io surfacing (same RootDir::open path, second trigger)
- searcher_new_missing_root_converts_to_other: INTENT=Searcher::new deliberately converts canonicalize Io to Other; CAT=propagation; KILLS=conversion-drop(raw-Io-leak); VERDICT=KEEP (deliberate conversion contract)
- store_open_uncreatable_index_dir_converts_to_other: INTENT=create_dir_all Io deliberately maps to Other; CAT=propagation; KILLS=conversion-drop; VERDICT=KEEP (deliberate map-site contract)
- db_garbage_kind_preserved_through_store_open: INTENT=garbage open stays Database + NotADatabase kind + corrupt-detectable; CAT=propagation; KILLS=kind-drop, variant-swap; VERDICT=KEEP (kind-preservation merge anchor)
- db_garbage_through_searcher_new_stays_database: INTENT=same depth failure through read layer stays Database + kind; CAT=propagation; KILLS=read-layer-conversion; VERDICT=MERGE→garbage-kind preservation across layers (fold store/searcher/indexer legs into one multi-layer test); OVERLAP=E3 garbage_db_same_discriminant asserts the same cross-layer relation
- db_garbage_through_indexer_new_without_force_stays_database: INTENT=Indexer::new w/o force propagates Database untouched, file byte-identical; CAT=propagation; KILLS=silent-recovery, repair-before-reject; VERDICT=MERGE→garbage-kind preservation across layers (carry the byte-identical assertion)
- db_dropped_table_through_search_pattern_stays_database: INTENT=sqlite failure below search_pattern propagates Database, control Ok first; CAT=propagation; KILLS=lane-degrade-to-Ok-empty, degrade-to-Other; VERDICT=KEEP
- update_paths_over_max_rejects_before_side_effect: INTENT=over-max gate fires before ignore.clear/indexing, store never sees batch; CAT=propagation; KILLS=gate-after-side-effect; VERDICT=KEEP; OVERLAP=E3 failed_writes (E2 pins never-saw-batch via file_hash)
- cancelled_index_all_rejects_before_side_effect: INTENT=cancel-before-walk: Other + indexes nothing; CAT=propagation; KILLS=cancel-check-drop, partial-index-on-cancel; VERDICT=KEEP (cancel merge anchor)
- cancelled_update_paths_rejects_before_side_effect: INTENT=per-path cancel: Other without indexing the file; CAT=propagation; KILLS=cancel-check-drop; VERDICT=MERGE→cancel-before-side-effect (same check_cancel contract, second entry)
- per_file_failure_is_counted_not_silent: INTENT=binary .rs fails its path, sibling indexes, batch Ok with files_failed=1; CAT=propagation; KILLS=silent-skip, batch-Err-on-single-failure; VERDICT=KEEP (only loud-degradation pin)
- missing_db_through_searcher_new_fails_closed: INTENT=readonly missing-db guard fails Other, control Ok after index; CAT=propagation; KILLS=fail-open-to-empty-searcher; VERDICT=KEEP

## tests/core/error_api_pass3.rs (E3 relations, 11)

- garbage_db_same_discriminant_across_open_search_and_index: INTENT=same corrupt bytes: same discriminant + sqlite kind at all 3 layers; CAT=negative-metamorphic; KILLS=layer-conversion, kind-divergence; VERDICT=KEEP (MR1 anchor); OVERLAP=E2 garbage legs (this is the relation statement)
- blocked_index_dir_same_discriminant_across_store_and_indexer: INTENT=file-blocked index dir fails Other through both layers; CAT=negative-metamorphic; KILLS=layer-divergence; VERDICT=KEEP (distinct map site from garbage)
- oversize_query_same_discriminant_across_search_lanes: INTENT=oversize query Other on all 5 lexical lanes incl. multi-pattern fan-in; CAT=negative-metamorphic; KILLS=lane-truncate-and-answer, lane-conversion; VERDICT=KEEP
- dropped_table_same_discriminant_across_depth_and_search_lanes: INTENT=dropped pattern_nodes: Database + same kind via depth/free/search/multi lanes; CAT=negative-metamorphic; KILLS=lane-degrade, kind-divergence; VERDICT=KEEP (dropped-table merge anchor)
- dropped_table_fails_status_with_database_discriminant: INTENT=dropped symbols table fails status probe as Database + kind; CAT=negative-metamorphic; KILLS=status-Ok-over-broken-schema; VERDICT=MERGE→dropped-table Database relation (one dropped-table test covering depth/free/search/multi/status); OVERLAP=E4 dropped-tables drill pins status leg too
- repeated_triggers_yield_identical_discriminant_sequence: INTENT=4 triggers × 2 runs: identical discriminant + kind sequences; CAT=negative-metamorphic; KILLS=run-to-run-drift; VERDICT=KEEP
- error_does_not_poison_later_success_on_same_handle: INTENT=Err then valid call Ok on same handle; repeated Err identical; CAT=negative-metamorphic; KILLS=handle-poison-on-error; VERDICT=KEEP; OVERLAP=E4 oversize drill (E3 is ingress-level)
- poisoned_store_fails_closed_across_pattern_lanes: INTENT=dropped table: every pattern lane Err, controls Ok first, cache off; CAT=negative-metamorphic; KILLS=Ok-empty-over-poison, cache-masks-poison; VERDICT=MERGE→dropped-table Database relation (carry control-first + response-cache-off rationale)
- cancelled_writes_fail_closed_with_same_discriminant: INTENT=set cancel flag fails index_all + update_paths as Other, no Ok stats; CAT=negative-metamorphic; KILLS=entry-divergence-on-cancel; VERDICT=MERGE→cancel-before-side-effect (collapses the 3 cancel tests — E2 ×2 + this — into one entry×assert test)
- failed_opens_leave_filesystem_untouched: INTENT=failed opens leave fs byte-identical (snapshot before/after); CAT=negative-metamorphic; KILLS=half-created-index, garbage-repair-on-reject; VERDICT=KEEP
- failed_writes_preserve_committed_rows: INTENT=failed writes preserve committed hash + line count, same Other; CAT=negative-metamorphic; KILLS=committed-row-clobber-on-failure; VERDICT=KEEP

## tests/core/error_api_pass4.rs (E4 drills, 8)

- drill_deleted_index_search_fails_closed_then_reindex_recovers: INTENT=deleted db fails Other, reindex restores search+status+exact hash; CAT=e2e-drill; KILLS=heal-hash-divergence, fail-open-on-missing-db; VERDICT=KEEP
- drill_corrupt_index_open_fails_database_then_force_reindex_recovers: INTENT=garbage db fails Database, force_reindex quarantines + rebuilds clean; CAT=e2e-drill; KILLS=quarantine-regression, heal-hash-divergence; VERDICT=KEEP (corrupt-drill merge anchor); OVERLAP=recovery
- drill_truncated_index_search_fails_database_then_rebuild_recovers: INTENT=32-byte stump fails Database, delete + rebuild restores clean; CAT=e2e-drill; KILLS=truncation-tolerated; VERDICT=MERGE→corrupt-index drill (stump is a second corruption trigger with identical discriminant + recovery; fold as trigger leg)
- drill_unreadable_index_search_fails_database_then_perm_restore_recovers: INTENT=unreadable db (unix 000) fails Database, perm restore revives byte-identical; CAT=e2e-drill; KILLS=perm-error-misclassified, rebuild-on-transient; VERDICT=KEEP (distinct fault + no-rebuild recovery)
- drill_oversize_query_search_rejects_then_clean_query_succeeds: INTENT=oversize rejects Other untouched, same handle serves search+status+hash; CAT=e2e-drill; KILLS=rejection-side-effect, handle-poison; VERDICT=KEEP; OVERLAP=E3 no-poison (adds status+hash+repeat)
- drill_dropped_tables_search_and_status_fail_database_then_rebuild_recovers: INTENT=dropped tables fail search AND status Database, rebuild restores; CAT=e2e-drill; KILLS=Ok-empty-over-poison, status-Ok-over-poison; VERDICT=KEEP; OVERLAP=E3 dropped-table legs (end-to-end + rebuild)
- drill_missing_root_flows_fail_then_restore_recovers: INTENT=renamed-away root: write flow Io(NotFound), read flow Other, restore revives; CAT=e2e-drill; KILLS=flow-confusion(Io↔Other), restore-divergence; VERDICT=KEEP
- drill_chained_double_fault_query_precedence_then_schema_fault_then_recovery: INTENT=oversize query Other wins over poisoned store, valid query Database, rebuild + still-rejects; CAT=e2e-drill; KILLS=ingress-after-store-touch, precedence-swap; VERDICT=KEEP (only precedence pin)

## tests/lang/error_api_pass1.rs (E1 taxonomy E1-LANG-01..12, 12)

- lang_id_rejections: INTENT=unknown/blank lang ids rejected (None) across parse/from_extension/canonical + controls; CAT=taxonomy; KILLS=silent-default-language, blank-accepted; VERDICT=KEEP
- detect_language_unsupported_paths: INTENT=unknown ext / no content / no shebang → None + controls; CAT=taxonomy; KILLS=detect-guess-on-unknown; VERDICT=KEEP
- registry_parses_every_language_ok: INTENT=registry parse Ok on empty + hostile input for every language (Err side unreachable); CAT=taxonomy; KILLS=introduced-Err-on-user-input, per-language-panic; VERDICT=KEEP
- extraction_empty_and_garbage_fail_closed: INTENT=empty/garbage source → Ok + empty rows, flag clear + control; CAT=taxonomy; KILLS=invented-rows-on-garbage, Err-on-garbage; VERDICT=KEEP
- depth_guard_threshold: INTENT=shallow clear / 300-deep paren nesting sets depth_truncated (256 cap); CAT=taxonomy; KILLS=cap-drop(fail-open-deep), cap-too-tight; VERDICT=KEEP
- classify_native_rejections: INTENT=exotic shapes classify None + call-shape control; CAT=taxonomy; KILLS=misclassified-exotic; VERDICT=KEEP
- cached_and_candidate_signature_gaps: INTENT=braced/chain/malformed unindexable None; empty→Some([]); + controls; CAT=taxonomy; KILLS=index-serve-unindexable, empty-rejected; VERDICT=KEEP
- required_pattern_literal_absent: INTENT=meta-only/keyword/comment/empty patterns yield no prefilter literal + controls; CAT=taxonomy; KILLS=wrong-literal-filter(drops-files), keyword-as-literal; VERDICT=KEEP
- serve_and_answer_gates: INTENT=serve/answer/keyword/ident/universal-root gate denials + controls; CAT=taxonomy; KILLS=gate-serve-unserveable, keyword-ident-serve; VERDICT=KEEP
- fallback_loud_class: INTENT=fallback-loud shapes need ast-grep; native/empty shapes do not; CAT=taxonomy; KILLS=fallback-misroute; VERDICT=KEEP
- match_ok_empty_rejections: INTENT=rejected shapes match Ok(empty) incl. garbage source, never Err + control; CAT=taxonomy; KILLS=Err-on-rejected-shape, garbage-Err; VERDICT=KEEP
- declaration_prefix_non_decl_none: INTENT=declaration_prefix Some("fn") on function_item, None on name node; CAT=taxonomy; KILLS=prefix-on-non-decl, prefix-drop; VERDICT=KEEP (only node-level pin)

## tests/lang/error_api_pass2.rs (E2 propagation, 10)

- detect_some_always_parses_ok: INTENT=every indexed ext detects + parses hostile Ok; undetectable stays None; CAT=propagation; KILLS=detect-parse-handoff-break, silent-default; VERDICT=KEEP; OVERLAP=E1 registry rows (adds the handoff link)
- classifier_rejection_propagates_to_index_none: INTENT=classifier None → both signature stages None; classified keeps signatures; CAT=propagation; KILLS=serve-rejected-shape-from-index; VERDICT=KEEP
- unindexable_propagates_to_gate_denial_then_ok_match: INTENT=unindexable → gate deny → match Ok via walk; indexable control served; CAT=propagation; KILLS=deny-to-Err, deny-to-silent-empty; VERDICT=KEEP
- prefilter_none_still_scans: INTENT=no-literal patterns still scan via walk; all-comment stays Ok-empty; CAT=propagation; KILLS=None-means-skip, comment-fail-open; VERDICT=KEEP
- match_never_errs_or_panics_on_hostile_input: INTENT=hostile pattern × hostile source × every lang always Ok both entries; CAT=propagation; KILLS=Err-on-user-input, single-node-expect-panic; VERDICT=KEEP (answerable.rs guard pin)
- garbage_propagates_ok_empty_through_extract_and_match: INTENT=garbage → Ok + empty rows, flag clear, match Ok, every lang; CAT=propagation; KILLS=invented-hits-on-garbage, garbage-Err; VERDICT=KEEP; OVERLAP=E1-LANG-04 (adds all-lang + match agreement)
- depth_breach_propagates_loud_flag_not_err: INTENT=breach keeps Ok + sets depth_truncated; match independent Ok; CAT=propagation; KILLS=breach-as-Err, silent-breach; VERDICT=KEEP
- gate_denial_still_answers_via_walk: INTENT=statement-keyword shapes unserveable yet walk-answered (break/return); CAT=propagation; KILLS=deny-to-silent-empty; VERDICT=KEEP
- fallback_loud_shapes_match_closed: INTENT=fallback-loud patterns natively Ok-empty; degenerates unanswerable + control; CAT=propagation; KILLS=fabricated-hits-on-loud-shape; VERDICT=KEEP
- chain_span_guards_hold: INTENT=optional/member chains across connectors always Ok, genuine hit binds; CAT=propagation; KILLS=calls.rs-end-unwrap-panic; VERDICT=KEEP (span-guard pin)

## tests/lang/error_api_pass3.rs (E3 relations, 10)

- match_entries_agree_and_repeat_deterministically: INTENT=both match entries agree is_ok + identical hit vectors on repeat (disjoint corpus); CAT=negative-metamorphic; KILLS=entry-divergence(Ok-vs-Err), nondeterministic-hits; VERDICT=KEEP; OVERLAP=E2 hostile set (corpus disjoint by design)
- language_id_entries_agree: INTENT=parse/ext/filter/normalize/detect agree per ext, case/padding invariant, unknowns lowercase; CAT=negative-metamorphic; KILLS=entry-divergence, case-sensitivity-regression; VERDICT=KEEP
- classifier_acceptance_implies_downstream_presence: INTENT=cached/candidate Some ⟹ classified (dollar-less ident fast-path excepted); classified ⟹ no fallback; CAT=negative-metamorphic; KILLS=implication-break, fast-path-regression; VERDICT=KEEP
- gate_denial_is_total_and_order_invariant: INTENT=empty/kind sigs deny every pattern; keywords deny under any sigs; padding/order invariant; CAT=negative-metamorphic; KILLS=deny-hole, order-dependence; VERDICT=KEEP
- prefilter_literal_is_sound_substring: INTENT=Some literal is non-empty $-free whitespace-free substring; padding/repeat invariant; CAT=negative-metamorphic; KILLS=unsound-literal(drops-matching-files); VERDICT=KEEP
- semicolon_count_boundary_consistent_across_languages: INTENT=1×`;` answerable everywhere; 2+ family one verdict per lang (py/swift/kt only); match agrees; CAT=negative-metamorphic; KILLS=threshold-off-by-one, per-lang-divergence; VERDICT=KEEP
- depth_cap_monotone_with_loud_flip: INTENT=depth_truncated monotone in nesting; bisected flip has clear-below/loud-above; match Ok both sides; CAT=negative-metamorphic; KILLS=nonmonotone-flag, flip-regression; VERDICT=KEEP
- degenerate_inputs_total_and_stable: INTENT=every entry repeated on 21 degenerates agrees with itself (no panic, no drift); CAT=negative-metamorphic; KILLS=BEHAVIOR-ONLY (f(x)==f(x) self-comparison kills only nondeterminism/global drift); VERDICT=MERGE→match agreement + padding sweep (split: match/extraction repetition into E3-01, pure-entry repetition into E3-09; keep the no-panic sweep as its legs)
- padding_invariant_entries_vs_sensitive_ident: INTENT=trimming entries padding-invariant (+BOM-strip for match) vs is_pattern_ident padding-SENSITIVE; CAT=negative-metamorphic; KILLS=trim-regression, ident-trim-added; VERDICT=KEEP
- structural_term_signatures_shape_and_injectivity: INTENT=6 signatures per term, term last, each carries term; deterministic + injective; CAT=other; KILLS=BEHAVIOR-ONLY (no failure, rejection, or boundary under test — not error-API); VERDICT=DELETE (belongs to structural/boost suites; kills no error mutant)

## tests/lang/error_api_pass4.rs (E4 drills, 8)

- garbage_source_end_to_end_rank_empty: INTENT=garbage × every lang: detected, Ok, empty rows, flag clear, empty rank; CAT=e2e-drill; KILLS=garbage-invented-output, garbage-Err; VERDICT=MERGE→empty/garbage-source drill (E4-02 header states the identical documented outcome; one parameterized drill over source classes)
- empty_source_end_to_end_rank_empty: INTENT=empty/whitespace × every lang: same empty outcome as garbage; CAT=e2e-drill; KILLS=empty-invented-output; VERDICT=KEEP (empty/garbage merge anchor)
- unicode_bom_nul_source_end_to_end_sound_and_stable: INTENT=unicode/BOM/NUL: total pipeline, spans in-source, identical repeat rank; CAT=e2e-drill; KILLS=span-out-of-source, nondeterministic-rank; VERDICT=KEEP
- unsupported_language_short_circuits_with_no_ranked_output: INTENT=unsupported inputs short-circuit None: no parse/match/rank + control; CAT=e2e-drill; KILLS=silent-default-language, fabricated-hits; VERDICT=KEEP
- hostile_pattern_on_real_source_stays_sound: INTENT=hostile patterns on real files: match Ok, rank sound + deterministic; CAT=e2e-drill; KILLS=dropped-hits, unsound-rank; VERDICT=KEEP
- fallback_loud_pattern_end_to_end_match_closed: INTENT=fallback-loud patterns rank empty end-to-end + native control answers; CAT=e2e-drill; KILLS=fabricated-hits-on-loud-shape; VERDICT=KEEP
- depth_breach_end_to_end_loud_flag_with_sound_rank: INTENT=deep pipeline: loud flag + sound deterministic rank; shallow control clear; CAT=e2e-drill; KILLS=silent-breach, unsound-rank-on-deep; VERDICT=KEEP
- mixed_hostile_corpus_drill: INTENT=corpus mixing all hostile classes: per-class outcomes, union equality, corpus-rank determinism; CAT=e2e-drill; KILLS=invented/dropped-corpus-hits, nondeterministic-corpus-rank; VERDICT=KEEP

## tests/mcp/error_api_pass1.rs (E1 taxonomy J1..T6,S1, 11)

- taxonomy_unknown_method_is_32601_with_id_echo: INTENT=unknown methods -32601, id echo, no result; CAT=taxonomy; KILLS=code-swap, id-drop; VERDICT=KEEP; OVERLAP=protocol.rs -32601 (repin in table context)
- taxonomy_unshaped_envelope_is_32601_not_32602: INTENT=unshaped tools/call + initialize params map -32601 incl. non-object arguments; CAT=taxonomy; KILLS=code-swap(-32602), dispatch-on-unshaped; VERDICT=KEEP (rmcp mapping surprise documented)
- taxonomy_invalid_request_is_32600_without_id: INTENT=invalid requests -32600 with NO id member, no result; CAT=taxonomy; KILLS=code-swap, id-echo-on-32600; VERDICT=KEEP (new row)
- taxonomy_pre_initialize_request_is_32602: INTENT=pre-init requests -32602 with id echo, fatal non-zero exit; CAT=taxonomy; KILLS=code-swap, non-fatal-pre-init; VERDICT=KEEP (only wire-reachable -32602)
- taxonomy_unknown_tool_is_tool_error_not_jsonrpc_error: INTENT=unknown/case/whitespace tool names are tool errors, never JSON-RPC errors; CAT=taxonomy; KILLS=level-escalation(tool→rpc), fuzzy-name-match; VERDICT=KEEP; OVERLAP=protocol.rs unknown-tool (adds variants)
- taxonomy_search_arg_rejections_share_tool_error_shape: INTENT=11 search bound/type/unknown-key rejections share tool-error shape; CAT=taxonomy; KILLS=arg-accepted, level-escalation; VERDICT=KEEP
- taxonomy_code_read_arg_rejections_share_tool_error_shape: INTENT=6 code_read bound/type rejections share shape + 20-id ceiling still succeeds; CAT=taxonomy; KILLS=arg-accepted, ceiling-off-by-one; VERDICT=KEEP; OVERLAP=pass 3 pins 21 rejected (this pins the ceiling)
- taxonomy_code_read_node_id_rows_share_tool_error_shape: INTENT=6 node-id shape rejections share tool-error shape; CAT=taxonomy; KILLS=shape-accepted, level-escalation; VERDICT=KEEP
- taxonomy_index_tool_arg_rejections_share_tool_error_shape: INTENT=7 index-tool unknown-key/type/file-root rejections share shape, no indexing runs; CAT=taxonomy; KILLS=arg-accepted, index-before-validate; VERDICT=KEEP
- taxonomy_sandbox_escape_is_tool_error_across_all_tools: INTENT=per-call root outside workspace is tool error on all 4 root-taking tools; CAT=taxonomy; KILLS=jail-drop, per-tool-divergence; VERDICT=KEEP; OVERLAP=protocol.rs index_status-only (extends to all tools)
- taxonomy_notification_produces_no_response_session_continues: INTENT=absent/null-id notifications silent; next line is the ping; clean exit; CAT=taxonomy; KILLS=notification-response-leak, session Stall; VERDICT=KEEP

## tests/mcp/error_api_pass2.rs (E2 propagation, 10)

- backend_empty_index_is_success_miss_on_every_channel: INTENT=no index: success on all 5 channels, empty_index miss ×4 + native hits ×1; CAT=propagation; KILLS=miss-as-tool-error, channel-divergence; VERDICT=KEEP
- backend_corrupt_index_is_tool_error_on_every_channel: INTENT=corrupt db: tool error ×5 channels, code_read still success; CAT=propagation; KILLS=silent-empty-success, fabricated-hits; VERDICT=KEEP; OVERLAP=recovery corrupt pins (adds cross-channel uniformity + read contrast)
- backend_neural_gate_refusal_is_tool_error_not_success: INTENT=same semantic_search args: success default, tool error under ASGREP_NEURAL_EMBED=1; CAT=propagation; KILLS=gate-drop, refusal-as-success; VERDICT=KEEP
- backend_file_deleted_mid_session_flips_read_to_tool_error: INTENT=same node id flips success→toolerr→success as file deleted/restored; CAT=propagation; KILLS=parse-vs-read-confusion, stale-success; VERDICT=KEEP
- backend_symlink_escape_read_is_tool_error_session_survives: INTENT=symlink escape fails containment as tool error (unix), session serves after; CAT=propagation; KILLS=jail-drop, session-poison; VERDICT=KEEP
- backend_multi_id_read_fails_atomically_no_partial_nodes: INTENT=multi-id read with one bad id fails whole call either position, lone good succeeds; CAT=propagation; KILLS=partial-nodes-leak; VERDICT=KEEP
- backend_compact_id_resolves_only_after_search_registration: INTENT=compact id toolerr before registry, success after search registers; CAT=propagation; KILLS=registry-bypass, unregistered-success; VERDICT=KEEP
- propagation_sequence_reports_per_call_errors_without_dropping_results: INTENT=7-call mixed transcript: 3 tool-error classes uniform, -32601 rpc row, successes intact, ids 1..=7; CAT=propagation; KILLS=result-drop, reorder, level-confusion; VERDICT=KEEP
- propagation_pipelined_batch_with_backend_failure_keeps_every_id: INTENT=pipelined 6-call batch: every id exactly once, exactly one of result/error, good intact; CAT=propagation; KILLS=id-drop, id-dup, result+error-coexist; VERDICT=KEEP
- backend_empty_vs_corrupt_index_map_to_miss_vs_tool_error: INTENT=same query: empty→miss vs corrupt→toolerr contrast; CAT=propagation; KILLS=state-confusion(miss↔error); VERDICT=MERGE→empty-index miss + corrupt-index toolerr (P1+P2 already pin both halves with identical triggers; keep the contrast as closing lines in P2, delete this session pair)

## tests/mcp/error_api_pass3.rs (E3 relations, 10)

- consistency_mistyped_root_is_same_tool_error_on_every_root_tool: INTENT=same mistyped-root fault: identical discriminant on all 8 root tools; CAT=negative-metamorphic; KILLS=cross-tool-divergence; VERDICT=KEEP
- consistency_unknown_argument_is_same_tool_error_on_every_tool: INTENT=same unknown-arg fault: identical discriminant on all 8 tools; CAT=negative-metamorphic; KILLS=cross-tool-divergence; VERDICT=KEEP
- consistency_limit_fault_spellings_share_one_tool_error_code: INTENT=limit 0/-1/mistyped/huge share one discriminant; CAT=negative-metamorphic; KILLS=spelling-divergence; VERDICT=KEEP
- determinism_identical_tool_errors_are_byte_identical_across_sessions: INTENT=same bad tool calls × 2 sessions: byte-identical raw lines; CAT=negative-metamorphic; KILLS=nondeterministic-error-bytes; VERDICT=KEEP (determinism merge anchor)
- determinism_identical_jsonrpc_errors_are_byte_identical_across_sessions: INTENT=same rpc-error triggers × 2 sessions: byte-identical raw lines; CAT=negative-metamorphic; KILLS=nondeterministic-error-bytes; VERDICT=MERGE→byte-identical determinism (same relation at the second level; one test, two call sets)
- determinism_repeated_bad_call_in_one_session_is_identical_modulo_id: INTENT=same bad args twice in one session identical modulo id; CAT=negative-metamorphic; KILLS=per-call-randomness, counter-state-leak; VERDICT=KEEP (within-session; distinct from cross-session)
- position_permuted_transcript_yields_same_per_call_responses: INTENT=bad call first vs last: per-id byte-identical responses; CAT=negative-metamorphic; KILLS=position-dependence; VERDICT=KEEP
- position_notification_before_bad_call_leaves_error_bytes_unchanged: INTENT=notification ahead of bad call leaves error bytes unchanged; CAT=negative-metamorphic; KILLS=notification-perturbs-error; VERDICT=KEEP
- failclosed_error_responses_carry_no_success_members_or_payload_keys: INTENT=9-call fault catalog fails closed: no result/structuredContent/payload keys; rpc keys uniform; CAT=negative-metamorphic; KILLS=payload-smuggle-in-error, key-divergence; VERDICT=KEEP
- failclosed_backend_failures_carry_no_partial_payloads: INTENT=corrupt ×5 channels + deleted target: tool errors carry no partial payloads, positive controls first; CAT=negative-metamorphic; KILLS=partial-payload-smuggle; VERDICT=KEEP; OVERLAP=P2 corrupt-channels trigger (adds byte-absence + controls)

## tests/mcp/error_api_pass4.rs (E4 drills, 7)

- drill_index_db_deleted_mid_session_miss_then_live_reindex_heals: INTENT=live db delete → miss success + zero status, reads serve, live index_repo heals; CAT=e2e-drill; KILLS=delete-as-tool-error, stale-hits, heal-regression; VERDICT=KEEP (deletion-drill merge anchor); OVERLAP=recovery deletion (kill+restart only; this is live mapping)
- drill_index_db_overwritten_mid_session_refused_then_live_heal: INTENT=live garbage overwrite → search/status/reindex toolerr, reads serve, delete+rebuild heals; CAT=e2e-drill; KILLS=silent-empty-on-garbage, heal-regression; VERDICT=KEEP (refusal vs miss is the D1/D2 contrast)
- drill_asgrep_dir_deleted_mid_session_miss_then_live_rebuild: INTENT=live .asgrep/ delete → miss on search channel, reads serve, live rebuild recreates dir; CAT=e2e-drill; KILLS=dir-delete-as-tool-error; VERDICT=MERGE→index-deletion drill (same miss-mapping + live-rebuild shape as D1; differs only in fault granularity file-vs-dir and channel — fold as second fault leg in D1)
- drill_malformed_envelopes_mid_session_codes_then_session_usable: INTENT=live malformed envelopes report -32601/-32600/-32601, read+ping serve after; CAT=e2e-drill; KILLS=code-swap-live, session-poison; VERDICT=KEEP; OVERLAP=recovery mid-stream shape-only (this pins code values + -32600 live)
- drill_bad_argument_storm_mid_session_uniform_then_usable: INTENT=9-call parse-level storm uniform toolerr with ids, search+read serve after; CAT=e2e-drill; KILLS=storm-divergence, session-poison; VERDICT=KEEP; OVERLAP=P8 static mix (this is parse-storm + live usability)
- drill_sandbox_escape_mid_session_refused_session_unpoisoned: INTENT=live escape roots ×3 tools toolerr, read+status+ping serve after; CAT=e2e-drill; KILLS=jail-drop-live, session-poison; VERDICT=KEEP; OVERLAP=T6 static rows (this is live + usability)
- drill_chained_double_fault_staged_heal_restores_live_session: INTENT=garbage db + deleted source live: staged heal revives reads while search still fails, then full heal; CAT=e2e-drill; KILLS=fault-masking, heal-coupling; VERDICT=KEEP; OVERLAP=recovery double crash (kill+restart; this is live staged heal)

## Helper patterns (testkit candidates)

Repeated verbatim or near-verbatim across the pass files in each area; each is one extractable module:

- CLI kit (`tests/cli/error_testkit.rs`): asgrep_bin/run/sargs/parse_stdout/assert_failure_envelope/assert_human_error/assert_no_success_shape/fixture_root/index_root — identical in all 4 CLI files; pass4 adds assert_success_envelope/assert_human_success/index_db_path/corrupt_index_db/assert_fixture_hits (fold in).
- Codemode kit (`tests/codemode/error_testkit.rs`): session_at/config_at/batch_request/batch_call/serve_lines/serve_request_line — identical in all 4 codemode files; plus discriminant() projection (pass3) and assert_other_preserves_io_cause (pass2) as shared asserts.
- Core kit (`tests/core/error_testkit.rs`): err_of/discriminant(u8)/mem_searcher/write_garbage_db/indexer_at/searcher_at — repeated across all 4 core files; pass4 adds populate/remove_db_files/assert_clean_state (fold in).
- MCP kit (`tests/mcp/error_testkit.rs`): mcp_bin/init_payload/initialized_notif/rpc_session(+_env/_raw/pipeline variants)/tool_call/tool_text/tool_body/assert_tool_error_shape/assert_tool_success_shape/assert_jsonrpc_error/tool_error_discriminant/file_tree/indexed_tree/corrupt_index_db/LiveSession — repeated across all 4 MCP files with only sentinel-string and strictness (content-len check) drift; unify on the strictest shape.
- Lang pipeline kit (`tests/lang/error_testkit.rs`): score_hit/rank_hits/assert_rank_sound/assert_spans_in_source/run_pipeline/PipelineOutcome (pass4 in-test score+rank lane) — single use today but the canonical hostile-input pipeline runner; extract so future drills reuse rather than fork.
- Cross-cutting: three parallel failure-envelope asserts (CLI machine envelope, MCP tool-error envelope, JSON-RPC error row) — keep per-protocol, but unify the discriminant-projection idiom (tuple of shape fields, message excluded) already shared by codemode-pass3/MCP-pass3/pass4.

## Proposed structure

Keep the 4-files-per-area layout (taxonomy/propagation/relations/drills reads well); consolidate inside it:

- `tests/<area>/error_api_pass1.rs`: grouped taxonomy rows only — CLI folds 12 cells into 2 family tests (usage/operational anchors already marked KEEP); core folds 10 Other rows into 3 group tests (io-bounds, search-ingress, batch/shape); codemode/lang/MCP pass1 already grouped, keep.
- `tests/<area>/error_api_pass2.rs`: propagation pins stay per-contract; fold same-contract second triggers (CLI file-filter ×2 → 1, in-scope ×2 → 1; core file-as-root, garbage ×3 → 1 multi-layer, cancel ×2 → 1; codemode short-circuit ×2 → 1, parallel → E3 mode-equivalence).
- `tests/<area>/error_api_pass3.rs`: one test per relation — CLI agreement ×3 → 1, determinism ×2 → 1; codemode dispatch ×3 → 1; core dropped-table ×3 → 1, cancel → E2; lang degenerate repetition split into E3-01/E3-09; MCP D1+D2 → 1.
- `tests/<area>/error_api_pass4.rs`: drills stay per-fault-class; fold same-shape second drills (CLI bad-lang → conflict drill; codemode plan-prefix → E3 fail-closed + resume half; core truncated → corrupt drill; lang garbage → empty drill; MCP D3 → D1, P10 → P2).
- `tests/<area>/error_testkit.rs` (5 new modules): extract the helper patterns above; deletes ~600 lines of duplicated helpers across the 20 files.
- Net: 213 → 161 tests (50 merged, 2 deleted), 20 → 25 files (20 consolidated + 5 testkits).

## Counts

- Total: 213 (CLI 45, codemode 44, core 46, lang 40, MCP 38)
- By category: taxonomy 61, propagation 58, negative-metamorphic 51, e2e-drill 39, other 4
- By verdict: KEEP 161, MERGE 50, DELETE 2
- By file (KEEP/MERGE/DELETE): cli-pass1 2/12/0, cli-pass2 10/2/0, cli-pass3 7/4/0, cli-pass4 7/1/0; codemode-pass1 9/3/0, codemode-pass2 11/2/1, codemode-pass3 8/2/0, codemode-pass4 7/1/0; core-pass1 4/10/0, core-pass2 9/4/0, core-pass3 8/3/0, core-pass4 7/1/0; lang-pass1 12/0/0, lang-pass2 10/0/0, lang-pass3 8/1/1, lang-pass4 7/1/0; mcp-pass1 11/0/0, mcp-pass2 9/1/0, mcp-pass3 9/1/0, mcp-pass4 6/1/0
- Oracle/recovery/protocol overlaps flagged: 26 entries carry OVERLAP notes; all kept overlaps add a new discriminant, relation, or recovery leg the earlier pin lacks.

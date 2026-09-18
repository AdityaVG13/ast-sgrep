# Oracle codemode catalog (foundry pass1–4, by intent)

Source: `tests/codemode/oracle_foundry_pass{1,2,3,4}.rs`. All expectations hand-computed; failures assert discriminants via `matches!`, never Display text.

## pass1 — L1 single-surface oracles (6)

- output_format_parse_matches_hand_table: INTENT=parse() maps 13 aliases case-insensitively, rejects bogus/empty; CAT=other; KILLS=alias-arm-drop; VERDICT=KEEP
- compact_budget_default_matches_hand_values: INTENT=CompactBudget defaults are 96/768; CAT=budget; KILLS=default-drift; VERDICT=MERGE→output_budget_default_and_select_floor (fold 2 asserts as budget-defaults leg)
- detail_levels_order_and_label_by_hand: INTENT=4 levels ordered Metadata<Signature<Block<Full with exact labels; CAT=budget; KILLS=order-swap/label-drift; VERDICT=KEEP
- example_plan_parses_with_nonempty_steps: INTENT=example_plan() parses, every step has id+tool; CAT=plan; KILLS=fixture-rot; VERDICT=KEEP (sole example_plan pin)
- plan_parse_failures_carry_invalid_args_discriminant: INTENT=4 malformed plans → InvalidArgs, empty steps → Ok; CAT=plan; KILLS=validation-guard-removal; VERDICT=KEEP
- testkit_fixture_helpers_resolve_to_real_content: INTENT=sample_root is a dir, src/main.rs non-empty; CAT=other; KILLS=fixture-rot; VERDICT=KEEP (sole filesystem-anchor pin)

## pass2 — L2 mutation-discriminating oracles (11)

- tool_name_aliases_resolve_exactly: INTENT=26 alias/case/whitespace cases resolve exactly + catalog roundtrip; CAT=other; KILLS=alias-arm-drop/case-fold/trim/as_str-parse-skew; VERDICT=KEEP
- call_budget_boundary_is_calls_gte_max: INTENT=3rd call at max 2 → BudgetExhausted(2), max 0 refuses immediately; CAT=budget; KILLS=>=-vs->-flip/zero-budget-blindness; VERDICT=KEEP
- run_plan_refuses_empty_duplicate_and_dangling: INTENT=run rejects empty/dup-id/dangling-$ref, good plan ok+count 1; CAT=plan; KILLS=empty-accept/dup-blindness/ref-blindness/ok-lie; VERDICT=KEEP
- batch_validation_rejects_before_execution: INTENT=rejects empty/33-calls/bad-id/bad-tool, accepts 32/128B, unknown-128B-tool dispatches per-call; CAT=batch; KILLS=ceiling-drop/guard-removal; VERDICT=KEEP
- catalog_search_empty_all_case_fold_miss: INTENT=empty→all, SEARCH folds, junk→none, describe exact-only; CAT=other; KILLS=filter-removal/case-fold-removal; VERDICT=KEEP
- output_budget_default_and_select_floor: INTENT=default 900/Block, starved→Metadata-never-drop, funded→Full, cost sums, render bodies exact; CAT=budget; KILLS=default-drift/evidence-drop/upgrade-removal/cost-mutant; VERDICT=KEEP
- miss_reason_labels_and_next_steps: INTENT=4 labels + 5 next_step branches exact; CAT=other; KILLS=label-swap/branch-mutant; VERDICT=KEEP
- golden_canonicalize_trims_unifies_terminates: INTENT=6 point-values for trim/CRLF-unify/blank-pop/terminate; CAT=other; KILLS=trim-drop/CRLF-drop/pop-drop/terminate-flip; VERDICT=KEEP
- scrubber_machine_contract_splits_version_fields: INTENT=scrubs version, keeps schema_version, UUID→placeholder, none passthrough; CAT=other; KILLS=under-scrub/over-scrub; VERDICT=KEEP
- single_result_wraps_main_with_count_1: INTENT=single_result sets ok/count 1/return-echo/steps[main]; CAT=plan; KILLS=ok-flag/step-key/count-mutant; VERDICT=MERGE→run_plan_refuses_empty_duplicate_and_dangling (fold as positive-shape leg; tiny ctor needs no standalone test)
- session_pins_relative_index_under_root: INTENT=relative index pins under root, absolute untouched; CAT=session; KILLS=jail-escape/absolute-rewrite; VERDICT=KEEP

## pass3 — L3 metamorphic/differential/adversarial (13)

- plan_rerun_is_deterministic_across_fresh_sessions: INTENT=same plan × 2 sessions → identical steps/return/count 2, tools[0]=search; CAT=plan; KILLS=render-cache/order-nondeterminism; VERDICT=KEEP
- batch_validation_and_outcome_are_order_independent: INTENT=permuted calls permute results, per-id ok-map + counts stable; CAT=batch; KILLS=order-sensitive-validation; VERDICT=KEEP
- batch_serial_parallel_paths_agree_per_call: INTENT=serial/parallel agree per-id (ok,value), labels differ; CAT=batch; KILLS=path-divergence; VERDICT=KEEP
- budget_exhaustion_is_monotone_and_sticky: INTENT=tight≤roomy counts (1,2), exhaustion sticky with same payload; CAT=budget; KILLS=budget-reset/off-by-one; VERDICT=KEEP
- empty_plan_and_batch_edges_fail_stably: INTENT=empty plan/batch fail InvalidArgs twice identically; empty filter/select → zero shapes; CAT=other; KILLS=error-caching/empty-flap; VERDICT=KEEP (repeat-stability + zero-shape legs are new vs pass2)
- golden_canonicalize_is_idempotent_fixpoint: INTENT=canonicalize is a fixpoint over 10-input corpus + 2 hand fixpoints; CAT=other; KILLS=non-fixpoint-mutant; VERDICT=MERGE→golden_canonicalize_trims_unifies_terminates (same fn, overlapping corpus; append loop + 2 asserts)
- plan_ref_adversarial_matrix_fails_invalid_args: INTENT=7 $ref attacks (self/forward/missing/overrun/scalar/bare-$/object) → InvalidArgs, never panic/Null; CAT=plan; KILLS=ref-resolution-hole/panic; VERDICT=KEEP
- plan_default_return_equals_explicit_last_ref: INTENT=omitted return == explicit $last; solo + indexed paths exact; CAT=plan; KILLS=default-return/path-mutant; VERDICT=KEEP
- batch_result_shaping_echoes_ids_in_order_with_exclusive_fields: INTENT=order/ids echo, ok⊕error exclusive, dup ids both run; CAT=batch; KILLS=shaping/dedup/exclusivity-mutant; VERDICT=KEEP
- error_taxonomy_splits_unknown_tool_from_invalid_args: INTENT=unknown→UnknownTool ×4, bad-args→InvalidArgs ×14, batch-unknown stays per-call; CAT=session; KILLS=taxonomy-collapse; VERDICT=KEEP
- unicode_and_empty_adversarial_inputs: INTENT=unicode ids roundtrip, unicode tool unknown, unicode query deterministic-empty, batch id echoes; CAT=other; KILLS=unicode-roundtrip/case-mutant; VERDICT=KEEP
- parallel_mode_precedence_and_max_batch_order: INTENT=explicit mode beats legacy bool, legacy true parallelizes, 32 results ordered; CAT=batch; KILLS=precedence/max-order-mutant; VERDICT=KEEP
- pure_transforms_agree_across_call_plan_batch_paths: INTENT=filter_hits/select agree across call/plan/batch + limit-monotone + catalog wrap; CAT=other; KILLS=dispatch-path-divergence; VERDICT=KEEP

## pass4 — L4 end-to-end compositions (10)

- indexed_plan_execute_shape_golden_flow: INTENT=index→search→filter→select→golden text, hit_count 1 byte-exact; CAT=e2e; KILLS=index-shape-golden-break; VERDICT=KEEP
- batch_output_feeds_plan_end_to_end: INTENT=batch tools (10, search,find) feed plan select → 2 names; CAT=e2e; KILLS=cross-surface-wiring/catalog-drift; VERDICT=KEEP (count 10 hand-derived: 8 kind-hits + filter_hits + catalog_search; fails loudly on catalog change by design)
- rerun_determinism_freezes_golden_text: INTENT=2 reruns + direct path → byte-identical golden, batch leg agrees; CAT=e2e; KILLS=golden-text-nondeterminism; VERDICT=KEEP (text-level; pass3 pins Value-level — complementary)
- budget_exhaustion_midplan_with_batch_isolation: INTENT=3-step/max-2 dies at last step (count 2, ref burns none), batch unaffected; CAT=e2e; KILLS=ref-burns-budget/isolation-leak; VERDICT=KEEP
- missing_root_fail_closed_pure_vs_bound: INTENT=missing root: pure Ok, bound→Other, plan dies at bound step, batch per-call; CAT=e2e; KILLS=fail-open-on-missing-root; VERDICT=KEEP
- root_escape_fail_closed_across_surfaces: INTENT=child Ok, foreign root→Other on direct/plan/batch; CAT=e2e; KILLS=root-jail-escape; VERDICT=KEEP
- oversized_response_fail_closed_across_surfaces: INTENT=cap+1 select → Other direct/plan, per-call in batch, small Ok; CAT=e2e; KILLS=cap-bypass; VERDICT=KEEP
- error_taxonomy_bump_order_and_bound_split: INTENT=unknown 2nd step consumes 2 calls; bound-arg→Other vs pure-arg→InvalidArgs; batch mirrors; CAT=e2e; KILLS=bump-after-dispatch/taxonomy-collapse; VERDICT=KEEP
- session_read_agrees_with_budget_rendering: INTENT=live read text == budget excerpt; Block upgrade, Signature hand-shape, cost==len; CAT=e2e; KILLS=read-render-skew; VERDICT=KEEP
- degenerate_inputs_fail_closed_or_documented: INTENT=missing read/edit→Other, empty id→InvalidArgs, impossible threshold→0 hits, solo forced-parallel pins serial; CAT=e2e; KILLS=fail-open-on-degenerate; VERDICT=KEEP (grab-bag but each leg is a documented degenerate contract)

## Helper patterns

- `session_at` (pass2/3/4, identical): temp-root session, limit 5, no embed, AgentCapsule default. `config_at` (pass3/4, identical): same as SessionConfig.
- `batch_request` (pass2/3/4, identical): all-None envelope around calls. `catalog_call` (pass2/3: 1-arg query=search; pass4: 2-arg with query) — signatures diverged, unify to 2-arg.
- `search_plan` (pass3 only): 2-step catalog→select fixture. `indexed_repo` (pass4 only): fixed-content temp repo + index.db config, caller keeps TempDir alive.
- `sample_hit` duplicated with drift: pass2 (src/lib.rs, foo) vs pass4 (lib.rs, alpha) — same struct, unify.
- Inline closures: `by_id` sorter (order-independence), `golden` pretty+canonicalize (pass4) — repeated pretty-print idiom could be one helper.
- Duplication note: `session_at`/`config_at`/`batch_request` copy-pasted across 3 files; a shared `tests/codemode/common.rs` (mod-gated) would kill ~60 lines but cross-file test mod wiring needs care.

## Counts

- Files: pass1 6, pass2 11, pass3 13, pass4 10 → 40 total.
- CAT: plan 7, batch 5, budget 4, session 2, e2e 10, other 12 → 40.
- VERDICT: KEEP 37, MERGE 3 (compact_budget_default, single_result, golden_fixpoint), DELETE 0.
- No zero-assert or one-assert tests found; every test has ≥2 asserts or a table loop. Tautology risk: none — all expectations are hand-computed literals/tables.

# numerical-codemode catalog

Intent catalog of `tests/codemode/numerical_pass{1,2,3,4}.rs` (39 tests, pure transforms + tempdir disk reads).
Merge targets are proposed per-surface contract tests (one anchor per arithmetic surface).

## pass1 — hand-computed numeric oracles (Mission N1)

- filter_min_score_boundary_is_inclusive: INTENT=reject is strict `<`: score==min survives (2.0/2.001 kept, 1.999 cut); CAT=exactness; KILLS=comparison-flip (`<`-vs-`<=`) mutant; VERDICT=MERGE→filter_hits-contract
- filter_missing_score_defaults_to_zero: INTENT=missing score reads as 0.0 (kept at min 0.0/-1.0, cut at 0.5); CAT=exactness; KILLS=default-value (`unwrap_or(0.0)`) mutant; VERDICT=MERGE→filter_hits-contract
- filter_limit_zero_clamps_to_one: INTENT=filter limit clamps [1,1000]: 0→1 hit survives, 99999→all 3, no error; CAT=exactness; KILLS=clamp-bound (floor/ceiling) mutant; VERDICT=MERGE→filter_hits-contract
- select_limit_zero_empties_array: INTENT=select limit 0 truncates to `[]` (no clamp; opposite of filter); CAT=exactness; KILLS=truncate-vs-clamp (added-clamp) mutant; VERDICT=MERGE→select-contract
- budget_constants_and_fresh_session_are_hand_computed: INTENT=byte caps are 4MiB / 4MiB-64KiB / 8KiB; fresh session is 64-budget/0-used/not-exhausted; CAT=exactness; KILLS=constant-value (byte-arithmetic literal) mutant; VERDICT=MERGE→batch-contract (caps half) + budget-contract (fresh-session half)
- read_start_end_clamp_to_valid_range: INTENT=start `.max(1)`, end `.max(start)`: 0→line 1, end 0 pins to start line; CAT=exactness; KILLS=clamp-bound (`max` floor) mutant; VERDICT=MERGE→read-contract
- read_context_lines_widens_symmetrically: INTENT=ctx 1 on line 3→lines 2..4; huge ctx saturates to whole file (min 100); CAT=exactness; KILLS=off-by-one-widen / missing-saturation mutant; VERDICT=MERGE→read-contract
- read_char_budgets_truncate_with_flag: INTENT=max_chars clamps [1,100000] with truncated flag; 2500-char line pins to 2000; CAT=exactness; KILLS=clamp-bound / truncated-flag mutant; VERDICT=MERGE→read-contract
- unknown_tool_suggestion_threshold: INTENT=typo within len/3 Levenshtein budget suggests (`serach`), far name (`xyzq`) does not; CAT=exactness; KILLS=distance-threshold (budget-formula) mutant; VERDICT=KEEP

## pass2 — degenerate-input totality (Mission N2)

- filter_non_numeric_min_score_is_ignored: INTENT=string/bool/null/object/array/NaN min_score disables filtering (all 3 survive); CAT=totality; KILLS=type-coercion (`as_f64` None-path) mutant; VERDICT=MERGE→filter_hits-contract
- filter_non_numeric_score_defaults_to_zero: INTENT=string/bool/null/missing scores read as 0.0 across min 0.0/0.5/-1.0; CAT=totality; KILLS=default-value (`as_f64().unwrap_or(0.0)`) mutant; VERDICT=MERGE→filter_hits-contract
- filter_negative_and_non_numeric_limit_defaults_to_max: INTENT=-1/float/string/bool/null/object limits fall back to 1000 (all survive, never error); CAT=totality; KILLS=type-coercion (`as_u64` None → unwrap_or MAX) mutant; VERDICT=MERGE→filter_hits-contract
- select_negative_huge_and_non_numeric_limit_keeps_all: INTENT=select ignores -1/u64MAX/float/string/null limits (no truncate, all 3 kept); CAT=totality; KILLS=type-coercion (`as_u64` None means no-truncate) mutant; VERDICT=MERGE→select-contract
- session_huge_budget_never_exhausts_on_few_calls: INTENT=max_calls=usize::MAX serves calls without tripping or overflow; CAT=totality; KILLS=budget-overflow (saturating-add/exhausted-predicate) mutant; VERDICT=MERGE→budget-contract
- batch_limit_zero_and_huge_clamp_without_panic: INTENT=batch limit clamps [1,500] (0→1, MAX→500); batch stays Ok/all_ok/serial; CAT=totality; KILLS=clamp-bound (batch-limit) mutant; VERDICT=MERGE→batch-contract
- search_limit_zero_huge_and_negative_totality: INTENT=find limit 0→1 hit, huge/-1/string→fallback counts (1,3,3,3) on 3-file fixture; CAT=totality; KILLS=clamp-bound + type-coercion (`unwrap_or` config default) mutant; VERDICT=MERGE→find-contract
- read_negative_start_end_default_to_one: INTENT=negative/string read bounds default to 1..1; negative end pins to start; CAT=totality; KILLS=type-coercion (`as_u64` None → unwrap_or) mutant; VERDICT=MERGE→read-contract
- read_beyond_eof_and_huge_end_are_total: INTENT=start past EOF yields empty untruncated text; huge end/max_chars saturate to full file; CAT=totality; KILLS=clamp-bound (EOF-saturation) mutant; VERDICT=MERGE→read-contract
- empty_batch_and_empty_plan_reject_under_degenerate_budgets: INTENT=empty batch/plan reject InvalidArgs even under limit/budget 0 or MAX (validation precedes clamp); CAT=totality; KILLS=error-discriminant + validation-order mutant; VERDICT=MERGE→batch-contract (batch half) + plan-contract (plan half)
- read_empty_refs_rejects_without_panic: INTENT=empty refs / missing path fail closed as Other, never Ok-empty or panic; CAT=totality; KILLS=error-discriminant (Other-vs-Ok) mutant; VERDICT=MERGE→read-contract

## pass3 — metamorphic relations (Mission N3)

- budget_larger_never_serves_fewer_calls: INTENT=served(b) non-decreasing over budgets 1..8 with saturating attempts; CAT=metamorphic; KILLS=non-monotonic-budget (inverted-cap) mutant; VERDICT=MERGE→budget-contract
- budget_served_never_exceeds_budget: INTENT=served(b)==b exactly (cap binds, one call each, exhausted after); CAT=metamorphic; KILLS=budget-accounting (counter-increment/exhausted-predicate) mutant; VERDICT=MERGE→budget-contract
- min_score_lower_threshold_never_yields_fewer_hits: INTENT=hit counts non-decreasing as min descends 6.0→-1.0 through every fixture score; CAT=metamorphic; KILLS=comparison-flip (inverted-reject) mutant; VERDICT=MERGE→filter_hits-contract
- min_score_higher_result_nests_inside_lower: INTENT=higher-threshold file list is a subsequence of every lower one (order preserved); CAT=metamorphic; KILLS=ordering (unstable-filter/reorder) mutant; VERDICT=MERGE→filter_hits-contract
- filter_limit_topk_is_prefix_of_topm: INTENT=limit k<m yields strict prefix: counts grow and out_k==out_m[..k]; CAT=metamorphic; KILLS=ordering (truncate-reorder) mutant; VERDICT=MERGE→filter_hits-contract
- select_limit_topk_is_prefix_of_unlimited: INTENT=select limit k equals first k rows of the unlimited projection; CAT=metamorphic; KILLS=ordering (select-reorder) mutant; VERDICT=MERGE→select-contract
- find_limit_topk_is_prefix_of_topm: INTENT=find limit-1 hits equal the first hit of the limit-5 run (rank then truncate); CAT=metamorphic; KILLS=rank-vs-truncate-order (truncate-before-rank) mutant; VERDICT=MERGE→find-contract
- batch_call_order_does_not_change_counts_or_per_id_results: INTENT=forward vs reversed batch agree on counts/mode/per-id (ok,value); CAT=metamorphic; KILLS=order-dependence (positional-result-mapping) mutant; VERDICT=MERGE→batch-contract
- filter_rerun_is_deterministic: INTENT=same filter call twice on one session yields byte-identical JSON; CAT=metamorphic; KILLS=nondeterministic-ordering (hash-iteration) mutant; VERDICT=MERGE→filter_hits-contract
- find_rerun_is_deterministic: INTENT=same lexical find twice on one indexed session yields identical hits; CAT=metamorphic; KILLS=nondeterministic-ranking mutant; VERDICT=MERGE→find-contract
- batch_rerun_is_deterministic: INTENT=same batch twice agrees on mode/counts/payloads (wall_ms excluded); CAT=metamorphic; KILLS=nondeterministic-batch (payload/mode-wobble) mutant; VERDICT=MERGE→batch-contract

## pass4 — end-to-end numeric drills (Mission N4)

- budget_mixed_tool_flow_trips_at_exact_count: INTENT=4 distinct tools consume 1..4, 5th trips BudgetExhausted(4), counter pinned; CAT=numeric-drill; KILLS=budget-accounting (per-tool-miscount/payload) mutant; VERDICT=MERGE→budget-contract
- plan_budget_exact_trip_and_success_counts: INTENT=7-step plan trips on step 5 under budget 4, succeeds with exact outputs under budget 7; CAT=numeric-drill; KILLS=plan-budget-accounting (step-count/trip-point) mutant; VERDICT=KEEP
- threshold_sweep_exact_survivor_counts: INTENT=sweep min 6..0 yields survivor vector [1,2,3,4,5,6,6] with exact order, 8 calls; CAT=numeric-drill; KILLS=comparison-flip + ordering (absolute-sweep) mutant; VERDICT=MERGE→filter_hits-contract
- batch_size_sweep_exact_counts: INTENT=batch sizes 1..32 report call_count==N, N ok, all_ok, serial; CAT=numeric-drill; KILLS=batch-counting (call_count/results-length) mutant; VERDICT=MERGE→batch-contract
- batch_threshold_sweep_exact_per_id_counts: INTENT=5-call threshold batch yields exact per-id map {t5:2,t4:3,t3:4,t2:5,t1:6}; CAT=numeric-drill; KILLS=per-id-mapping (crossed-results) mutant; VERDICT=MERGE→batch-contract
- filter_then_select_topk_exact_orderings: INTENT=filter min2.0/limit4 → [f6..f3], select top-2/3 exact rows, 3 calls total; CAT=numeric-drill; KILLS=chain-ordering (cross-tool-order-break) mutant; VERDICT=KEEP
- find_filter_read_select_pipeline_exact_counts: INTENT=index→find(4)→path-filter(1)→read(exact text)→select(3 rows) with per-step counts; CAT=numeric-drill; KILLS=BEHAVIOR-ONLY; VERDICT=KEEP
- read_fanout_exact_window_counts: INTENT=one read over 3 refs yields count 3 with exact (start,end,text) in ref order; CAT=numeric-drill; KILLS=fanout-ordering (window-reorder/drop) mutant; VERDICT=MERGE→read-contract

## Helper patterns

- `session_at(root)`: unindexed `CodeModeSession` (limit 5, lexical, AgentCapsule) for pure-transform tests; identical copy in all four files.
- `indexed_session_at(root)`: tempdir index + `index_repo` call asserting `ok:true`; returns `(TempDir, session)` so the index dir outlives the test; identical copy in all four files.
- `hits_fixture()` (pass1/2) / `scored_hits()` (pass3/4): fixed-score hit arrays; pass1/2 use 3 hits straddling 2.0, pass3 uses 5 hits (5..1), pass4 uses 6 hits (6..1) with input order == score order.
- `files_of(out)`: extracts `hits[].file` as `Vec<String>` for order assertions (pass3/4).
- `is_subsequence(needle, haystack)` (pass3): order-preserving nesting check for threshold relations.
- `serve_until_exhausted(session, attempts)` (pass3): drives identical pure calls until `BudgetExhausted`, asserting the trip discriminant is always budget (never another error).
- `config_at(root)` + `batch_request(...)` + `catalog_call(id[, query])` (pass2/3/4): `run_batch` harness over pure `catalog_search`/`filter_hits` calls; pass2 variant takes an explicit `limit` for clamp tests.
- `parse_plan` + `run_plan` (pass2/4): plan harness; pass2 for empty-plan rejection, pass4 for the 7-step budget drill.
- Index discipline: `find`/`read` math tests index first (lexical only, deterministic); `read` fixtures omit trailing newline where line-count oracles would shift (pass1 context test).

## Counts

- Total: 39 tests (pass1: 9, pass2: 11, pass3: 11, pass4: 8).
- By CAT: exactness 9, totality 11, metamorphic 11, numeric-drill 8, other 0.
- By VERDICT: KEEP 4, MERGE 35, DELETE 0.
- By merge target (full tests + split halves): filter_hits-contract 11, read-contract 7, batch-contract 5+2 halves, budget-contract 4+1 half, select-contract 3, find-contract 3, plan-contract 0+1 half.
- KEEP set (standalone intents, each is its own surface contract): `unknown_tool_suggestion_threshold` (sole suggestion-distance owner), `plan_budget_exact_trip_and_success_counts` (sole full-plan execution drill), `filter_then_select_topk_exact_orderings` (sole filter→select chain drill), `find_filter_read_select_pipeline_exact_counts` (sole full find→filter→read→select pipeline).
- Split tests (span two surfaces, halves merge separately): `budget_constants_and_fresh_session_are_hand_computed` (caps→batch, fresh session→budget), `empty_batch_and_empty_plan_reject_under_degenerate_budgets` (batch→batch, plan→plan).

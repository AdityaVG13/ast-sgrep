# numerical-cli catalog

Intent catalog of `tests/cli/numerical_pass{1,2,3,4}.rs` (40 tests, all real-binary `asgrep` subprocess runs).
Merge targets are proposed per-surface contract tests; the named anchor (KEEP) absorbs the listed arms.
Strict calibration: per-metric / per-limit / per-flag variants MUST merge; KEEP is for standalone intents only.

## pass1 — eval metric math + token-budget caps (Mission N1)

- eval_total_miss_yields_exact_zeros: INTENT=Total miss yields exact zeros across RR/nDCG/recall and aggregates; CAT=exactness; KILLS=zero-guard-omission (miss emits garbage/NaN); VERDICT=MERGE→eval-arithmetic-contract
- eval_single_rank1_hit_yields_exact_ones: INTENT=Rank-1 single hit yields exact ones across RR/nDCG/recall and aggregates; CAT=exactness; KILLS=reciprocal/log-base-error (RR!=1/1, DCG base!=2); VERDICT=MERGE→eval-arithmetic-contract
- eval_k1_truncation_halves_recall_keeps_ndcg_one: INTENT=k=1 truncation halves recall but keeps nDCG=1 via IDCG(min(relevant,k)); CAT=exactness; KILLS=IDCG-clamp-omission (IDCG over all relevant); VERDICT=MERGE→eval-k-sweep-contract
- eval_two_of_three_rounds_recall_to_667_and_333: INTENT=2-of-3 recall rounds through round3 to 0.667/0.333 with nDCG=1; CAT=exactness; KILLS=rounding-omission (raw 0.6666667 leaks); VERDICT=MERGE→eval-arithmetic-contract
- eval_mixed_hit_miss_halves_mrr_and_ndcg: INTENT=Hit+miss 2-query aggregates average to exactly 0.5; CAT=exactness; KILLS=aggregation-mean-error (wrong divisor); VERDICT=MERGE→eval-arithmetic-contract
- eval_human_table_rounds_to_three_decimals: INTENT=Human eval table renders {:.3} in per-query row and MRR summary line; CAT=exactness; KILLS=format-precision-mutant ({:.2}/unrounded); VERDICT=KEEP
- budget_tokens_cap_names_65536: INTENT=Over-cap --budget-tokens usage message names the exact max 65536; CAT=totality; KILLS=cap/message-regression (wrong max in code or text); VERDICT=KEEP
- snippet_tokens_cap_names_4096: INTENT=Over-cap --snippet-tokens usage message names the exact max 4096; CAT=totality; KILLS=cap/message-regression; VERDICT=MERGE→budget-cap-contract
- response_snippet_tokens_cap_names_65536: INTENT=Over-cap --response-snippet-tokens usage message names the exact max 65536; CAT=totality; KILLS=cap/message-regression; VERDICT=MERGE→budget-cap-contract
- budget_tokens_max_is_accepted: INTENT=Cap-boundary value 65536 parses and searches exit 0 / ok:true; CAT=totality; KILLS=boundary-off-by-one (>= vs >); VERDICT=MERGE→budget-cap-contract

## pass2 — degenerate numeric totality (Mission N2)

- limit_zero_remaps_to_default: INTENT=--limit 0 and ASGREP_LIMIT=0 remap to default 16 in envelope; CAT=totality; KILLS=remap-omission (0 passes through); VERDICT=KEEP
- limit_negative_equals_form_is_usage: INTENT=--limit=-1 equals form is exit-1 usage on stdout; CAT=totality; KILLS=parse-accept-garbage (negative accepted); VERDICT=MERGE→limit-parse-contract
- limit_nonnumeric_and_overflow_are_usage: INTENT=Non-numeric and 2^64 --limit values are exit-1 usage; CAT=totality; KILLS=parse-fail-open (silent fallback or panic); VERDICT=KEEP
- env_limit_garbage_is_usage: INTENT=ASGREP_LIMIT=abc is exit-1 usage, never silent fallback; CAT=totality; KILLS=env-parser-bypass (env skips clap parser); VERDICT=MERGE→limit-parse-contract
- zero_size_windows_are_accepted: INTENT=Zero excerpt/snippet/budget windows accepted exit 0, incl. all-zero hit run; CAT=totality; KILLS=zero-rejection (0 treated as invalid); VERDICT=KEEP
- unclamped_usize_flags_accept_zero_reject_overflow: INTENT=Unbounded usize flags accept 0, reject usize-overflow as usage; CAT=totality; KILLS=overflow-wrap-panic; VERDICT=MERGE→zero-window-contract
- call_path_bounds_reject_zero_and_over_max: INTENT=call-path max-depth/nodes/edges reject 0 and over-max as usage; CAT=totality; KILLS=range-check-omission; VERDICT=KEEP
- eval_k_zero_yields_exact_zeros: INTENT=eval k=0 succeeds with exact zeros, all floats JSON numbers; CAT=totality; KILLS=NaN-leak (0/0 renders null); VERDICT=KEEP
- eval_empty_relevant_yields_exact_zeros: INTENT=eval relevant=[] succeeds with exact zeros via 0/0 guards; CAT=totality; KILLS=div-by-zero (unguarded recall_of); VERDICT=MERGE→eval-degenerate-contract
- eval_degenerate_gold_fails_closed: INTENT=Empty-queries / non-usize-k gold fails closed exit-2 operational; CAT=totality; KILLS=fail-open (fabricated zeros with ok:true); VERDICT=KEEP
- eval_empty_corpus_fails_closed: INTENT=eval on empty corpus fails closed exit-2, never all-zero MRR; CAT=totality; KILLS=fail-open; VERDICT=MERGE→eval-failclosed-contract
- eval_usize_max_k_stays_finite: INTENT=eval k=usize::MAX succeeds with finite unit metrics and rank-1 hit; CAT=totality; KILLS=limit-derivation-overflow (unclamped huge k wraps/panics); VERDICT=MERGE→eval-degenerate-contract

## pass3 — metamorphic relations (Mission N3)

- search_limit_growth_never_shrinks_hits: INTENT=Growing --limit never shrinks hits and never reports more than limit; CAT=metamorphic; KILLS=limit-count-inversion; VERDICT=MERGE→search-limit-contract
- search_smaller_limit_hits_are_a_prefix_of_larger: INTENT=Smaller-limit hits are exact-JSON prefix of larger-limit hits; CAT=metamorphic; KILLS=rank-then-truncate-violation (per-limit re-rank); VERDICT=MERGE→search-limit-contract
- search_rerun_yields_identical_hit_bytes: INTENT=Repeated search yields byte-identical hit payload; CAT=metamorphic; KILLS=nondeterminism (hash order/timestamps in hits); VERDICT=KEEP
- eval_k_growth_never_lowers_found_or_recall: INTENT=Growing eval k never lowers found/recall and never worsens rank; CAT=metamorphic; KILLS=cutoff-inversion; VERDICT=MERGE→eval-k-sweep-contract
- eval_recall_cutoffs_are_monotone: INTENT=recall@1<=recall@5<=recall@20 per query and in aggregate; CAT=metamorphic; KILLS=cutoff-clamp-bug (narrow scan exceeds wide); VERDICT=KEEP
- eval_hit_scores_no_lower_than_miss: INTENT=Rank-1-hit query dominates total-miss query on RR/nDCG/all recalls; CAT=metamorphic; KILLS=TAUTOLOGY-RISK+exact endpoint tests already pin hit=1/miss=0 in identical shapes; VERDICT=DELETE+no independent killing power beyond exact endpoint tests
- eval_aggregates_stay_within_per_query_band: INTENT=Aggregates sit in per-query [min,max] +/-0.001 with exact recall@k mean; CAT=metamorphic; KILLS=aggregation-mean-error; VERDICT=MERGE→unrounded-mean-contract
- eval_reported_floats_are_round3_fixed_points: INTENT=Every reported float is a round3 fixed point (v*1000 integral); CAT=metamorphic; KILLS=rounding-omission (unrounded float leaks); VERDICT=MERGE→eval-arithmetic-contract
- eval_rerun_yields_identical_metric_bytes: INTENT=Repeated eval yields byte-identical queries+aggregate payload; CAT=metamorphic; KILLS=nondeterminism in metric payload; VERDICT=MERGE→search-determinism-contract
- eval_human_and_json_metrics_agree: INTENT=Human and JSON renderings agree within 0.001, counts/rank exact; CAT=metamorphic; KILLS=rendering-divergence (separate formulas drift); VERDICT=KEEP

## pass4 — end-to-end metric drills (Mission N4)

- eval_three_query_fractional_aggregates: INTENT=Hit+miss+2-of-4 corpus pins hand-computed fractional aggregates; CAT=metric-drill; KILLS=aggregation-mean-error/wrong-denominator; VERDICT=KEEP
- eval_five_of_six_rounds_recall_to_833_and_167: INTENT=5-of-6 recall rounds to 0.833/0.167 with nDCG=1; CAT=metric-drill; KILLS=rounding-omission; VERDICT=MERGE→eval-arithmetic-contract
- eval_k_sweep_found_and_recall_match_hand_values: INTENT=k=1..5 sweep pins found 1,2,3,4,4 and recall 0.25-1.0; CAT=metric-drill; KILLS=found/recall-off-by-one across k; VERDICT=KEEP
- search_limit_sweep_hit_counts_match_hand_values: INTENT=Limits 1..7 pin counts 1..5,5,5 with limit echo and exact file set; CAT=metric-drill; KILLS=limit-count-off-by-one/limit-echo-drop; VERDICT=KEEP
- eval_filename_ordered_decoy_forces_rank_two: INTENT=Path-ordered decoy forces rank 2 with RR=0.5/nDCG=0.631; CAT=metric-drill; KILLS=rank-order-bug+DCG-log-error; VERDICT=KEEP
- eval_two_filename_ordered_decoys_force_rank_three: INTENT=Two path-ordered decoys force rank 3 with RR=0.333/nDCG=0.5; CAT=metric-drill; KILLS=RR-reciprocal-error/rank-order-bug; VERDICT=MERGE→adversarial-rank-contract
- eval_adversarial_plus_clean_aggregate_uses_unrounded_means: INTENT=Rank-2+rank-1 mix pins unrounded-mean nDCG=0.815, not 0.816; CAT=metric-drill; KILLS=round-then-mean-order (mean of rounded values); VERDICT=KEEP
- search_exact_file_membership_and_term_bytes: INTENT=3 matching + 1 unrelated file yields exactly those hits with term bytes in excerpts; CAT=metric-drill; KILLS=membership-pollution/excerpt-drop; VERDICT=MERGE→search-limit-contract

## Helper patterns

- `asgrep_bin` / `run` / `parse_stdout` / `write_fixture`: binary-run harness copy-pasted in all 4 files.
- `write_gold`: in all 4 files (pass1 fixed filename, pass2/3/4 parameterized).
- `f64_at` JSON-pointer float accessor in pass1/3/4; pass2 uses `assert_finite_unit` (pointer + finite-unit check).
- `indexed_project` in pass2/3/4 (pass2 single-file, pass3/4 n-file sharing one term).
- `run_eval_ok` (pass1/4) vs `run_eval_json` (pass3): same shape under different names.
- `run_search_json` identical in pass3/4.
- `assert_usage_envelope` / `assert_operational_envelope` shape-only envelope guards (pass2 only).
- `rank_key` null-as-+inf comparator, `parse_human_row` / `parse_human_summary` table parsers (pass3 only).
- Observation: ~60-100 lines of harness duplicated per file; a shared `tests/cli/common.rs` (or equivalent) would remove 4x drift risk. Noted only; this catalog makes no code changes.

## Counts

- Total: 40 (pass1: 10, pass2: 12, pass3: 10, pass4: 8)
- By CAT: exactness 6, totality 16, metamorphic 10, metric-drill 8, other 0
- By VERDICT: KEEP 16, MERGE 23, DELETE 1
- KEEP anchors: eval_three_query_fractional_aggregates, eval_k_sweep_found_and_recall_match_hand_values, eval_filename_ordered_decoy_forces_rank_two, eval_adversarial_plus_clean_aggregate_uses_unrounded_means, search_limit_sweep_hit_counts_match_hand_values, eval_human_table_rounds_to_three_decimals, eval_human_and_json_metrics_agree, eval_recall_cutoffs_are_monotone, search_rerun_yields_identical_hit_bytes, budget_tokens_cap_names_65536, limit_zero_remaps_to_default, limit_nonnumeric_and_overflow_are_usage, zero_size_windows_are_accepted, call_path_bounds_reject_zero_and_over_max, eval_k_zero_yields_exact_zeros, eval_degenerate_gold_fails_closed
- Merge targets: eval-arithmetic-contract (6), budget-cap-contract (3), search-limit-contract (3), eval-k-sweep-contract (2), limit-parse-contract (2), eval-degenerate-contract (2), zero-window-contract (1), eval-failclosed-contract (1), search-determinism-contract (1), adversarial-rank-contract (1), unrounded-mean-contract (1)
- DELETE: eval_hit_scores_no_lower_than_miss (subsumed by exact endpoint tests)

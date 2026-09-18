# numerical-core catalog (N1–N4)

Source: `tests/core/numerical_pass1.rs` (N1 exactness, 14) → `numerical_pass2.rs` (N2 totality, 12) → `numerical_pass3.rs` (N3 metamorphic, 14) → `numerical_pass4.rs` (N4 drills, 8). Total 48.
Contract targets are proposed per-function merges (`<fn>_contract`); no code changed.

## N1 — exactness (`numerical_pass1.rs`)

- default_weights_symbol_table_exact: INTENT=pins Symbol 8-channel weight literals; CAT=exactness; KILLS=table-literal-mutant; VERDICT=MERGE→default_weights_contract
- default_weights_conceptual_and_flat_intents_exact: INTENT=pins Conceptual literals + Literal/Structural all-ones; CAT=exactness; KILLS=table-literal-mutant; VERDICT=MERGE→default_weights_contract
- weights_for_env_override_clamps_and_filters: INTENT=env spec grammar: override, [0.25,2.0] clamp, all ignore rules, sole env toucher; CAT=exactness; KILLS=parse/clamp-branch-mutant; VERDICT=KEEP
- route_hits_asgrep_ceiling_exact: INTENT=Asgrep ceiling (1/61)*200 normalization incl. clamp of 2c; CAT=exactness; KILLS=ceiling-formula-mutant; VERDICT=MERGE→route_hits_contract
- route_hits_def_caller_ceilings_and_empty_terms: INTENT=Def/Caller ceilings (13/11.5/23), spelling-denom branch, empty-terms zeroing; CAT=exactness; KILLS=denom-branch-mutant; VERDICT=MERGE→route_hits_contract
- route_hits_fixed_channel_ceilings: INTENT=Graph/Anchor/Embed/Pattern/Import fixed ceilings map ceiling→1.0, half→0.5; CAT=exactness; KILLS=ceiling-constant-mutant; VERDICT=MERGE→route_hits_contract
- apply_weighted_rrf_single_and_merge_exact: INTENT=single 1/61, two-channel sum, canonical Def kind + contributor order; CAT=exactness; KILLS=fuse-sum/canonical-mutant; VERDICT=MERGE→apply_weighted_rrf_contract
- apply_weighted_rrf_rank_order_and_zero_drop: INTENT=within-channel rank by score-desc, 0/-1/NaN drop, empty no-op; CAT=exactness; KILLS=rank-order/gate-mutant; VERDICT=MERGE→apply_weighted_rrf_contract (drop half overlaps N2 inf-drop; N2 subsumes set)
- learn_fusion_weights_single_pair_exact: INTENT=softplus loss_before/after vs python oracle, rail convergence to 2.0, tied/empty loss 0; CAT=exactness; KILLS=loss-formula/search-step-mutant; VERDICT=MERGE→learn_fusion_weights_contract
- sensitivity_empty_and_absent_channels: INTENT=empty-table exact zeros, NaN-step sanitize, live gradient<0/curvature>0/stiff ladder; CAT=exactness; KILLS=finite-difference/stiff-flag-mutant; VERDICT=MERGE→sensitivity_contract (empty-zero half overlaps N2 hostile-step loop)
- intent_weight_spec_format_exact: INTENT=6-decimal spec string exact rendering; CAT=other; KILLS=format/order-mutant; VERDICT=KEEP
- ann_threshold_boundary_table: INTENT=default/override/env/garbage threshold + should_use_ann edge + sufficiency truth table; CAT=exactness; KILLS=threshold-comparison-mutant; VERDICT=KEEP
- search_flat_brute_force_cosine_oracle: INTENT=cosine ranking, 0.08 gate, limit/empty, query-normalization identity; CAT=exactness; KILLS=gate/normalize-mutant; VERDICT=MERGE→search_flat_contract
- finish_margins_confidence_and_decisive_boundary: INTENT=margin ladder + tie rule, confidence base+agreement+cap, decisive 10% boundary; CAT=exactness; KILLS=margin/confidence-formula-mutant; VERDICT=MERGE→finish_margin_contract

## N2 — totality (`numerical_pass2.rs`)

- rrf_score_hostile_k_and_extreme_rank: INTENT=NaN/±inf/−1/−61 k propagation, MAX-rank finiteness, negative-k order; CAT=totality; KILLS=guard-insertion-mutant; VERDICT=MERGE→rrf_score_contract
- fuse_rrf_nonfinite_propagation_and_bulk_sum: INTENT=NaN/+inf/neg-k propagation, exact −1.5 cancel, 1000-term + MAX-rank finiteness; CAT=totality; KILLS=sum/scale-mutant; VERDICT=MERGE→fuse_rrf_contract
- weighted_rrf_score_hostile_weights_and_extreme_ranks: INTENT=±inf→1.0, −3/−0.0→0.25, 1e308→2.0 rails, MAX rank, 8-channel clamp sum; CAT=totality; KILLS=clamp-rail-mutant; VERDICT=MERGE→weighted_rrf_score_contract
- apply_weighted_rrf_drops_nonfinite_keeps_extreme_finite: INTENT=±inf/NaN/0/neg drop, subnormal/1e308 kept at 1/61, mixed-member exclusion, hostile weights finite; CAT=totality; KILLS=gate/sanitize-mutant; VERDICT=MERGE→apply_weighted_rrf_contract
- route_hits_hostile_scores_propagate_or_clamp: INTENT=NaN propagates via clamp no-op, ±inf/±1e308 to rails, subnormal→0.0, channel-independence; CAT=totality; KILLS=clamp/NaN-zeroing-mutant; VERDICT=MERGE→route_hits_contract
- learn_fusion_weights_degenerate_relevance_and_initial_weights: INTENT=NaN relevance skips pairs, ±inf relevance gives ln2, hostile initials sanitize to rails; CAT=totality; KILLS=pair-filter/entry-sanitize-mutant; VERDICT=MERGE→learn_fusion_weights_contract
- sensitivity_hostile_step_and_degenerate_examples: INTENT=step sanitize (inf/NaN→0.1, huge→0.5, neg→0.1), empty/NaN-rel all-zero tables; CAT=totality; KILLS=step-sanitize-mutant; VERDICT=MERGE→sensitivity_contract
- finish_hostile_scores_survive_with_zero_margin: INTENT=NaN/±inf retained with margin 0, 1e308−5e-324 margin, confidence bounded, decisive NaN/inf/subnormal; CAT=totality; KILLS=margin-gate/drop-mutant; VERDICT=MERGE→finish_margin_contract
- search_flat_hostile_vectors_gate_or_normalize: INTENT=nonfinite/zero queries gate empty, NaN row dropped, f32 over/underflow dropped, safe extremes normalize, dim-0/huge-limit; CAT=totality; KILLS=zero-fill/guard-mutant; VERDICT=MERGE→search_flat_contract
- chain_decay_hostile_propagates_without_panic: INTENT=NaN/inf/−2/−0.0 decay bit-propagates to hops, total order kept, NaN determinism; CAT=totality; KILLS=hop-formula-mutant; VERDICT=KEEP (sole chain test; expected self-derived via same multiply but kills non-multiplicative formulas)
- score_def_caller_extreme_term_counts: INTENT=100k terms exact 1000003/1000001.5, empty/sub-floor → 0.0 no base; CAT=totality; KILLS=coverage/base-guard-mutant; VERDICT=MERGE→score_def_caller_contract
- ivf_candidate_selection_degenerate_inputs: INTENT=NaN query ≡ zero query, probe clamp, reassign_all fails closed; CAT=totality; KILLS=zero-fill/clamp-mutant; VERDICT=KEEP

## N3 — metamorphic (`numerical_pass3.rs`)

- rrf_score_decreases_in_rank_and_k: INTENT=strict decrease in rank (0..200) and in k; CAT=metamorphic; KILLS=formula-sign-mutant; VERDICT=MERGE→rrf_score_contract
- fuse_rrf_superset_monotone_and_pair_commutative: INTENT=superset strictly raises sum, 2-term swap bit-identical, lexical scale-up monotone; CAT=metamorphic; KILLS=sum/max-swap-mutant; VERDICT=MERGE→fuse_rrf_contract
- weighted_rrf_rank_improvement_never_lowers: INTENT=rank 5→0 raises sum per-channel and over populated background; CAT=metamorphic; KILLS=rank-direction-mutant; VERDICT=MERGE→weighted_rrf_score_contract
- weighted_rrf_adding_absent_channel_never_lowers: INTENT=None→Some strictly raises sum, 8-channel build-up chain rises; CAT=metamorphic; KILLS=absent-term-mutant; VERDICT=MERGE→weighted_rrf_score_contract
- weighted_rrf_channel_pair_swap_invariant: INTENT=(rank,weight) pair swap bit-identical, 8-channel reversal within 1e-14; CAT=metamorphic; KILLS=channel-index-mutant; VERDICT=MERGE→weighted_rrf_score_contract
- apply_weighted_rrf_input_permutation_bit_identical: INTENT=reverse/rotate/halve input → bit-identical fused stream; CAT=metamorphic; KILLS=order-leak-mutant; VERDICT=MERGE→apply_weighted_rrf_contract
- uniform_weight_scaling_preserves_pairwise_order: INTENT=1.0→1.5 scaling preserves all pairwise total_cmp + ~1.5 ratio; CAT=metamorphic; KILLS=weight-application-mutant; VERDICT=MERGE→weighted_rrf_score_contract
- absent_channel_equals_zero_contribution: INTENT=all-absent ≡ 0.0, removal delta equals channel term; CAT=metamorphic; KILLS=empty-sum-mutant; VERDICT=MERGE→weighted_rrf_score_contract (overlaps adding_absent; removal-delta half is the unique bit)
- repeated_calls_bit_identical: INTENT=serial replay bit-identical across all pure fns + learn/sensitivity; CAT=metamorphic; KILLS=nondeterminism-mutant; VERDICT=MERGE→determinism_contract
- multithreaded_scoring_bit_identical: INTENT=8-thread fused/model/sensitivity agree with serial; CAT=metamorphic; KILLS=shared-mutable-state-mutant; VERDICT=MERGE→determinism_contract
- tie_scores_break_identically: INTENT=all-tied inputs break by (file,line), repeat run-to-run, emission sorted; CAT=metamorphic; KILLS=tiebreak-mutant; VERDICT=MERGE→apply_weighted_rrf_contract (overlaps permutation test; sorted-emission half is unique)
- learn_pair_candidate_order_invariant: INTENT=worse-first vs better-first pair → identical model; CAT=metamorphic; KILLS=pair-normalization-mutant; VERDICT=MERGE→learn_fusion_weights_contract
- route_hits_monotone_within_fixed_ceiling_channel: INTENT=Embed routed scores non-decreasing, in [0,1], over-ceiling ties; CAT=metamorphic; KILLS=divide/clamp-mutant; VERDICT=MERGE→route_hits_contract
- def_caller_relevance_ladder_relations: INTENT=def>caller iff coverage>0, equal when unmatched, noise-term identity, exact>substring>unrelated; CAT=metamorphic; KILLS=ladder/scale-mutant; VERDICT=MERGE→score_def_caller_contract

## N4 — scoring drills (`numerical_pass4.rs`)

- breadth_beats_clamped_single_spike: INTENT=two-channel breadth beats 500-raw single spike clamped to 1.0; exact fused + order; CAT=scoring-drill; KILLS=clamp/fuse-mutant; VERDICT=KEEP
- weight_tilt_flips_fused_order: INTENT=same corpus ties at unit weights, lex-heavy → A first, def-heavy → B first, exact tilted scores; CAT=scoring-drill; KILLS=weight-application-mutant; VERDICT=KEEP
- routing_clamp_creates_ties_with_key_breaks: INTENT=13–1300 raws all route to 1.0, key-break ranks, exact fused, reversal bit-identical; CAT=scoring-drill; KILLS=clamp/tiebreak-mutant; VERDICT=KEEP (reversal half overlaps N3 permutation/tie; clamp-creates-ties intent is unique)
- hostile_weights_fuse_on_rails: INTENT=8 channels × 8 sanitize rails fuse to exact rail×1/61 with rail-cohort ranking; CAT=scoring-drill; KILLS=sanitize-rail-mutant; VERDICT=KEEP
- empty_and_zeroed_channels_vanish: INTENT=zeroed/negative hits vanish, lone Embed fuses; empty-terms kills all text channels; CAT=scoring-drill; KILLS=fuse-gate/empty-terms-mutant; VERDICT=KEEP
- real_producer_scores_invert_through_routing: INTENT=genuine score_def raws X>Y invert to Y>X via per-hit ceilings, cemented in fusion; CAT=scoring-drill; KILLS=ceiling-denom-mutant; VERDICT=KEEP
- three_way_merge_canonical_and_breadth_win: INTENT=3-channel merge: canonical Def, contributor order, breadth beats three channel winners; CAT=scoring-drill; KILLS=merge/canonical-mutant; VERDICT=KEEP
- full_pipeline_through_finish_order_scores_margins: INTENT=capstone route→fuse→finish: scores preserved, [b,a,c] order, margins 0, confidences; CAT=scoring-drill; KILLS=finish-rewrite/sort-mutant; VERDICT=KEEP

## Helper patterns

- `n1_hit/n2_hit/n3_hit/n4_hit` — identical SearchHit::span constructors per file (unshared, copy-paste ×4); merge to one shared helper on consolidation.
- `n1_parsed/n1_options/n2_options/n4_options` — ParsedQuery/SearchOptions fixtures; same shape each file.
- `n1_pair_example/n2_pair_example/n3_pair_examples` — same worse-first lexical pair ×3; canonicalize to one.
- `n3_key/n4_key`, `n3_fused_keys`, `n4_pipeline/n4_ranked/n4_files` — fused-row bits + pipeline runners; keep as the shared drill harness.
- `n3_set_rank/n3_set_weight/n3_single_rank/n3_mixed_hits` — ChannelRanks/Weights builders + mixed corpus; keep.
- `n2_base/n2_chain_store` — IndexStore chain fixture; sole chain fixture, keep.

## Counts

- Total tests: 48 (N1 14, N2 12, N3 14, N4 8).
- By CAT: exactness 13, totality 12, metamorphic 14, scoring-drill 8, other 1 (intent_weight_spec_format_exact) = 48.
- By VERDICT: KEEP 13 (weights_for_env, intent_weight_spec, ann_threshold, chain_decay, ivf, 8× N4), MERGE 35, DELETE 0.
- DELETEs: none outright; three oracle-overlaps folded as MERGE notes instead (apply drop N1⊂N2, sensitivity empty N1⊂N2, absent-channel/tie/permutation N3+N4 cluster).
- Proposed contract tests: 14 (default_weights, route_hits, apply_weighted_rrf, learn_fusion_weights, sensitivity, finish_margin, search_flat, rrf_score, fuse_rrf, weighted_rrf_score, score_def_caller, determinism + KEEP standalone weights_for_env/ann/chain/ivf/spec/N4s).

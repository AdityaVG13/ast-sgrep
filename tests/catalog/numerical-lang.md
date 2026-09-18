# Numerical lang test catalog (N1–N4)

Source files: `tests/lang/numerical_pass1.rs` (N1 exactness), `tests/lang/numerical_pass2.rs` (N2 totality),
`tests/lang/numerical_pass3.rs` (N3 metamorphic), `tests/lang/numerical_pass4.rs` (N4 scoring drills).
Scope note: all four files pin `ast-sgrep-embed` math only; keyword/pattern scoring (core) and rerank/neural backends are out of reach (no dependency from this test target).

Calibration applied: KEEP iff (a) sole test of its function, or (b) cross-function/cross-cutting intent that cannot live in one
function contract. All per-function exactness/totality/symmetry tests MERGE into one contract test per function (targets in
## Merge targets). No whole test is a pure weaker duplicate, so no whole-test DELETE; line-level oracle overlaps to delete on merge are listed under ## Merge targets.

## numerical_pass1 (N1 exactness, 14 tests)

- threshold_nextafter_bits_are_exact_all_signs: INTENT=strict threshold bit-boundaries (1.0/0.0/-1.0 neighbors) plus nonfinite-min admits nothing; CAT=exactness; KILLS=threshold-comparison mutant (strict→nonstrict, ulp-off-by-one, nonfinite-min admit-all); VERDICT=MERGE→score_ranker_contract
- top_k_heap_tie_eviction_keeps_ascending_indices: INTENT=heap tie-eviction keeps lowest indices under truncation across input permutations; CAT=exactness; KILLS=tie-eviction-order mutant (keeps wrong indices); VERDICT=MERGE→score_ranker_contract
- top_k_flat_truncates_ragged_tail_and_shapes: INTENT=flat ragged tail ignored (no third row) plus limit/threshold shaping incl strict Some(1.0) drop; CAT=exactness; KILLS=row-count/ragged-tail mutant + limit/threshold-shaping mutant; VERDICT=MERGE→flat_ranker_contract
- top_k_flat_parallel_boundary_agrees_with_sequential: INTENT=parallel flat path (n=64,65) matches sequential cosine+heap path and hand order [5,60,0]; CAT=exactness; KILLS=parallel-fold/reduce-divergence mutant; VERDICT=MERGE→flat_ranker_contract
- rank_chunk_indices_pins_hand_cosine_and_filters: INTENT=chunk hand cosines (1.0 exact, 7/(5√2) approx) plus dim/orthogonal/zero/negative filters and zero-query empty; CAT=exactness; KILLS=chunk-score mutant + dim/threshold-filter-removal mutant; VERDICT=MERGE→chunk_ranker_contract
- rank_chunk_indices_empty_and_limit_shape: INTENT=chunk shaping guards (empty corpus, zero limit, wrong-dim, orthogonal all empty); CAT=exactness; KILLS=limit/empty-guard-removal mutant; VERDICT=MERGE→chunk_ranker_contract
- normalize_overflow_subnormal_and_dyadic_pins: INTENT=normalize overflow→zeros, subnormal-square→zeros, dyadic-exact unit vectors; CAT=exactness; KILLS=finite-guard/zero-guard-removal mutant (inf/inf or 0/0 NaN); VERDICT=MERGE→normalize_contract
- dot_overflow_and_dyadic_lanes: INTENT=dot overflow collapses to 0.0 on scalar and SIMD-width lanes, dyadic sum exact 1.0; CAT=exactness; KILLS=overflow-collapse-guard-removal mutant; VERDICT=MERGE→dot_contract
- cosine_simd_lane_does_not_skip_nonfinite: INTENT=SIMD NaN poisons to 0.0 while scalar skips (24/25 formula) plus SIMD all-finite 1.0; CAT=exactness; KILLS=lane-divergence mutant (SIMD-skip or scalar-poison) + count-skipped-norm mutant; VERDICT=MERGE→cosine_contract
- embed_byte_layout_is_little_endian_exact: INTENT=LE byte layout of 1.0/-2.5/0.0 embeddings, not just roundtrip; CAT=exactness; KILLS=endianness/byte-layout mutant; VERDICT=MERGE→embed_contract
- expand_concepts_single_and_multi_group_strings_are_exact: INTENT=exact expansion strings (combine, auth refresh) plus exact 16-term token set; CAT=exactness; KILLS=expansion-term/sort-order mutant; VERDICT=KEEP
- embed_trigram_path_is_order_sensitive_while_tokens_static: INTENT=token set is order-invariant while trigram path makes "ab cd" vs "cd ab" embeddings differ, both unit-norm; CAT=metamorphic; KILLS=trigram-weight-removal (order-blind-embed) mutant; VERDICT=MERGE→embed_contract
- embed_unit_norm_and_cauchy_schwarz_table: INTENT=unit-norm self-dot (1e-5) and Cauchy-Schwarz cross bound over 8-query table plus exact zero table; CAT=exactness; KILLS=normalize-division-skipped mutant; VERDICT=MERGE→embed_contract
- top_by_keeps_negative_and_orders_signed_zero_by_index: INTENT=sort ranker keeps negatives ordered and breaks ±0.0 ties by index with payload bits pinned; CAT=exactness; KILLS=negative-drop mutant + tie-break/sign-payload mutant; VERDICT=MERGE→score_ranker_contract

## numerical_pass2 (N2 totality, 13 tests)

- dot_zero_subnormal_and_signed_zero_lanes_are_exact: INTENT=dot zero vectors honest zeros, MIN_POSITIVE lanes exact, signed-zero payload bits; CAT=totality; KILLS=zero/subnormal-collapse mutant + sign-payload mutant; VERDICT=MERGE→dot_contract
- dot_extreme_magnitudes_split_finite_from_overflow: INTENT=MAX×1 finite-exact vs MAX×2/MAX² overflow→0.0 plus exact cancellation zeros; CAT=totality; KILLS=overflow-boundary mutant + cancellation-collapse mutant; VERDICT=MERGE→dot_contract
- cosine_skip_all_pairs_and_one_sided_survivor: INTENT=all-skipped pairs→0.0, one-sided survivors still score 1.0, SIMD +inf poisons; CAT=totality; KILLS=skip-guard-removal (nonfinite-poison) mutant; VERDICT=MERGE→cosine_contract
- cosine_extreme_magnitudes_avoid_dot_overflow_collapse: INTENT=f64-lane cosine stays ≈±1.0 at 1e30/MAX where dot collapses to 0.0; CAT=totality; KILLS=f64-accumulation-removal (overflow-collapse) mutant; VERDICT=MERGE→cosine_contract
- normalize_in_place_totality_and_signed_zero_bits: INTENT=in-place empty/overflow/nonfinite lanes plus +0.0 fill bits for ±0.0 inputs; CAT=totality; KILLS=in-place-divergence mutant + zero-fill-sign mutant; VERDICT=MERGE→normalize_contract
- rankers_all_nonfinite_empty_and_limit_edges: INTENT=both rankers return honest empties for all-nonfinite corpora, heap empty/k=0/k>len edges; CAT=totality; KILLS=nonfinite-admit mutant + heap-limit-edge mutant; VERDICT=MERGE→score_ranker_contract
- rankers_drop_negative_infinity_and_keep_negative_order: INTENT=both rankers drop -inf like NaN/+inf and keep negatives below positives; CAT=totality; KILLS=-inf-admit mutant + negative-order mutant; VERDICT=MERGE→score_ranker_contract
- flat_degenerate_shapes_empty_and_limit_edges: INTENT=flat dim=0/short-row/empty-query empties, k>len unpadded, zero rows kept then threshold-dropped; CAT=totality; KILLS=shape-guard (div-by-zero/panic) mutant + k>len-pad mutant; VERDICT=MERGE→flat_ranker_contract
- flat_nonfinite_corpus_and_query_totality: INTENT=all-NaN rows→0.0 kept/dropped by threshold arm, inf query partial survival, inf row sorts after 1.0; CAT=totality; KILLS=nonfinite-row/query-collapse mutant; VERDICT=MERGE→flat_ranker_contract
- flat_parallel_path_with_hostile_rows_keeps_hand_order: INTENT=parallel path (n=64) with odd/NaN zero rows keeps exact 40-slot hand order, all finite; CAT=totality; KILLS=parallel-hostile-order mutant; VERDICT=MERGE→flat_ranker_contract
- chunk_ranker_nonfinite_query_and_rows_fail_closed: INTENT=NaN query empties, NaN/inf rows drop beside exact 1.0 survivor, empty-dim empty, usize::MAX limit unpadded; CAT=totality; KILLS=nonfinite-fail-open mutant + extreme-limit mutant; VERDICT=MERGE→chunk_ranker_contract
- embed_text_degenerate_inputs_are_total: INTENT=whitespace/punctuation exact zeros, non-ASCII and 500× inputs shaped/finite/deterministic; CAT=totality; KILLS=degenerate-input panic-or-NaN mutant + nondeterminism mutant; VERDICT=MERGE→embed_contract
- similarity_delegation_and_byte_edges: INTENT=similarity delegates to dot on degenerate lanes, empty bytes empty, aligned NaN payload bits preserved; CAT=totality; KILLS=delegation-divergence mutant + byte-edge mutant; VERDICT=MERGE→embed_contract

## numerical_pass3 (N3 metamorphic, 14 tests)

- dot_symmetry_bit_exact_scalar_lane: INTENT=dot(a,b)==dot(b,a) bit-exact on scalar lane incl hostile/empty/mismatched pairs; CAT=metamorphic; KILLS=argument-order (non-commutative-fold) mutant; VERDICT=MERGE→dot_contract
- dot_symmetry_simd_lane_approx: INTENT=dot swap-agreement to 1e-5 on 64/70-dim SIMD lane; CAT=metamorphic; KILLS=SIMD argument-order mutant; VERDICT=MERGE→dot_contract
- cosine_symmetry_bit_exact_scalar_lane: INTENT=cosine(a,b)==cosine(b,a) bit-exact on scalar lane incl asymmetric nonfinite pairs; CAT=metamorphic; KILLS=asymmetric-skip mutant; VERDICT=MERGE→cosine_contract
- cosine_symmetry_simd_lane_approx: INTENT=cosine swap-agreement to 1e-6 on 64/70/128-dim SIMD lane; CAT=metamorphic; KILLS=SIMD normalization-order mutant; VERDICT=MERGE→cosine_contract
- self_similarity_is_maximal: INTENT=cosine self≈1.0 ceiling with cross≤self, unit dot self≈1.0, zero vector collapses to 0.0; CAT=metamorphic; KILLS=ceiling-violation (cross-beats-self) mutant; VERDICT=MERGE→cosine_contract
- normalize_idempotent_twice_equals_once: INTENT=second normalize drifts <1e-6 on general vectors, exactly idempotent on degenerate lanes; CAT=metamorphic; KILLS=second-pass-divergence mutant; VERDICT=MERGE→normalize_contract
- normalize_in_place_agrees_with_out_of_place: INTENT=in-place and out-of-place normalize agree bit-exactly on general and hostile inputs; CAT=metamorphic; KILLS=API-divergence mutant; VERDICT=MERGE→normalize_contract
- cosine_positive_rescale_preserves_scores_and_ranking: INTENT=positive row/query rescale leaves cosine scores ≈unchanged and flat ranking order identical; CAT=metamorphic; KILLS=magnitude-leak (non-invariance) mutant; VERDICT=MERGE→cosine_contract
- dot_positive_rescale_preserves_order_negative_reverses: INTENT=positive query rescale preserves dot order bit-exactly, negative rescale reverses it; CAT=metamorphic; KILLS=rescale-sign/order mutant; VERDICT=MERGE→dot_contract
- rankers_invariant_under_input_permutation: INTENT=score rankers return identical output under input permutation incl ties/drops, both threshold arms; CAT=metamorphic; KILLS=input-order-leak mutant; VERDICT=MERGE→score_ranker_contract
- flat_ranking_invariant_under_row_permutation: INTENT=flat ranking maps back through row permutation with bit-identical scores on distinct-angle rows; CAT=metamorphic; KILLS=row-index/score-pairing mutant; VERDICT=MERGE→flat_ranker_contract
- top_k_nesting_prefix_property: INTENT=top-k is a prefix operation (top-2==top-5[..2]) on sort/heap/flat rankers with distinct and tied scores; CAT=metamorphic; KILLS=truncation-reorder mutant; VERDICT=MERGE→score_ranker_contract
- threshold_only_filters_never_reorders: INTENT=thresholded output equals unthresholded output filtered to s>min with identical relative order; CAT=metamorphic; KILLS=threshold-reorder mutant; VERDICT=MERGE→score_ranker_contract
- determinism_across_threads: INTENT=8 threads reproduce cosine/dot/normalize/rank/embed workload bit-exactly incl n≥64 rayon flat path; CAT=metamorphic; KILLS=nondeterminism/rayon-race mutant; VERDICT=KEEP

## numerical_pass4 (N4 scoring drills, 9 tests)

- cosine_pipeline_ranks_by_angle_not_magnitude: INTENT=flat cosine pipeline ranks [1,0,2,3] by angle with dot control proving the magnitude confound is real; CAT=scoring-drill; KILLS=dot-for-cosine (magnitude-confound) mutant; VERDICT=MERGE→flat_ranker_contract
- near_tie_ladder_orders_by_epsilon_with_index_break: INTENT=epsilon ladder [4,1,2,0,3] resolves 1.5e-4 gaps with bit-identical duplicate tie-break plus f64 rung checks; CAT=scoring-drill; KILLS=epsilon-resolution/tie mutant; VERDICT=MERGE→flat_ranker_contract
- hostile_mixed_corpus_keeps_clean_order_zeros_sort_by_index: INTENT=NaN/inf/zero/orthogonal rows score exact 0.0 and sort by index after clean 1.0 ties; CAT=scoring-drill; KILLS=hostile-fail-open/order mutant; VERDICT=MERGE→flat_ranker_contract
- flat_threshold_boundary_drops_equal_score_keeps_above: INTENT=hand 0.6 score drops at Some(0.6) while 0.7071/1.0 survive, unthresholded control proves strictness; CAT=scoring-drill; KILLS=strictness-at-0.6 mutant; VERDICT=MERGE→flat_ranker_contract
- chunk_ranker_threshold_straddle_keeps_above_drops_below: INTENT=0.0797 dropped/0.0830 kept straddling MIN_SIMILARITY with f64 margin guards; CAT=scoring-drill; KILLS=MIN_SIMILARITY-cut mutant; VERDICT=MERGE→chunk_ranker_contract
- duplicate_corpus_truncation_keeps_lowest_indices: INTENT=8 identical rows truncate to [0..5] with bit-equal 1/√2 scores; CAT=scoring-drill; KILLS=flat-tie-truncation mutant; VERDICT=MERGE→flat_ranker_contract
- zero_query_flat_pipeline_orders_by_index_threshold_empties: INTENT=zero query yields all-exact-0.0 index order, Some(0.0) drops everything; CAT=scoring-drill; KILLS=zero-query-collapse mutant; VERDICT=MERGE→flat_ranker_contract
- text_embed_dot_rank_self_match_wins_deterministically: INTENT=text→embed→dot→rank self-match wins with separation and bit-exact rerun determinism; CAT=scoring-drill; KILLS=TAUTOLOGY-RISK+exact-order-[1,2,0,3]-asserts-observed-blake3-hashes-not-hand-values (self-first+separation+determinism arms still kill ranking/nondeterminism mutants; weaken order line to self-first); VERDICT=KEEP
- normalize_dot_rank_hostile_pipeline_exact_order: INTENT=normalize→dot→rank over hostile corpus (NaN row→0.8) yields exact [0,3,1,2] with hand margins; CAT=scoring-drill; KILLS=pipeline-composition (wrong-normalized-score-reorder) mutant; VERDICT=KEEP

## Merge targets

One contract test per function; MERGE verdicts fold in as clauses (no whole-test DELETE: overlaps below are line-level, delete the weaker lines on merge):

- dot_contract ← dot_overflow, dot_zero_subnormal, dot_extreme, dot_sym_scalar, dot_sym_simd, dot_rescale (6 clauses; drop: none — dot-control line in cosine_extreme stays as cross-ref, not duplicated here)
- cosine_contract ← cosine_simd_lane, cosine_skip, cosine_extreme, cos_sym_scalar, cos_sym_simd, self_maximal, cosine_rescale (7 clauses; drop on merge: self_maximal zero-vector arms, weaker vs N2 dot_zero/cosine_skip pins)
- normalize_contract ← normalize_overflow, normalize_inplace, normalize_idempotent, normalize_parity (4 clauses; drop on merge: idempotent exact-lane arms, weaker vs N1/N2 normalize pins)
- score_ranker_contract ← threshold_nextafter, heap_tie, top_by_neg, rankers_nonfinite_edges, rankers_neginf, rankers_permutation, nesting (sort/heap arms), threshold_filters (8 clauses; drop on merge: heap-vs-sort differential lines in heap_tie/rankers_permutation/threshold_filters, weaker vs L1/L2-owned agreement)
- flat_ranker_contract ← flat_ragged, flat_parallel, flat_degenerate, flat_nonfinite, flat_parallel_hostile, flat_row_permutation, nesting (flat arm), angle_drill, ladder_drill, hostile_drill, flat_threshold_drill, duplicate_drill, zero_query_drill (13 clauses)
- chunk_ranker_contract ← chunk_hand, chunk_empty, chunk_nonfinite, straddle_drill (4 clauses)
- embed_contract ← byte_layout, trigram, unit_norm, embed_degenerate, similarity_delegation (5 clauses)
- KEEP standalone: expand_concepts (sole test of its function); determinism_across_threads, text_embed_dot_rank, normalize_dot_rank_hostile (cross-function/cross-cutting)

## Helper patterns

- `approx(a, b)` (1e-6 band): duplicated identically in pass1/2/3; pass4 inlines literal `< 1e-6` / `< 1e-5` bands instead — merge into one shared tolerance helper.
- `chunk_row(emb)` tuple builder `("f.rs",0,1,"sym","ex",emb)`: duplicated identically in pass1/2/4 — merge into one shared fixture.
- `indices(ranked)`: pass4-only helper extracting index order for exact-order assertions.
- `lcg_vec(seed, len, scale)` deterministic xorshift-free LCG with fixed per-test seeds: pass3-only; each test uses its own seed constant for reproducibility.
- Tolerance bands: BIT-EXACT (`assert_eq!`/`to_bits`) for orderings/indices/strings/guard outcomes; 1e-6 for cosine/hand-formula scores; 1e-5 for 256-lane embedding self-dots; each file carries an inline tolerance table in its header.

## Counts

- Tests per file: pass1=14, pass2=13, pass3=14, pass4=9; total=50.
- By CAT: exactness=13, totality=13, metamorphic=15, scoring-drill=9, other=0.
- By VERDICT: KEEP=4, MERGE=46, DELETE=0.
- KEEP list (4): expand_concepts_single_and_multi_group_strings_are_exact, determinism_across_threads, text_embed_dot_rank_self_match_wins_deterministically, normalize_dot_rank_hostile_pipeline_exact_order.
- TAUTOLOGY-RISK (1): text_embed_dot_rank order line (observed-hash snapshot; weaken to self-first + determinism, keep the test).
- BEHAVIOR-ONLY (0): every kept/merged test names a killing mutant class.
- Oracle overlaps (all line-level, deleted on merge, no whole-test DELETE): heap-vs-sort differentials (heap_tie, rankers_permutation, threshold_filters) vs L1/L2; self_maximal zero arms vs N2; idempotent exact lanes vs N1/N2.

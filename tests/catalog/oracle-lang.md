# oracle-lang catalog (oracle-foundry Mission 2, lang surface)

Source: `tests/lang/oracle_foundry_pass1.rs` (L1 hand oracles), `pass2.rs` (L2 mutation-discriminating),
`pass3.rs` (L3 metamorphic/differential/adversarial), `pass4.rs` (L4 e2e pipelines).
Convention: expectations hand-computed; errors assert discriminants (`is_err`/`None`/emptiness), never messages.

## Pass 1 — L1 hand oracles

- dot_similarity_matches_hand_products: INTENT=dot equals hand products, degenerate inputs (empty/mismatch/nonfinite) collapse to 0.0; CAT=similarity; KILLS=degenerate-guard-removal (len-mismatch/empty/Inf/NaN→0); VERDICT=KEEP
- cosine_similarity_matches_hand_angles: INTENT=cosine equals hand angles (orthogonal 0, identical 1), degenerate inputs collapse to 0.0; CAT=similarity; KILLS=zero-norm/degenerate-NaN guard-removal; VERDICT=KEEP
- normalize_vec_matches_hand_norms: INTENT=normalize matches hand norms ([3,4]→[0.6,0.8]), nonfinite zeroed, zero/empty fixed; CAT=similarity; KILLS=nonfinite-zeroing-removal + zero-norm-guard-drop; VERDICT=KEEP (zero-vector line duplicated in pass2/3 — dedup note only)
- top_by_similarity_orders_truncates_and_drops_nan: INTENT=rank is score-desc/index-asc, truncates to limit, drops NaN, threshold exclusive; CAT=rank; KILLS=sort/tie-break/limit/threshold-flip + NaN-filter-removal; VERDICT=KEEP (MIN_SIMILARITY-exclusive line duplicated in pass2 threshold test — dedup note only)
- embed_bytes_roundtrip_is_bit_exact: INTENT=vec→bytes→vec is bit-exact incl nonfinite, empty Ok, ragged lengths Err; CAT=other; KILLS=width/endianness mutant + ragged-length-accept; VERDICT=KEEP
- split_ident_matches_hand_splits: INTENT=ident splitting matches hand table (camel/snake/acronym/dash, empty/underscore fallback); CAT=tokenize; KILLS=split-boundary/case-fold mutants; VERDICT=KEEP
- tokenize_matches_hand_sets: INTENT=tokenize matches hand token sets incl min-length filtering and empty; CAT=tokenize; KILLS=min-length-filter/sort mutants; VERDICT=KEEP
- pattern_gates_treat_empty_and_literal_as_native: INTENT=empty and plain-literal patterns stay in-process, empty pattern is Ok-empty; CAT=pattern; KILLS=literal-fallback-mistable + empty-pattern-Err; VERDICT=KEEP (thinnest P1 test, but empty-pattern-Ok and literal-gate lines have no exact duplicate)

## Pass 2 — L2 mutation-discriminating

- threshold_is_strict_nextafter_not_gte: INTENT=threshold is strict `>` at 0.0/negative/MIN_SIMILARITY incl denormal step, Inf filtered; CAT=rank; KILLS=`>`→`>=`-flip + denormal-step-drop + Inf-filter-removal; VERDICT=KEEP
- heap_and_sort_rankers_agree_on_adversarial_corpus: INTENT=heap and sort rankers agree bit-exactly on NaN/Inf/tie/negative corpus; CAT=rank; KILLS=heap-vs-sort divergence (NaN/Inf/tie-break/threshold mismatch); VERDICT=KEEP
- top_k_flat_guards_division_and_shape: INTENT=flat top-k returns empty on dim-0/shape-mismatch/empty/limit-0, ranks valid rows; CAT=rank; KILLS=`checked_div`→`/`-panic + shape-guard-removal; VERDICT=KEEP
- normalize_zero_and_nonfinite_have_no_nan: INTENT=zero/all-nonfinite normalize to zero vector (never NaN), in-place agrees; CAT=similarity; KILLS=inf/inf-NaN + 0/0-NaN + inplace-divergence; VERDICT=KEEP
- dot_simd_and_scalar_paths_agree: INTENT=SIMD (≥64 lanes) and scalar dot paths give exact hand sums incl 63/64 boundary; CAT=similarity; KILLS=SIMD-vs-scalar lane-boundary divergence; VERDICT=KEEP
- cosine_skips_nonfinite_pairs_and_normalizes: INTENT=cosine skips NaN pairs, guards zero-norm, returns normalized (parallel→1.0); CAT=similarity; KILLS=NaN-poisoning-fold + unnormalized-dot; VERDICT=KEEP
- independent_dot_oracle_agrees: INTENT=dot matches dyadic-exact hand totals on signed/fractional vectors; CAT=similarity; KILLS=sign/zip-truncation mutants via hand totals (f64-fold half is BEHAVIOR-ONLY — mirrors impl zip; TAUTOLOGY-RISK on that half alone, saved by hand totals); VERDICT=KEEP
- split_ident_camel_and_acronym_tables: INTENT=camel/acronym/digit/separator splits match hand table (HTTPStatusCode, a1B2, single cap); CAT=tokenize; KILLS=per-capital-split + digit-edge-flip + separator-swap; VERDICT=KEEP
- tokenize_dedups_and_sorts: INTENT=tokenize dedups repeats and sorts output; CAT=tokenize; KILLS=dedup-drop + sort-drop; VERDICT=KEEP (zebra/apple line duplicated in pass3 order-invariance — dedup note only)
- expand_concepts_trigger_precision: INTENT=combine fires conjunction group (not rrf/fusion), eviction fires prune/cache/stale, unknown passes through; CAT=tokenize; KILLS=trigger-group-drop/merge; VERDICT=KEEP
- keyword_literal_roots_match_exact_table: INTENT=keyword-literal admission matches exact 9-in/7-out table incl trim/case edges; CAT=pattern; KILLS=matches-arm add/drop + trim-removal + case-fold; VERDICT=KEEP
- match_pattern_literal_and_trivia_edges: INTENT=literal present hits, ws-only/BOM/absent behave (Ok-empty, never Err); CAT=pattern; KILLS=literal-lane-removal + trim/BOM-strip-removal + error-on-absent; VERDICT=KEEP

## Pass 3 — L3 metamorphic / differential / adversarial

- similarity_symmetric_and_cosine_bounded: INTENT=dot/cosine symmetric, cosine bounded ±1 with hand values (-8, 0.96, ±1); CAT=similarity; KILLS=asymmetric-accumulation + unnormalized-dot + unbounded-cosine; VERDICT=KEEP
- normalize_idempotent_and_inplace_agrees: INTENT=normalize is idempotent fixed-point with unit norm, in-place bit-agrees, degenerate fixed exact; CAT=similarity; KILLS=non-idempotent-normalize + inplace-divergence + degenerate-fixed-point; VERDICT=KEEP
- ranking_invariant_under_input_permutation_and_ties: INTENT=ranking invariant under input permutation, all-tie→index-asc, over-limit unpadded; CAT=rank; KILLS=input-order-dependence + tie-break-flip + limit-padding; VERDICT=KEEP
- tokenize_order_invariant_and_deterministic: INTENT=tokenize invariant under word order/layout, camel hand-set, repeat-deterministic; CAT=tokenize; KILLS=order-dependence + layout-sensitivity + camel-split (repeat-call line BEHAVIOR-ONLY); VERDICT=KEEP
- expand_concepts_superset_and_empty: INTENT=expansion only adds terms (query tokens survive), empty→" ", throttle hand trigger, deterministic; CAT=tokenize; KILLS=term-drop + empty-format-change (determinism line BEHAVIOR-ONLY); VERDICT=KEEP
- embed_text_deterministic_zero_for_empty: INTENT=empty embeds to exact zero vector, real queries deterministic/finite/unit-norm, provider agrees with free fn; CAT=similarity; KILLS=empty-nonzero + nondeterminism + nonfinite + non-unit-norm + provider/free-divergence; VERDICT=KEEP
- language_parse_roundtrip_and_extension_table: INTENT=parse/from_extension/detect agree per hand alias table, shebang sniffing works, unknown stays None; CAT=other; KILLS=alias-table mistable (ext-vs-name lanes) + detect/parse divergence + sniffing-drop; VERDICT=KEEP
- is_pattern_ident_admission_table: INTENT=ident admission matches 6-in/8-out table incl unicode/digit/punct edges; CAT=pattern; KILLS=admission-table add/drop; VERDICT=KEEP
- classify_native_trim_and_modifier_invariance: INTENT=classify invariant under trim and decl modifiers, rejects case/garbage/empty; CAT=pattern; KILLS=trim-sensitivity + modifier-strip-drop + permissive-head; VERDICT=KEEP
- signature_serve_consistency: INTENT=signature-serve table exact (ident/decl serve, kind-only never, keywords escape) + prefilter/structural keys byte-exact; CAT=pattern; KILLS=serve-table flip + prefilter-literal mistable + structural-key drift; VERDICT=KEEP
- literal_differential_and_match_monotonicity: INTENT=unified/direct literal lanes agree, matches grow monotonically, unicode ident total+deterministic; CAT=pattern; KILLS=lane-divergence + non-monotonic-match + unicode-Err/empty; VERDICT=KEEP
- connector_carve_and_fallback_class: INTENT=bare connectors match nothing, $-less class never falls back (9-pattern table); CAT=pattern; KILLS=connector-match-breach + $-less-fallback-breach; VERDICT=KEEP
- adversarial_vectors_and_deep_extraction: INTENT=all-NaN@SIMD-len→0, NaN-query rows→0.0, 100k-dim exact-class, 300-deep truncates loud-Ok, empty Ok; CAT=other; KILLS=NaN-poisoning@SIMD-len + huge-dim-overflow + silent-depth-breach + empty-Err; VERDICT=KEEP (dual-intent: vectors + extraction; would SPLIT if that verdict existed — kept as one hostile-input-totality suite)

## Pass 4 — L4 end-to-end pipelines

- e2e_rust_decl_pattern_to_ranked_excerpts: INTENT=decl pattern→2 hits→embed→score→rank agrees with independent f64-fold order; CAT=e2e; KILLS=pipeline-wiring (match→embed→rank) + ranker-vs-fold divergence (fold half BEHAVIOR-ONLY differential); VERDICT=KEEP
- e2e_extract_symbols_drive_prefilter_and_match: INTENT=extracted symbol (name/lines) drives prefilter literal and is span-covered by match hit; CAT=e2e; KILLS=extract/match span-divergence + prefilter-mistable; VERDICT=KEEP
- e2e_multilanguage_detect_parse_match: INTENT=detect→parse→match pipeline works on rs/py/go fixtures; CAT=e2e; KILLS=per-language detect/parse/match wiring; VERDICT=KEEP
- e2e_symbol_names_scored_and_ranked: INTENT=exact-name query self-scores 1.0 and ranks first over sibling symbol; CAT=e2e; KILLS=scoring/rank-inversion; VERDICT=KEEP (shares alpha/beta scoring setup with threshold test — distinct asserts, no merge)
- e2e_threshold_shapes_ranked_pipeline: INTENT=limit-1 keeps head, head-score threshold keeps nothing, below-head partitions kept/dropped exactly; CAT=e2e; KILLS=limit-head + exclusive-threshold + partition leak/loss (partition loop re-states `>` rule — weakest, near-BEHAVIOR-ONLY half); VERDICT=KEEP
- e2e_pipeline_deterministic_under_repetition: INTENT=extract→match→score→rank bit-identical across 3 runs, nonempty; CAT=e2e; KILLS=BEHAVIOR-ONLY (no hand oracle — pure repetition equality; only full-pipeline determinism proof, hence kept); VERDICT=KEEP
- e2e_empty_source_fail_closed: INTENT=empty source parses/matches/ranks to honest empty on rs+py, no truncation flag; CAT=e2e; KILLS=empty-panic + ghost-rows + truncation-flag-breach; VERDICT=KEEP
- e2e_garbage_input_fail_closed: INTENT=garbage source/pattern stay total-Ok, silent, never match-everything; CAT=e2e; KILLS=garbage-panic/Err + match-everything-breach; VERDICT=KEEP
- e2e_captures_agree_with_extracted_symbols: INTENT=$NAME captures equal hand set {alpha,beta} and equal registry extraction; CAT=e2e; KILLS=capture-drop + extract/match name-divergence; VERDICT=KEEP
- e2e_call_site_line_agrees_with_literal_match: INTENT=extracted call row (caller/callee/line 2) agrees with literal match line; CAT=e2e; KILLS=call-extract wiring + line-divergence; VERDICT=KEEP

## Helper patterns

- `approx` (pass1/3): 1e-6 float compare for hand norms/angles; pass4 uses inline 1e-5 for embedding self-sim.
- `embedder()` + `fold_rank` (pass4): fresh `SemanticLocalEmbedding` per test; independent f64-fold + explicit hand rank rule as differential oracle.
- Hand-table loops (pass2 keyword roots, pass3 ident/classify/fallback tables): arrays of in/out cases with `{word:?}` messages.
- Discriminant errors everywhere: `is_err`/`is_empty`/`None`/`is_ok`, never message text.
- Corpus fixtures: hand-scored `(idx, score)` vecs (rank), tiny Rust sources (`fn foo/greet/alpha`), NaN/Inf adversarial vectors, 300-deep paren source.

## Counts

- Tests: 43 total (pass1: 8, pass2: 12, pass3: 13, pass4: 10).
- CAT: similarity 12, rank 6, tokenize 8, pattern 9, e2e 10, other 3.
- VERDICT: KEEP 43, MERGE 0, DELETE 0.
- One-assert scan: 0 one-assert tests found (minimum is 3+ asserts) — MERGE-by-default rule fires on nothing.
- Dedup notes (not verdicts): zero-normalize line ×3 (P1/P2/P3), MIN_SIMILARITY-exclusive ×2 (P1/P2), zebra/apple ×2 (P2/P3), alpha/beta scoring setup ×2 (P4 symbol/threshold).
- Weakest halves (BEHAVIOR-ONLY, kept for unique coverage): P2 f64-fold mirror, P4 fold agreement, P4 partition loop, P4 repetition determinism.

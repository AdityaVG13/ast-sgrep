# oracle-core catalog (pass1–pass4)

Source: `tests/core/oracle_foundry_pass{1,2,3,4}.rs`. 46 tests total.

## pass1 — L1 unit contracts

- output_limit_clamp_matches_hand_table: INTENT=output clamp honors None/0→default, default floor 1, ceiling 1000; CAT=limit-clamp; KILLS=ceiling-drop/floor-drop mutants; VERDICT=KEEP
- agent_limit_clamp_uses_stricter_ceiling: INTENT=agent clamp caps at 100 with same default/floor rules; CAT=limit-clamp; KILLS=ceiling-swap (100↔1000) mutants; VERDICT=KEEP
- query_length_counts_chars_not_bytes: INTENT=query len validated in chars incl pure-multibyte é at MAX; CAT=query-parse; KILLS=bytes-for-chars/`>`→`>=` mutants; VERDICT=KEEP
- fts_term_escaping_matches_hand_table: INTENT=FTS term quoting with doubled-quote escape incl empty/bare-quote; CAT=query-parse; KILLS=escape-drop mutants; VERDICT=KEEP
- fts_query_joins_terms_with_or: INTENT=FTS query OR-joins escaped terms, empty→""; CAT=query-parse; KILLS=join-separator/empty-default mutants; VERDICT=MERGE→fts_term_escaping_matches_hand_table
- schema_mismatch_roundtrips_and_rejects_garbage: INTENT=schema-mismatch message round-trips (7,5), garbage/empty/non-numeric→None; CAT=other; KILLS=format/parse mutants; VERDICT=KEEP
- mmap_readonly_returns_exact_bytes: INTENT=mmap returns exact bytes incl NUL/multibyte, stable on remap; CAT=other; KILLS=truncation/encoding mutants; VERDICT=KEEP

## pass2 — L2 mutation discriminants

- rrf_score_distinguishes_adjacent_ranks: INTENT=RRF 1/(k+r+1) exact values plus strict decrease over ranks; CAT=scoring; KILLS=`+1`-drop/rank-ignored/k-ignored mutants; VERDICT=KEEP
- fuse_rrf_sums_terms_and_zeroes_on_empty: INTENT=fusion sums RRF terms, empty→0.0; CAT=fusion; KILLS=sum→max/first-only/empty-nonzero mutants; VERDICT=KEEP
- score_lexical_rrf_applies_scale: INTENT=lexical RRF applies 200x scale (200/61), empty→0; CAT=scoring; KILLS=scale-drop mutants; VERDICT=KEEP
- score_symbol_exact_substring_absent_ladder: INTENT=symbol ladder exact=5/substring=2/absent=0 with case-fold and 2-char floor; CAT=scoring; KILLS=branch-swap/floor-flip/one-sided-fold mutants; VERDICT=KEEP
- best_and_coverage_scores_split_max_vs_sum: INTENT=best=max vs coverage=sum split (2.0 vs 4.0) plus empty→0; CAT=scoring; KILLS=max↔sum-swap mutants; VERDICT=KEEP
- weighted_rrf_ignores_absent_channels_and_clamps_weights: INTENT=weighted RRF skips None, sums channels, clamps huge→2x and NaN→1x; CAT=fusion; KILLS=None-as-rank-0/first-only/clamp-removal/NaN-passthrough mutants; VERDICT=KEEP
- independent_rrf_rational_oracle_agrees: INTENT=cross-multiplication identity score*(k+r+1)==1 over ranks×k; CAT=scoring; KILLS=`+2`/rank-ignored/const-return mutants; VERDICT=KEEP
- parsed_query_mode_table_kills_prefix_mutants: INTENT=prefix→mode/target/terms table incl Word-vs-Literal case rules and raw echo; CAT=query-parse; KILLS=prefix-misroute/case-swap/trim mutants; VERDICT=KEEP
- path_scope_splits_and_refuses_loudly: INTENT=`in:` scope split with loud refuse (bare/dup/escape/abs), quote-blindness, glob suffix; CAT=query-parse; KILLS=silent-drop fail-open/escape-accept/glob-passthrough mutants; VERDICT=KEEP
- limit_constants_pin_hand_values: INTENT=pins 5 byte/line/char const values (65536/100/1024/4096/1M); CAT=limit-clamp; KILLS=arithmetic/ceiling-drift mutants; VERDICT=KEEP
- clamp_ceilings_are_not_interchangeable: INTENT=agent/output ceilings differ (101 and 1000 split cases) plus `>0` survivor witness; CAT=limit-clamp; KILLS=ceiling-swap/zero-floor mutants; VERDICT=KEEP
- query_length_boundary_is_exclusive_over_chars: INTENT=exactly-MAX ok, MAX+1 err, mixed aé-char counting; CAT=query-parse; KILLS=`>`→`>=`/bytes-for-chars mutants; VERDICT=KEEP
- schema_mismatch_field_order_kills_swap: INTENT=(disk,supported) order incl inverted-magnitude (5,7); CAT=other; KILLS=tuple-swap/magnitude-order mutants; VERDICT=KEEP

## pass3 — L3 stage oracles

- stdin_bounded_line_table_crlf_and_edges: INTENT=bounded stdin line table: CRLF strip, at-limit/over boundary, TooLong drain, zero-limit; CAT=other; KILLS=boundary-flip/CRLF-strip/drain mutants; VERDICT=KEEP
- capped_read_boundary_and_fail_closed: INTENT=capped read accepts at-cap, refuses over-cap/binary/dir; CAT=other; KILLS=cap-flip/lossy-decode/dir-accept mutants; VERDICT=KEEP
- indexed_rel_path_accepts_and_refuses: INTENT=rel-path accept table, refuse table (abs/../NUL/empty); CAT=other; KILLS=traversal-accept mutants; VERDICT=KEEP
- split_lines_eol_numbering_and_roundtrip: INTENT=EOL detection, 1-based numbering, CRLF strip, LF round-trip, `text::` differential; CAT=other; KILLS=CRLF/numbering/strip mutants; VERDICT=KEEP
- excerpt_bound_is_idempotent_and_marked: INTENT=excerpt cap with `\n…` marker, at-cap passthrough, idempotence, char-boundary walk-back; CAT=other; KILLS=marker-drop/cap-flip/mid-char-cut mutants; VERDICT=KEEP
- wire_hits_distrust_signal_margin_contributors: INTENT=wire decode re-derives signal/contributors, sanitizes margin, bounds excerpt, serialize fixpoint; CAT=other; KILLS=trust-wire-field/unbounded-excerpt mutants; VERDICT=KEEP
- signal_ladder_confidence_and_dedup_merge: INTENT=signal ladder + confidence bases/bonus-cap + order-free dedup merge + why/format rendering; CAT=fusion; KILLS=ladder-swap/confidence-const/merge-key/order-dependence/format mutants; VERDICT=KEEP
- planner_decisive_followups_and_pool_floor: INTENT=decisive 10% boundary, follow-up drill-down table, suggestion chain, lexical pool floor; CAT=other; KILLS=ratio/order/quote/floor mutants; VERDICT=KEEP
- finish_response_gates_limits_and_filters: INTENT=finish limits/order/bytes, count-only, dedup flag, filter compat, scope differential, def promotion; CAT=other; KILLS=limit-off-by-one/order/filter-fail-open/promotion-drop mutants; VERDICT=KEEP
- resolution_tiers_upgrade_and_candidates: INTENT=resolution rank ladder, precision set, upgrade algebra, candidate table, describe/qualified; CAT=other; KILLS=rank-swap/precision/upgrade/cap mutants; VERDICT=KEEP
- lexicon_support_gate_expand_stable: INTENT=subtoken/prose splitting, MIN_SUPPORT gate, PPMI=ln2, order, expand stability, template gate, per-term cap; CAT=other; KILLS=support-gate/PPMI/order/expand/truncation mutants; VERDICT=KEEP
- scip_degrades_never_fails_and_ident_table: INTENT=SCIP hostile-input degrade-never-fail, path/ident/role/line tables; CAT=other; KILLS=fail-open/path/ident/role/line mutants; VERDICT=KEEP
- file_filters_skip_hidden_foreign_and_negate: INTENT=skip-dir/file tables, case-insensitive extensions, ignore+negation, matcher differential; CAT=other; KILLS=skip-set/extension-case/negation mutants; VERDICT=KEEP
- intent_classify_weights_routing_and_scoring: INTENT=classify table, pinned weight tables, routing normalize/clamp/contraction, raw-vs-normalized agreement; CAT=scoring; KILLS=misroute/weight-drift/route-clamp/path-split mutants; VERDICT=KEEP

## pass4 — L4 end-to-end

- pipeline_defs_and_callers_cite_indexed_symbols: INTENT=build→defs/callers round-trip cite indexed symbol and callee with positive scores; CAT=e2e; KILLS=BEHAVIOR-ONLY; VERDICT=KEEP
- pipeline_hybrid_merges_evidence_with_signal_provenance: INTENT=hybrid includes Def hit; every hit has contributors, signal==kind, 0<conf≤1; CAT=e2e; KILLS=BEHAVIOR-ONLY; VERDICT=KEEP
- pipeline_snapshot_stamp_provenance: INTENT=stamp carries schema version, git_head None outside worktree; CAT=e2e; KILLS=BEHAVIOR-ONLY; VERDICT=KEEP
- pipeline_repeated_full_runs_are_identical: INTENT=rebuild+research bit-identical incl exact score bits; CAT=e2e; KILLS=BEHAVIOR-ONLY; VERDICT=KEEP
- pipeline_reindex_is_stable_noop: INTENT=unchanged reindex indexes 0 files, results identical; CAT=e2e; KILLS=BEHAVIOR-ONLY; VERDICT=KEEP
- pipeline_empty_corpus_returns_absence_not_error: INTENT=empty corpus Ok-with-empty across query modes incl count-only; CAT=e2e; KILLS=error-on-empty mutants; VERDICT=KEEP
- pipeline_missing_root_fails_closed: INTENT=missing root and file-as-root fail closed at Indexer/Searcher ctors; CAT=e2e; KILLS=silent-default mutants; VERDICT=KEEP
- pipeline_limit_shapes_results_end_to_end: INTENT=limit truncates (2 of 5), echo names request, excerpts contain token; CAT=e2e; KILLS=limit-ignored mutants; VERDICT=KEEP
- pipeline_file_filter_keeps_matching_subtree: INTENT=filter keeps src/** 3/5, nomatch empty, unfiltered 5; CAT=e2e; KILLS=filter-ignored mutants; VERDICT=KEEP
- pipeline_count_only_reports_hand_computed_counts: INTENT=count-only per-file (1×5) hand table summing to 5; CAT=e2e; KILLS=count-aggregation mutants; VERDICT=KEEP
- pipeline_update_paths_add_then_remove: INTENT=incremental add searchable, remove leaves no def evidence, no collateral; CAT=e2e; KILLS=BEHAVIOR-ONLY; VERDICT=KEEP
- pipeline_pre_cancelled_index_fails_closed: INTENT=pre-cancelled index fails with INDEX_CANCELLED discriminant, recovers on clear; CAT=e2e; KILLS=cancel-ignored mutants; VERDICT=KEEP

## Helper patterns

Repeated code worth lifting into a shared testkit:

- `write_temp(bytes) -> NamedTempFile` — 3 near-identical copies (pass1 mmap, pass3 capped_read, pass3 scip). Top lift candidate.
- `SearchHit::span` builders — pass3 `mk_hit` + local `excerpt_of`, `hit_with`, `scored` wrappers; same shape repeated per test. One `testkit::mk_hit(kind, file, line, score)` plus mutators.
- Corpus+index fixture pair (`write_fixture`, `index_options`, `search_options`, `build`, `searcher`) — pass4 already coherent; lift as-is for reuse by future e2e passes.
- `hit_keys` / `sorted_files` / `hit_key` / `sorted_contributors` — key-projection helpers duplicated in spirit between pass3 (dedup/finish) and pass4 (determinism). One key-projection helper.
- Determinism-under-repetition assert pairs (`f(x)==f(x)`: drain, rel-path, follow-ups, routing, finish keys) — individually TAUTOLOGY-RISK in isolation (same-input rerun rarely discriminates); fine as trailing asserts, never as a test's sole content.
- `wire`/`decode` JSON builders (pass3 wire test) — single use; lift only if more wire tests appear.

## Counts

- Total: 46 (pass1: 7, pass2: 13, pass3: 14, pass4: 12)
- By CAT: limit-clamp 4, query-parse 6, scoring 6, fusion 3, e2e 12, other 15
- By VERDICT: KEEP 45, MERGE 1 (fts_query_joins_terms_with_or → fts_term_escaping_matches_hand_table), DELETE 0
- TAUTOLOGY-RISK as sole test content: 0; no one-assert tests exist (smallest is 2 asserts with a unique value pin)

# numerical-mcp catalog

Intent catalog of `tests/mcp/numerical_pass{1,2,3,4}.rs` (40 tests, all stdio-protocol driven).
Merge targets are proposed per-surface contract tests (one anchor per arithmetic surface).

## pass1 — exact hand-computed values (Mission N1)

- per_ref_budgets_deal_remainder_to_first_refs: INTENT=5/3 split deals [2,2,1] with exact contents aa/bb/c; CAT=exactness; KILLS=remainder-deal mutant (dropped/to-last/float-div); VERDICT=MERGE→code_read-split-contract
- zero_budget_tail_yields_empty_truncated_node: INTENT=1/2 split yields second node ("",true), not error or ("",false); CAT=exactness; KILLS=zero-budget guard / truncated-flag mutant; VERDICT=MERGE→code_read-split-contract
- context_window_clamps_to_file_edges: INTENT=L1 ctx100→(1,5), L5 ctx2→(3,5) on 5-line file; CAT=exactness; KILLS=missing-clamp / off-by-one-widen mutant; VERDICT=MERGE→code_read-window-contract
- context_window_asymmetric_at_edges: INTENT=range requests clamp one-sided: L1-L2 ctx1→(1,3), L4-L5 ctx1→(3,5); CAT=exactness; KILLS=symmetric-extension mutant (start 0 / end 6); VERDICT=MERGE→code_read-window-contract
- truncation_counts_chars_not_bytes_with_exact_boundary: INTENT="ééé" truncates by chars (2→"éé"/true, 3→full/false, 1→"é"/true); CAT=exactness; KILLS=byte-slice / `>=`-vs-`>` flag mutant; VERDICT=MERGE→code_read-truncation-contract
- empty_file_reads_as_single_empty_line: INTENT=0-byte file is one virtual line: L1-L1→("",false), L1-L2 rejects; CAT=exactness; KILLS=total_lines 0-or-2 mutant; VERDICT=MERGE→code_read-emptyfile-contract
- zd_echoes_budget_and_spent_matches_snippet_bytes: INTENT=zd[0] echoes budget (7, 65536), zd[1] equals re-summed snippet bytes; CAT=exactness; KILLS=zd echo dropped/swapped, cost-vs-body drift; VERDICT=MERGE→keyword_search-budget-contract
- full_preview_defaults_to_8192_while_unbudgeted_short_has_no_zd: INTENT=preview=full defaults budget to 8192; unbudgeted short/none omit zd; CAT=exactness; KILLS=default-literal / renderer-arm-swap mutant; VERDICT=KEEP
- tools_list_schema_bounds_match_parser: INTENT=tools/list schema bounds equal parser literals; ttlMs is exactly 3_600_000; CAT=exactness; KILLS=schema/parser bound drift; VERDICT=KEEP

## pass2 — boundary sides + degenerate totality (Mission N2)

- limit_boundary_sides_accept_1_and_100_reject_0_and_101: INTENT=limit accepts 1 and 100 (1 caps to one row), rejects 0 and 101; CAT=totality; KILLS=bound off-by-one mutant; VERDICT=MERGE→keyword_search-limit-contract
- limit_degenerate_rejects_negative_huge_float_string: INTENT=limit -1/u64MAX/1.5/"4"/2^64 all reject, session survives; CAT=totality; KILLS=panic / silent-clamp / wraparound-accept mutant; VERDICT=MERGE→keyword_search-limit-contract
- budget_boundary_sides_accept_1_and_65536_reject_0_and_65537: INTENT=budget accepts 1 and 65536 (zd[0] echo), rejects 0 and 65537; CAT=totality; KILLS=bound off-by-one mutant; VERDICT=MERGE→keyword_search-budget-contract
- budget_degenerate_rejects_negative_huge_float_string_bool: INTENT=budget -1/u64MAX/1.5/"many"/true all reject, session survives; CAT=totality; KILLS=wire-type accept / panic mutant; VERDICT=MERGE→keyword_search-budget-contract
- context_boundary_sides_accept_0_and_100_reject_101_and_degenerate: INTENT=ctx accepts 0 (exact window) and 100 (clamped), rejects 101/-1/u64MAX/1.5; CAT=totality; KILLS=bound off-by-one mutant; VERDICT=MERGE→code_read-window-contract
- max_chars_boundary_sides_accept_1_and_1000000_reject_0_and_1000001: INTENT=max_chars accepts 1 (truncates) and 10^6 (whole), rejects 0/10^6+1/-1/u64MAX/"many"; CAT=totality; KILLS=bound off-by-one mutant; VERDICT=MERGE→code_read-truncation-contract
- ids_arity_edges_reject_empty_and_21_accept_single: INTENT=ids rejects [] (no div-by-zero) and 21 items, accepts 1; CAT=totality; KILLS=div-by-zero / arity-fencepost mutant; VERDICT=KEEP
- twenty_ref_split_spends_max_chars_exactly: INTENT=25/20 split yields [2;5]+[1;15], total exactly 25; CAT=exactness; KILLS=remainder-dropped / round-up / n=20-fencepost mutant; VERDICT=MERGE→code_read-split-contract
- empty_file_totality_under_min_and_max_budgets: INTENT=empty file reads ("",false) at budgets 1 and 10^6; L2-L2 rejects; CAT=totality; KILLS=empty-truncation flag mutant; VERDICT=MERGE→code_read-emptyfile-contract
- line_window_edges_for_missing_and_bare_newlines: INTENT="solo"/"x\n"/"\n" each total 1 line; past-total rejects; CAT=totality; KILLS=line-count mutant; VERDICT=MERGE→code_read-emptyfile-contract
- huge_line_truncates_to_budget_in_chars_not_bytes: INTENT=5000-char and 100x"é" lines keep exactly 10 chars at budget 10; CAT=exactness; KILLS=byte-slice / scale-miscount mutant; VERDICT=MERGE→code_read-truncation-contract

## pass3 — metamorphic relations (Mission N3)

- clamp_is_idempotent_from_top_edge: INTENT=re-clamping a saturated top window (same/larger ctx) is a fixed point; CAT=metamorphic; KILLS=wrong-endpoint / double-ctx / rewiden mutant; VERDICT=MERGE→code_read-window-contract
- clamp_is_idempotent_from_bottom_edge: INTENT=re-clamping a saturated bottom window is a fixed point (mirror); CAT=metamorphic; KILLS=asymmetric top/bottom clamp mutant; VERDICT=MERGE→code_read-window-contract
- per_ref_split_conserves_saturated_budget: INTENT=saturated 4-ref split sums to exactly the cap (10, 15); CAT=metamorphic; KILLS=remainder-dropped / double-counted / round-up mutant; VERDICT=MERGE→code_read-split-contract
- per_ref_split_conserves_unsaturated_total: INTENT=unsaturated split sums to fs-oracle total, cap-independent, no truncation; CAT=metamorphic; KILLS=pad-to-cap / truncate-when-sufficient mutant; VERDICT=MERGE→code_read-split-contract
- truncation_is_monotone_in_cap_single_ref: INTENT=lengths non-decreasing in cap, prefix-chained, flag flips once, saturation agrees; CAT=metamorphic; KILLS=reversed-comparison / byte-char-confusion / flag-flap mutant; VERDICT=MERGE→code_read-truncation-contract
- truncation_total_is_monotone_in_cap_multi_ref: INTENT=per-node and total chars non-decreasing in cap; saturated totals equal fs total; CAT=metamorphic; KILLS=per-node-shrink / remainder-rotation mutant; VERDICT=MERGE→code_read-split-contract
- window_nesting_contains_smaller_context: INTENT=starts non-increasing, ends non-decreasing, contents nest as ctx grows; CAT=metamorphic; KILLS=wrong-side / window-content-mismatch mutant; VERDICT=MERGE→code_read-window-contract
- window_nesting_contains_subrange_and_equivalent_spellings: INTENT=subranges nest; (L5,ctx1)==(L4-L6,ctx0) and (L4-L6,ctx1)==(L3-L7,ctx0); CAT=metamorphic; KILLS=range/context-conflation mutant; VERDICT=MERGE→code_read-window-contract
- rerun_is_byte_identical_within_session: INTENT=repeated read+search in one session are byte-identical; CAT=metamorphic; KILLS=per-call-counter / elision-state-leak mutant; VERDICT=MERGE→rerun_is_byte_identical_across_sessions
- rerun_is_byte_identical_across_sessions: INTENT=same calls in fresh processes are byte-identical; CAT=metamorphic; KILLS=time-seed / hash-order / ranking-flip mutant; VERDICT=KEEP
- limit_is_monotone_in_rows_with_stable_head: INTENT=row counts non-decreasing and within limit; head row identical across limits; CAT=metamorphic; KILLS=limit-ignored / limit-as-offset / limit-dependent-ranking mutant; VERDICT=MERGE→keyword_search-limit-contract
- multibyte_truncation_scales_chars_not_bytes: INTENT=on 24x"é", bytes always 2x chars across caps, prefix-chained, saturation agrees; CAT=metamorphic; KILLS=byte-step / split-code-point mutant; VERDICT=MERGE→code_read-truncation-contract

## pass4 — end-to-end numeric drills (Mission N4)

- split_chain_seven_then_eight_over_three_refs: INTENT=7/3→[3,2,2] then 8/3→[3,3,2] with exact strings and totals; CAT=numeric-drill; KILLS=remainder-dropped/rotated / cross-call-state-leak mutant; VERDICT=MERGE→code_read-split-contract
- split_chain_eleven_then_fourteen_over_four_refs: INTENT=11/4→[3,3,3,2] then 14/4→[4,4,3,3] with exact strings and totals; CAT=numeric-drill; KILLS=remainder-wrong-end / total-drift mutant; VERDICT=MERGE→code_read-split-contract
- window_sweep_exact_table_at_center: INTENT=ctx 0..=3 at L4 yields exact (start,end,content,chars,bytes,lines) table; CAT=numeric-drill; KILLS=clamp / content-size drift mutant; VERDICT=MERGE→code_read-window-contract
- limit_sweep_exact_counts_on_three_file_tree: INTENT=limits 1/2/3/100 yield 1/2/3/3 rows, zn echoes, 32-byte snippets, stable head; CAT=numeric-drill; KILLS=limit-ignored/offset / count-zn drift mutant; VERDICT=MERGE→keyword_search-limit-contract
- budget_chain_exact_echo_and_byte_totals: INTENT=budgets 3/50/200 fund 0/32/96 snippet bytes with exact zd echoes; CAT=numeric-drill; KILLS=echo-drift / cost-vs-body drift mutant; VERDICT=MERGE→keyword_search-budget-contract
- elision_chain_exact_counts_and_bytes: INTENT=first send 96B/no-ze, repeat 3B/ze=3 with "~", resend 96B/no-ze; CAT=numeric-drill; KILLS=elision off-by-one / ze-missing / resend-`~`-leak mutant; VERDICT=KEEP
- truncation_chains_exact_bytes_ascii_and_greek: INTENT=a-z and αβγδε give exact char/byte/flag tables across caps; CAT=numeric-drill; KILLS=byte-step / flag-flip / split-code-point mutant; VERDICT=MERGE→code_read-truncation-contract
- search_to_read_handoff_exact_window_and_bytes: INTENT=search→compact-id read→L1-L2 truncation (33/47/48) pipeline with exact bytes; CAT=numeric-drill; KILLS=count/window/byte pipeline-drift mutant; VERDICT=KEEP

## Helper patterns

- `rpc_session`: one server spawn per test, initialize handshake, N requests, assert clean exit (panic = EOF/failure).
- `rpc_session_versioned`: protocol-version override; used once to pin `ttlMs` on a 2026-07-28 session.
- `rpc_handoff_session` (pass4 only): live session chaining search → compact hit id → fixed follow-up reads.
- `search_call` / `read_call`: JSON-RPC builders for `keyword_search` / `code_read`.
- `tool_body` / `tool_text`: parse raw tool text as JSON / compare raw bytes for determinism.
- `is_error`: `isError` discriminant (never message text).
- Fixtures: `indexed_tree*` (1/3/5-file indexed trees), ad-hoc `tempfile` trees, `node_content` / `window` / `snippet_bytes` accessors, fs-oracle totals via `read_to_string`.

## Counts

- Total: 40 (pass1: 9, pass2: 11, pass3: 12, pass4: 8)
- By CAT: exactness 11, totality 9, metamorphic 12, numeric-drill 8, other 0
- By VERDICT: KEEP 6, MERGE 34, DELETE 0
- KEEP anchors: tools_list_schema_bounds_match_parser, ids_arity_edges_reject_empty_and_21_accept_single, full_preview_defaults_to_8192_while_unbudgeted_short_has_no_zd, rerun_is_byte_identical_across_sessions, elision_chain_exact_counts_and_bytes, search_to_read_handoff_exact_window_and_bytes
- Merge targets: code_read-split-contract (8), code_read-window-contract (8), code_read-truncation-contract (6), keyword_search-limit-contract (4), keyword_search-budget-contract (4), code_read-emptyfile-contract (3), rerun-anchor (1)

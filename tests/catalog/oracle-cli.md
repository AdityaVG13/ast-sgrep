# oracle-cli catalog (tests/cli/oracle_foundry_pass1–4.rs)

45 tests. Pass 1 = L1 hand-computed oracles (in-process). Pass 2 = L2
mutant-killing oracles (in-process). Pass 3 = L3 metamorphic/differential/
adversarial (real `asgrep` binary, keyword channel). Pass 4 = L4 e2e (real
binary: index/status/search/chain/outline/reindex, exit codes).

## pass1 (9)

- cpu_limit_bounds_match_contract: INTENT=bounds are (1,80,80); in-range parses, out-of-range/unparsable falls back to 80; CAT=cli-surface; KILLS=clamp-removal/fallback-default mutant; VERDICT=KEEP
- duty_cycle_windows_match_hand_windows: INTENT=hand 10ms windows for 50/0/100/1 pct, work+sleep==10 always; CAT=cli-surface; KILLS=formula/rounding mutant; VERDICT=KEEP
- utf16_offsets_map_to_hand_bytes: INTENT=UTF-16→byte incl BMP é and surrogate-pair 𝄞 midpoints, OOB clamps to len; CAT=offsets; KILLS=surrogate-midpoint comparison flip; VERDICT=KEEP
- identifier_extraction_matches_hand_spans: INTENT=identifier at cursor, space snaps left, empty/blank is None; CAT=offsets; KILLS=snap-direction swap; VERDICT=KEEP
- line_lookup_distinguishes_empty_from_absent: INTENT=trailing-newline yields Some(""), past-end and empty-doc idx1 are None; CAT=offsets; KILLS=trailing-newline miscount; VERDICT=KEEP
- symbol_kinds_match_lsp_spec_numbers: INTENT=method/class/interface/enum/type map to spec ints 6/5/11/10/23, unknown/empty default 12; CAT=cli-surface; KILLS=kind-table/alias mutant; VERDICT=KEEP
- file_uri_decode_matches_hand_paths: INTENT=file:// decodes incl %20, non-file/empty/bare-path are Err; CAT=uris; KILLS=scheme-check/pct-decode removal; VERDICT=KEEP
- file_uri_roundtrips_through_canonical_path: INTENT=path→URI→path round-trips through canonicalized tempdir file; CAT=uris; KILLS=BEHAVIOR-ONLY (only canonical-roundtrip pin); VERDICT=KEEP
- text_edit_applies_replace_and_rejects_bad_ranges: INTENT=full and ranged replaces apply, OOB line range is Err not silent clamp; CAT=edits; KILLS=OOB-clamp mutant; VERDICT=KEEP

## pass2 (10)

- duty_cycle_zero_stays_zero_nonzero_clamped_up: INTENT=0 stays (0,10), 1..9 clamp work to 1, work monotone over 0..=100, sum==10; CAT=cli-surface; KILLS=.max(1)-arm confusion; VERDICT=MERGE→duty_cycle_windows_match_hand_windows (same fn, fold rows+loop into one table; zero loss)
- parse_cpu_limit_edge_spellings: INTENT=newline/tab trim ok; 5_0/0x10/fullwidth fall back to 80; CAT=cli-surface; KILLS=trim-removal/leniency mutant; VERDICT=MERGE→cpu_limit_bounds_match_contract (same fn, fold 6 rows into bounds table)
- line_lookup_empty_and_trailing_edges: INTENT="" idx0 is Some(""); a-with/without-trailing-NL edge indices; CAT=offsets; KILLS=empty-content-None/trailing-NL mutant; VERDICT=MERGE→line_lookup_distinguishes_empty_from_absent (same fn, disjoint rows, one table)
- utf16_mid_surrogate_and_end_clamps: INTENT=mid-surrogate offset→byte 1 not 5, end/empty clamps; CAT=offsets; KILLS=`<`→`<=` surrogate flip, clamp removal; VERDICT=MERGE→utf16_offsets_map_to_hand_bytes (3 asserts literally duplicated; fold clamp rows in)
- extract_identifier_snaps_and_rejects: INTENT=past-end snaps to last ident, leading punct None, punct snaps left, mid-multibyte safe; CAT=offsets; KILLS=snap-right/OOB-None/panic mutant; VERDICT=MERGE→identifier_extraction_matches_hand_spans (same fn, disjoint rows, one table)
- symbol_kind_is_case_sensitive_with_function_default: INTENT=Method/METHOD/Class/struct all default 12; method=6, type=23; CAT=cli-surface; KILLS=case-folding/alias mutant; VERDICT=MERGE→symbol_kinds_match_lsp_spec_numbers (fold 4 case rows into spec table)
- file_uri_scheme_is_case_sensitive: INTENT=FILE:/ftp: are Err, %2F decodes, bare file:// is Ok; CAT=uris; KILLS=scheme case-folding mutant; VERDICT=MERGE→file_uri_decode_matches_hand_paths (same fn; fold into decode table)
- text_edit_range_length_overrides_end: INTENT=rangeLength overrides end (insert vs span), mid-surrogate/overrun are Err; CAT=edits; KILLS=rangeLength-ignored mutant; VERDICT=KEEP (distinct feature, not a table fold)
- text_edit_reversed_and_oob_rejected: INTENT=reversed and OOB ranges Err, empty-doc (0,0) insert ok; CAT=edits; KILLS=order-check-removal/panic mutant; VERDICT=KEEP (distinct validation contract)
- apply_text_edit_never_panics_falls_back: INTENT=best-effort apply returns content on bad range, edited text on good; CAT=edits; KILLS=unwrap_or_else→unwrap mutant; VERDICT=KEEP (only test of apply_ vs try_ fallback)

## pass3 (14, all e2e binary)

- keyword_repetition_is_byte_identical: INTENT=repeated keyword JSON run is byte-identical, non-empty; CAT=e2e; KILLS=nondeterminism (hash-order/timestamp leak); VERDICT=KEEP
- outline_repetition_is_byte_identical: INTENT=repeated outline JSON run is byte-identical; CAT=e2e; KILLS=nondeterminism on outline path; VERDICT=KEEP (distinct binary path from keyword)
- json_vs_human_hit_count_agrees: INTENT=human keyword rows == JSON hits count, clean stderr; CAT=e2e; KILLS=human-face header/footer/row-skew mutant; VERDICT=KEEP
- files_with_matches_human_equals_json_files_array: INTENT=files face is exactly [a.rs,b.rs] sorted/deduped in JSON and human; CAT=e2e; KILLS=sort/dedupe/face-divergence mutant; VERDICT=KEEP
- limit_growth_is_prefix_stable: INTENT=limit-2 hits == limit-50 prefix; saturation is fixed point; CAT=e2e; KILLS=limit-dependent-ranking mutant; VERDICT=KEEP (BEHAVIOR-ONLY note: wrong-but-deterministic ranking still passes)
- file_filter_narrowing_is_monotone: INTENT=filter keeps total order minus removed rows, shrinks set, no-match filter is ok:true zero hits; CAT=e2e; KILLS=filter-ignored/order-scramble mutant; VERDICT=KEEP
- keyword_cli_matches_library_search_lexical: INTENT=CLI keyword hit keys == library search_lexical keys (sorted); CAT=e2e; KILLS=CLI-envelope/marshal divergence; VERDICT=KEEP (only CLI-vs-lib differential; common-mode backend bugs invisible by design)
- outline_json_human_and_count_agree: INTENT=3 fns in line order, names/starts/kinds hand-pinned, human has 3 rows; CAT=e2e; KILLS=outline ordering/kind/span mutant; VERDICT=KEEP
- unicode_paths_index_search_and_outline: INTENT=non-ASCII filename round-trips through index, search hit, and outline; CAT=e2e; KILLS=path-mangling/non-UTF8-loss mutant; VERDICT=KEEP
- empty_file_indexes_searches_and_outline_refuses: INTENT=empty file coexists with search; outline on it is exit 2 ok:false; CAT=e2e; KILLS=fail-open-on-empty mutant; VERDICT=KEEP
- crlf_line_spans_are_hand_computed: INTENT=CRLF file outlines count 2 with starts [1,2]; CAT=e2e; KILLS=CRLF line-split mutant; VERDICT=KEEP
- huge_single_line_is_stable_and_located: INTENT=300KB single-line file indexes and locates hit at line 1; CAT=e2e; KILLS=line-length truncation/OOM-path mutant; VERDICT=KEEP
- invalid_utf8_is_stable_with_valid_json_output: INTENT=hostile file counted files_failed=1, stdout stays valid JSON, clean file searchable; CAT=e2e; KILLS=crash-on-invalid-UTF8/result-poisoning mutant; VERDICT=KEEP
- deep_directories_index_and_search: INTENT=25-deep nested leaf.rs indexes and is found by search; CAT=e2e; KILLS=depth-limit/walk-prune mutant; VERDICT=KEEP

## pass4 (12, all e2e binary)

- index_envelope_counts_match_hand_fixture: INTENT=index envelope hand counts 2 files / 0 failed / 3 symbols, exit 0; CAT=e2e; KILLS=count/envelope-shape mutant; VERDICT=KEEP
- search_finds_hand_placed_symbol_first: INTENT=search puts exact def hit first with symbol/file/1-1 span; CAT=e2e; KILLS=ranking/span-shape mutant; VERDICT=KEEP
- status_counts_match_hand_fixture: INTENT=status hand counts 2 files / 3 symbols / 1 caller edge; CAT=e2e; KILLS=status-count mutant; VERDICT=KEEP
- no_hits_is_ok_empty_exit_zero_both_channels: INTENT=no-hits is ok:true exit 0 with empty hits/human-stdout on search AND keyword; CAT=e2e; KILLS=fail-on-empty mutant; VERDICT=KEEP (already channel-looped; the merge pattern others should follow)
- missing_root_fails_closed_across_commands: INTENT=index and search on missing root are exit 2 ok:false operational; CAT=e2e; KILLS=fail-open-on-missing-root mutant; VERDICT=KEEP
- empty_tree_index_ok_then_search_fails_closed: INTENT=empty tree indexes ok with zero counts, search on it fails closed exit 2; CAT=e2e; KILLS=empty-index fail-open mutant; VERDICT=KEEP
- search_human_rows_equal_json_hits: INTENT=human search rows == JSON hits count, clean stderr; CAT=e2e; KILLS=human-face skew on search channel; VERDICT=KEEP (search≠keyword code path; mirror is load-bearing)
- search_repetition_is_byte_identical: INTENT=repeated search JSON run byte-identical, non-empty; CAT=e2e; KILLS=nondeterminism on search path; VERDICT=KEEP (same; channel-specific stability)
- search_files_with_matches_consistent_with_hits: INTENT=files==[a.rs,b.rs] and equal sorted-unique hit files; no-hits face empty exit 0; CAT=e2e; KILLS=files/hits inconsistency mutant; VERDICT=KEEP (stronger than pass3: adds internal consistency)
- reindex_picks_up_added_file: INTENT=reindex after adding b.rs reports 2 files and surfaces pass4_newcomer; CAT=e2e; KILLS=stale-index/reindex-noop mutant; VERDICT=KEEP
- chain_edges_follow_hand_call_graph: INTENT=isolated symbol 1 node/0 edges; beta chain carries the 1 hand edge; CAT=e2e; KILLS=call-graph edge-drop mutant; VERDICT=KEEP
- outline_mixed_kinds_match_hand_spans: INTENT=struct line1 kind type + fn line2 kind function, count 2, line order; CAT=e2e; KILLS=kind-conflation/span mutant; VERDICT=KEEP

## Helper patterns

- In-process builders (pass1/2): `full_replace`/`ranged`/`ranged_len` construct
  TextDocumentContentChangeEvent fixtures; tables of `(input, expected)` rows
  with per-row messages; discriminant asserts (`is_err`/`is_ok`/None) never
  message text.
- Binary harness (pass3/4, duplicated per file): `asgrep_bin` via
  CARGO_BIN_EXE, `run_raw` (code+stdout+stderr, NO_COLOR=1), `run_json`
  (panics with stdout+stderr dump if not JSON), `index_dir`, `Corpus`
  (TempDir root+index), `write_bytes` fixture writer.
- Query helpers: pass3 `keyword_json`/`keyword_codes`, pass4 `search_json`
  (channel-parametrized); `outline_corpus`/`search_corpus`/
  `adversarial_corpus` hand fixtures; `sorted_keys` + testkit
  `json_hit_keys`/`response_hit_keys`/`SurfaceHitKey` for order-free compares.
- Duplication note: `run_raw`/`run_json`/`write_bytes`/`asgrep_bin`/`Corpus`
  are copy-pasted between pass3 and pass4 — one shared `tests/cli/common.rs`
  (or testkit) module would delete ~60 lines with zero behavior change.

## Counts

- Total: 45 (pass1 9, pass2 10, pass3 14, pass4 12).
- By CAT: e2e 26, cli-surface 6, offsets 6, edits 4, uris 3, other 0.
- By VERDICT: KEEP 38, MERGE 7 (all pass2 same-function table folds into
  pass1), DELETE 0.
- TAUTOLOGY-RISK: 0 flagged (limit-prefix and CLI-vs-lib are self/differential
  comparisons but kill channel-specific mutants; noted inline, not tautologies).
- No one-assert tests found; every test carries ≥2 asserts or a table/loop.

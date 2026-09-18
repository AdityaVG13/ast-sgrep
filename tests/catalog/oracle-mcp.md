# Oracle-MCP catalog (26 tests, agent: catalog oracle-mcp)

No pass1 exists. All tests drive `asgrep-mcp` over stdio; discriminants are `isError`/codes/shapes, never message text.

## tests/mcp/oracle_foundry_pass2.rs (L2 arg boundaries, 5 tests)

- search_limit_accepts_1_and_100_rejects_0_and_101: INTENT=limit accepts 1 and 100, rejects 0 and 101, hits non-empty; CAT=validation; KILLS=bound off-by-one both sides, bound-check-removed; VERDICT=KEEP
- budget_tokens_accepts_1_and_max_rejects_0: INTENT=budget 1 and 65536 accepted with 6-wide detail tuples, 0 rejected; CAT=validation; KILLS=floor-flip and budget-ignored mutants; ceiling-drop NOT killed (no 65537-reject case despite comment claim); VERDICT=KEEP
- code_read_context_zero_is_exact_window: INTENT=context 0/100 accepted with exact L3-L3 window, 101 and max_chars 0 rejected; CAT=validation; KILLS=context floor/ceiling flips, window off-by-one; max_chars accept-side untested (reject-only); VERDICT=KEEP
- code_read_rejects_zero_start_and_reversed_range: INTENT=L1-L1 reads, L0-L1 and L2-L1 rejected; CAT=validation; KILLS=start>0-dropped, end>=start-dropped; VERDICT=KEEP
- search_query_single_char_ok_overlong_rejected: INTENT=query len 1 and 4096 accepted, 4097 rejected; CAT=validation; KILLS=ceiling-dropped, ceiling-flip; empty-string reject missing (floor reject side untested; pass3 pins only whitespace-only); VERDICT=KEEP

## tests/mcp/oracle_foundry_pass3.rs (L3 relations/adversarial, 10 tests)

- code_search_alias_matches_keyword_search_exactly: INTENT=alias agrees byte-identically incl structuredContent; CAT=mcp-surface; KILLS=alias-drift/compat mutants; VERDICT=KEEP
- repeated_index_status_and_code_read_are_byte_identical: INTENT=pure reads repeat byte-for-byte; file_count 1, exact content; CAT=mcp-surface; KILLS=state-leak/nondeterminism mutants; VERDICT=KEEP
- limit_growth_is_prefix_stable: INTENT=limit 2→8 extends hits without reorder/rewrite, path table monotonic; CAT=mcp-surface; KILLS=ranking-instability mutants; VERDICT=KEEP
- query_trim_and_preview_case_are_normalized: INTENT=whitespace trimmed (echoed q), preview case-insensitive; CAT=mcp-surface; KILLS=trim/case-fold-removed mutants; VERDICT=KEEP
- preview_none_keeps_ids_drops_snippets: INTENT=preview=none blanks snippets, keeps ids/ranking vs short; CAT=mcp-surface; KILLS=preview-ignored mutants (control proves snippets exist); VERDICT=KEEP
- unknown_tool_and_bad_search_args_share_tool_error_shape: INTENT=11 bad tool/arg calls share one tool-error shape; unicode query runs as miss; CAT=validation; KILLS=shape-divergence/dispatch mutants; VERDICT=KEEP
- adversarial_code_read_ids_rejected_server_survives: INTENT=10 adversarial id shapes are uniform tool errors; CAT=validation; KILLS=arg-shape validation mutants; liveness claim VOID (valid read runs in a FRESH process, not the poisoned session); VERDICT=KEEP
- index_repo_twice_keeps_status_file_count_stable: INTENT=reindex idempotent, second run indexes 0, status stable; CAT=mcp-surface; KILLS=reindex-duplication mutants; VERDICT=KEEP
- pipelined_batch_returns_every_id_exactly_once: INTENT=6 fire-all mixed requests each answered once with result/error xor; CAT=mcp-surface; KILLS=drop/duplicate/crosstalk mutants; VERDICT=KEEP
- malformed_envelope_topology_method_vs_tool_errors: INTENT=unknown method/unshaped envelopes are top-level -32601; absent/null args default to {} then tool-error; CAT=validation; KILLS=topology-confusion/default-args mutants; VERDICT=KEEP

## tests/mcp/oracle_foundry_pass4.rs (L4 end-to-end flows, 11 tests)

- full_chain_list_search_read_in_one_session: INTENT=list→search→read chain consuming prior outputs; tools.len()==8; zn==hits; CAT=e2e; KILLS=BEHAVIOR-ONLY; "one session" is 3 spawns, flow duplicated by next two tests, only unique value is the len==8 tripwire; VERDICT=MERGE→tool_names_consumed_from_list_response_drive_search_and_read (carry the len==8 assertion)
- tool_names_consumed_from_list_response_drive_search_and_read: INTENT=client dispatches by advertised names, search→read succeeds; CAT=e2e; KILLS=tool-rename/dispatch mutants; VERDICT=KEEP
- search_then_read_every_hit_resolves_with_matching_count: INTENT=fan-out read returns exactly one well-formed non-empty node per hit; CAT=e2e; KILLS=fan-out drop/extra mutants; VERDICT=KEEP
- search_path_table_agrees_with_read_node_files: INTENT=read-back files are members of search p table, all 3 files round-trip; CAT=e2e; KILLS=BEHAVIOR-ONLY cross-tool consistency (no ground-truth filename assertion; table decoded by production resolver so shared-encoder bugs pass; counts still kill fan-out mutants); VERDICT=KEEP
- ast_search_chain_reads_pattern_hits: INTENT=structural hits carry kind p and expand to non-empty nodes; CAT=e2e; KILLS=channel/kind mutants (only ast_search→read coverage); VERDICT=KEEP
- index_lifecycle_miss_then_index_then_hit_then_read: INTENT=empty_index miss→index 2 files→2-path hits→read (4 spawns despite "one session" comment); CAT=e2e; KILLS=lifecycle mutants; VERDICT=KEEP
- error_taxonomy_end_to_end_method_vs_tool_vs_args_vs_root: INTENT=method -32601 vs tool/arg/root uniform tool errors side by side; CAT=e2e; KILLS=taxonomy-confusion mutants; partial overlap with pass3 topology test but root-arg cases are new; VERDICT=KEEP
- missing_root_fails_closed_across_tools_then_recovers: INTENT=missing per-call root fails all tools uniformly, next valid call succeeds; CAT=e2e; KILLS=fail-open/session-poison mutants; VERDICT=KEEP
- session_recovers_after_errors_without_restart: INTENT=unknown tool + bad read don't poison session; CAT=e2e; KILLS=poison mutants; overclaim: only search replays in poisoned process, read half runs in fresh process; VERDICT=KEEP
- session_rerun_is_deterministic_across_processes: INTENT=identical chain in two fresh processes returns identical bytes; CAT=e2e; KILLS=per-process-state-leak mutants; VERDICT=KEEP
- empty_tree_fails_closed_status_search_read: INTENT=empty tree: file_count 0, empty_index miss (never bare empty list), read is tool error; CAT=e2e; KILLS=fabrication/fail-open mutants; VERDICT=KEEP

## Helper patterns

- `mcp_bin()`: CARGO_BIN_EXE_asgrep-mcp → CARGO_TARGET_DIR → manifest-relative target; copied verbatim in all 3 files (no shared module).
- `rpc_session(payloads, root)`: one spawn, initialize handshake, strictly sequential send/recv, asserts clean exit; per-test process isolation.
- `rpc_pipeline` (pass3 only): fire-all-then-collect for concurrency; asserts id multiset, not order.
- Builders `search_call`/`read_call` (pass2) vs generic `tool_call(id, name, args)` (pass3/4); extractors `tool_text`/`tool_body` (pass3/4 only).
- `assert_tool_error_shape` (pass3/4): isError true, no top-level error, single text block, no structuredContent.
- Fixtures: `indexed_tree` (1 file, all files), `multi_hit_tree` (pass3, 6+1 files), `multi_file_tree` (pass4, 3 files), `unindexed_tree` (pass4 lifecycle), ad-hoc tempdirs (pass2 windows); pass4 adds `index_tree(path)` helper and `distinct_hit_paths` (path-id prefix multiset).

## Counts

- Total: 26 (pass2: 5, pass3: 10, pass4: 11). No pass1.
- CAT: validation 8, mcp-surface 7, e2e 11, other 0.
- VERDICT: KEEP 25, MERGE 1 (full_chain→tool_names_consumed), DELETE 0.
- Gaps noted (not verdicts): pass2 budget ceiling-reject, query empty-reject, max_chars accept-side; pass3 liveness fresh-process; pass4 "one session" misnomers (3/4 spawns), path-table no ground-truth filenames.

# FFI bilateral test catalog

Scope: `tests/codemode/ffi_bilateral_pass{1,2,3,4}.rs` — 49 tests over
`ast-sgrep-codemode-napi` (`Session::new` / `call_now` / `call` / `batch`
construction, no JS `Env`; async `compute()` unobservable from Rust by design).

Counts: **KEEP 41 · MERGE 7 · DELETE 1** (of 49).

Categories: `export-inventory` (pins the export exists / happy-path shape),
`validation` (boundary/reason/counting pins), `bilateral` (napi-vs-core
agreement), `simulation` (JS-client drills), `other`.
"Kills" names the mutant class the test discriminates; BEHAVIOR-ONLY means it
pins behavior without an obvious single mutant; TAUTOLOGY-RISK means it likely
cannot fail.

## Pass 1 — export inventory (`ffi_bilateral_pass1.rs`, 14)

| # | test | intent | cat | kills | verdict |
|---|------|--------|-----|-------|---------|
| 1 | `identity_markers_are_exact` | Pins `binding_version`/`is_native`/`async_api_version==1` | export-inventory | constant-change (api_version) | KEEP |
| 2 | `session_new_none_and_each_config_field_map` | `None` defaults + each config field maps; limit clamp edges 0/1/500/600 still construct and serve | validation | mapping-swap, clamp-drop | KEEP |
| 3 | `session_new_index_path_flows_to_status` | Custom `index_path` file name surfaces in `index_status` (proves flow-through, not default home) | validation | mapping-swap (default-path fallback) | KEEP |
| 4 | `call_now_catalog_tools_return_contract_shapes` | `catalog_search` finds "search"; `catalog_describe` returns name+nonempty description; count==2 | export-inventory | dispatch-swap, shape-regression | KEEP |
| 5 | `call_now_fast_lookups_succeed_on_empty_root` | Pre-materialize `find` fails closed (counted); post-materialize find/defs/callers/imports render consistent capsules | export-inventory | gate-reorder, count-on-fail-swap | KEEP |
| 6 | `call_now_read_serves_fixture_window` | `read` serves exact window bytes incl. `fn alpha` | export-inventory | path/window-slice-swap | KEEP (merge target for P4T8) |
| 7 | `call_now_search_cold_returns_null_without_counting` | Unique search falls through as `Null`, count stays 0 | validation | Null-vs-error-swap, count-bump-on-fallthrough | KEEP |
| 8 | `call_now_search_hit_after_find_returns_identical_render` | `find("N")` then `search("word:N")` returns identical render, counted | validation | cache-key-swap, render-divergence | KEEP |
| 9 | `call_now_slow_and_unknown_tools_rejected_with_contract_reason` | 6 slow tools + unknown rejected with exact `CALL_NOW_ONLY`, uncounted | validation | reason-change, gate-after-bump | MERGE→P2T5 (`call_now_fast_slow_partition…` covers the same 6 tools + aliases with the same constant and also asserts counting; unknown case ⊂ P2T8) |
| 10 | `call_now_invalid_args_errors_but_still_counts` | `defs{}` errors non-empty, count==1 | validation | count-on-fail-swap | MERGE→P2T7 (`call_now_invalid_args_reasons…` pins the exact `defs` reason + counting for 8 cases; this non-empty-only pin is strictly weaker) |
| 11 | `call_now_concurrent_failures_use_only_busy_reason` | Under 8×25 contention every failure is exactly "session is busy"; count bounded | validation | busy-reason-change | MERGE→P2T6 (`call_now_busy_reason_is_exact…` pins the same reason non-vacuously; this one passes vacuously with zero collisions) |
| 12 | `async_call_and_batch_construct_without_env` | `call`/`batch` construct `Ok` with arg defaulting, uncounted | export-inventory | construction-reject-regression | KEEP (only happy-path construction pin with args) |
| 13 | `batch_validation_reasons_are_exact_and_synchronous` | All 6 batch violation classes rejected with exact reasons, uncounted | validation | reason-change, missing-check | KEEP (canonical reason text source; P3T11 proves agreement, not text) |
| 14 | `napi_object_struct_fields_roundtrip` | Asserts struct literals read back their fields | other | TAUTOLOGY-RISK | DELETE — asserts Rust field assignment, no FFI/serde behavior (no serialization roundtrip, no napi conversion); fails only if `struct` breaks |

## Pass 2 — boundary validation (`ffi_bilateral_pass2.rs`, 12)

| # | test | intent | cat | kills | verdict |
|---|------|--------|-----|-------|---------|
| 1 | `batch_exact_max_boundaries_construct_without_counting` | Exact-max calls/id/tool bytes construct (`>` is exclusive) | validation | off-by-one (`>`→`>=`) | KEEP |
| 2 | `batch_limits_count_bytes_not_chars` | 128-byte multibyte id/tool accepted, 130-byte rejected; pins 128 literally | validation | bytes-vs-chars-swap | KEEP |
| 3 | `batch_reports_first_violation_in_validation_order` | Overcount wins over per-call violations; scan covers whole list | validation | check-reorder, first-only-scan | KEEP |
| 4 | `construction_validates_identity_only_and_never_executes` | `call` validates nothing; `batch` accepts dup/whitespace ids and non-object args | validation | over-validation (added semantic checks) | KEEP |
| 5 | `call_now_fast_slow_partition_is_exact_over_catalog_and_aliases` | 8 fast tools succeed + cold search Null; 6 slow + 12 alias spellings gated with exact reason; count==9 | validation | gate-literal-vs-parse-swap, reason-change | KEEP (canonical partition pin) |
| 6 | `call_now_busy_reason_is_exact_under_contention` | 16-thread hammer until ≥1 busy loser; every failure exactly "session is busy" | validation | busy-reason-change | KEEP (canonical busy pin; non-vacuous) |
| 7 | `call_now_invalid_args_reasons_are_exact_contract_texts` | 8 invalid-args cases pinned to exact reason texts incl. InvalidArgs-vs-Other taxonomy; all counted | validation | reason-change, taxonomy-swap | KEEP (canonical reason pin) |
| 8 | `call_now_gate_precedes_dispatch_while_core_taxonomy_pins_directly` | Unknown/empty/case/padded/injection spellings gated uncounted; core `UnknownTool`/`InvalidArgs` discriminants pinned directly; napi reason byte-identical to core `Display` | bilateral | gate-after-dispatch, marshalling-drift | KEEP |
| 9 | `find_and_catalog_unicode_and_empty_roundtrip_without_panic` | Multibyte + empty queries render consistent capsules; unicode through catalog | validation | panic-on-multibyte, truncation | KEEP |
| 10 | `huge_query_fails_closed_naming_the_char_limit` | Oversize `find`/`defs` fail with exact limit reasons (defs names composed 4102); huge `search` falls through Null uncounted | validation | reason-change, compose-before-validate-swap | KEEP (note: duplicates `MAX_QUERY_CHARS=4096` literally — drift risk if core changes it) |
| 11 | `read_unicode_fixture_roundtrips_exact_text` | Unicode filename + content reads back byte-exact | validation | encoding-mangle, truncation | KEEP |
| 12 | `invalid_root_configs_fail_closed_at_first_use_with_documented_reasons` | Lazy construction; missing root / escaping per-call root / unresolvable root fail with documented prefixes; ENOTDIR index_path fails closed (discriminant only) | validation | eager-validation, jail-bypass | KEEP (jail prefix is security-relevant) |

## Pass 3 — bilateral agreement (`ffi_bilateral_pass3.rs`, 14)

All share one shape: napi `Session` + core `CodeModeSession` on the same
root/db, `assert_byte_identical` over canonical serialization.

| # | test | intent | cat | kills | verdict |
|---|------|--------|-----|-------|---------|
| 1 | `find_on_indexed_hits_is_byte_identical` | `find` over real indexed hits agrees byte-identical (hit_count≥1, non-vacuous) | bilateral | render-divergence, ordering-swap | KEEP |
| 2 | `read_on_fixture_window_is_byte_identical` | `read` window agrees byte-identical | bilateral | window-slice-divergence | KEEP |
| 3 | `defs_capsule_is_byte_identical` | `defs` zero-hit capsule agrees byte-identical | bilateral | capsule-shape-divergence | KEEP (merge target for T4,T5) |
| 4 | `callers_capsule_is_byte_identical` | Same as T3 for `callers` | bilateral | capsule-shape-divergence | MERGE→T3 — fold all three into one `symbol_capsules_are_byte_identical` loop over `(tool, args)`; identical 13-line bodies, zero coverage loss |
| 5 | `imports_capsule_is_byte_identical` | Same as T3 for `imports` | bilateral | capsule-shape-divergence | MERGE→T3 (same) |
| 6 | `index_status_is_byte_identical_on_shared_db` | `index_status` agrees incl. embedded root/index_path/counts | bilateral | config-mapping-divergence | KEEP |
| 7 | `catalog_search_is_byte_identical` | `catalog_search` agrees byte-identical | bilateral | catalog-dispatch-divergence | KEEP (merge target for T8) |
| 8 | `catalog_describe_is_byte_identical` | `catalog_describe` agrees byte-identical | bilateral | catalog-dispatch-divergence | MERGE→T7 — one `catalog_tools_are_byte_identical` test, two blocks, same pair setup |
| 9 | `search_cache_hit_agrees_and_counts_on_both` | Cache hit agrees byte-identical via `call_now` vs `take_cached_search`; both count 3 | bilateral | cache-key-divergence, count-divergence | KEEP |
| 10 | `search_cold_falls_through_uncounted_on_both` | Cold search is `Null`/`None` on both, both uncounted | bilateral | fallthrough-divergence | KEEP |
| 11 | `batch_validation_reasons_agree_byte_identical` | All 6 batch violation reasons napi==core `Display`, core discriminant `InvalidArgs` | bilateral | validation-layer-fork | KEEP |
| 12 | `call_count_matches_after_identical_mixed_sequence` | Mixed ok/error sequence: same ok-ness both sides, both count 6 | bilateral | count-divergence | KEEP |
| 13 | `invalid_args_fail_on_both_with_identical_reasons` | 8 bad-arg cases fail both sides, reasons byte-identical, discriminants pinned, both counted | bilateral | marshalling-drift, taxonomy-divergence | KEEP (extends P2T8's single-case fidelity to all cases) |
| 14 | `budget_exhaustion_fails_identically_on_both` | 10k calls both sides, 10_001st fails identically; core discriminant `BudgetExhausted(10_000)`; counts pin | bilateral | budget-cap-divergence | KEEP (pairs with P4T2, which pins the JS-visible text this test never states) |

## Pass 4 — JS-client simulation (`ffi_bilateral_pass4.rs`, 9)

No core imports; NAPI surface only.

| # | test | intent | cat | kills | verdict |
|---|------|--------|-----|-------|---------|
| 1 | `js_client_full_flow_status_find_read_search_cache` | status→find→read→cold-search→find→cached-search flow with exact values and count after every step | simulation | sequencing/count-choreography-swap | KEEP (the one end-to-end flow test) |
| 2 | `js_client_error_flow_bad_tool_bad_args_budget` | Bad tool gated uncounted; bad args counted; 10k budget fill then exact `BUDGET` text, count pins | simulation | budget-reason-change, cap-swap | KEEP — only pin of the JS-visible budget text; first two thirds overlap P2T8/P2T7 but serve as the counted setup (trimmable, not deletable) |
| 3 | `js_client_batch_construction_accepts_mixed_semantics` | Batch with valid + unknown-tool + invalid-args calls constructs Ok; only empty-id rejects with exact reason | simulation | semantic-validation-at-construction | KEEP — unknown-tool/invalid-args *inside batch* constructing Ok is not covered by P2T4 (which covers those only for `call`); trailing empty-id block duplicates P1T13 and could be trimmed |
| 4 | `js_client_busy_gate_retry_contract_under_contention` | Busy reason exact under hammering; count ≤ successes; sequential retry succeeds post-contention | simulation | busy-reason-change, post-contention-poison | KEEP (post-contention health check is unique) |
| 5 | `js_client_same_session_repeats_are_byte_identical` | Repeated find/describe on one session are byte-identical | simulation | BEHAVIOR-ONLY (nondeterminism, state pollution) | KEEP |
| 6 | `js_client_fresh_sessions_agree_byte_identical` | Two sessions over one store produce byte-identical 4-step flows, counts 4/4 | simulation | BEHAVIOR-ONLY (session-state leak into renders) | KEEP |
| 7 | `js_client_sessions_are_isolated_caches_and_budgets` | A caches a key; B probing it still gets Null uncounted; budgets independent | simulation | cache-leak-across-sessions, shared-budget | KEEP (isolation is correctness/security-relevant) |
| 8 | `js_client_read_window_returns_exact_bytes` | `read` start=2/end=3 returns exact middle bytes | simulation | start-ignored-swap | MERGE→P1T6 (`call_now_read_serves_fixture_window`) — add as a second window case on the existing fixture; standalone 20-line test for one assertion |
| 9 | `js_client_error_reasons_are_stable_across_repeats` | Gate + InvalidArgs reasons byte-stable across repeats; counts exact | simulation | BEHAVIOR-ONLY (counter/timestamp in reasons, first-vs-rest divergence) | KEEP — thin but cheap; catches call-dependent reason text that single-shot P2T7 can miss |

## Repeated helper patterns (testkit candidates)

Duplicated verbatim or near-verbatim across the 4 files; extract into one
shared module (e.g. `tests/codemode/ffi_testkit.rs`, included via
`#[path = "ffi_testkit.rs"]`) — estimated ~120 lines deduplicated:

1. `empty_root() -> TempDir` — identical in P1/P2 (P3/P4 inline `TempDir::new`).
2. `session_on(root)` hermetic session (explicit temp `index_path`, `use_embed=false`) — P1/P2 identical save db name; P4 adds `db` param (the right generalization); P3's `pair()` is the bilateral variant.
3. `materialize(session)` (`index_status` to create empty schema) — P1/P2 identical; P3's `materialize_both` is the bilateral variant.
4. Reason constants: `CALL_NOW_ONLY` in P1/P2/P4; `BUSY` in P2/P4; `BUDGET` and `DEFS_NEEDS_SYMBOL` only in P4 (move all to testkit so reason pins stay in sync).
5. `assert_byte_identical` — duplicated in P3/P4.
6. Contention hammer loops — three variants (P1 8×25, P2 16×200×rounds, P4 8×50×rounds); P2's non-vacuous loop subsumes P1's; P4's adds success counting.
7. Budget-fill loops — P3 (10k×2 sides) and P4 (9999 + cap pin); the two together dominate suite runtime.
8. `write(root, name, content)` fixture helper — P3 only; P1/P2/P4 inline `std::fs::write`.
9. `MAX_QUERY_CHARS = 4096` literal — P2 only, with a comment admitting core is not a dev-dep; testkit is the place to centralize (or add the dev-dep).

## Proposed consolidated structure

Keep the 4-pass taxonomy (inventory → validation → bilateral → simulation reads
well); do not renumber files. Apply verdicts:

- `ffi_bilateral_pass1.rs`: delete T14; delete T9/T10/T11 (covered by P2T5/P2T7/P2T6 — or keep T9's 7-tool loop as the readable contract statement and mark P2T5 the exhaustive extension; T10/T11 are strictly subsumed); absorb P4T8 as a second window case in T6. 14 → 10 tests.
- `ffi_bilateral_pass2.rs`: unchanged (11→12 all KEEP; it is the canonical validation layer the merges point at).
- `ffi_bilateral_pass3.rs`: merge T4+T5 into T3 (`symbol_capsules_are_byte_identical` loop); merge T8 into T7 (`catalog_tools_are_byte_identical`). 14 → 12 tests.
- `ffi_bilateral_pass4.rs`: move T8's window case to P1T6. 9 → 8 tests.
- New `tests/codemode/ffi_testkit.rs` (shared-only, no `#[test]`): items 1–5 + 8–9 above; P1/P2/P4 `session_on` unify on the `(root, db)` shape.
- Net: 49 → 42 tests, ~120 helper lines deduplicated, zero coverage loss. The
  only true deletion is the tautological struct-field test (P1T14).
- Optional (not recommended now): unifying the P3/P4 budget-fill loops would
  cut wall time but couples the bilateral and simulation passes; leave split.

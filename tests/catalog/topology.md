# Topology test catalog (feature-topology intent map)

Scope: all 8 files `tests/cli/topology_pass{1,2,3,4}.rs`,
`tests/core/topology_pass{1,2,3,4}.rs`. Read-only audit; this file is the only
write. No commits.

Totals: **84 tests** — KEEP 39, MERGE 40, DELETE 5.
CLI: 43 tests (KEEP 16, MERGE 23, DELETE 4).
Core: 41 tests (KEEP 23, MERGE 17, DELETE 1).

Categories: `default-surface` (default-build contract), `matrix-cell`
(one cfg cell of the neural x rerank matrix), `delta` (cross-set relation),
`topology-drill` (end-to-end flow drill), `other`.
Kills: mutant class the test kills, `BEHAVIOR-ONLY` (pins behavior, weak
mutant), or `TAUTOLOGY-RISK` (cannot fail without editing itself).
Verdict: `KEEP`, `MERGE→target`, `DELETE (reason)`.

Adversarial rule applied: a looser pin subsumed by a strictly stronger pin
(exact counts vs non-empty, deep-equal vs shape, ungated vs cell-gated same
body) is MERGE, not KEEP. Per-cell boilerplate with identical bodies merges
into per-surface intents.

## tests/cli/topology_pass1.rs — T1 default surface (12 tests)

| # | test | cfg | intent | category | kills | verdict |
|---|---|---|---|---|---|---|
| 1 | version_json_shape | — | version --json envelope shape fields | default-surface | BEHAVIOR-ONLY | MERGE→T3 version_envelope_values (exact keys+values subsume) |
| 2 | version_text_and_clap_version_shapes | — | text version 3 lines, --version 1 line | default-surface | BEHAVIOR-ONLY | MERGE→T3 version (line-exact cross-check) + T4 clap exact string |
| 3 | help_exits_zero_and_unknown_flag_is_usage_error | — | help exit 0, unknown flag exit 1 discriminator | default-surface | BEHAVIOR-ONLY | MERGE→T3 exit_taxonomy (unknown-flag duplicated there verbatim) |
| 4 | default_search_runs_offline_on_local_embeddings | — | default search hits + status semantic-v2 stamp | default-surface | gate-inversion, stamp-regression | MERGE→T3 default_flows_exact (exact counts subsume non-empty pins) |
| 5 | semantic_only_search_runs_offline | — | --semantic-only and semantic channel succeed | default-surface | BEHAVIOR-ONLY | MERGE→T3 semantic_channel_equivalence (deep-equal subsumes both) |
| 6 | rerank_flag_fails_closed_on_default_build | not(rerank) | all 5 rerank entries reject exit 2 operational | default-surface | gate-inversion, entry-point-leak | KEEP (load-bearing 5-entry negative guard; absorb T3 key-identity atoms) |
| 7 | rerank_flag_accepted_with_feature | rerank | search --rerank accepted exit 0 | matrix-cell | gate-inversion | MERGE→T3 rerank_degrade (accept + hits-equality subsume) |
| 8 | neural_embed_search_fails_closed_on_default_build | not(neural-embed) | search --neural-embed rejects exit 2 | default-surface | gate-inversion | KEEP (merge target: spans both not(neural) cells in one test) |
| 9 | neural_embed_with_no_embed_still_succeeds | — | --neural-embed + --no-embed succeeds everywhere | delta | gate-ordering | MERGE→T3 neural_ignored_when_embed_off (3-way deep-equal subsumes) |
| 10 | rerank_top_k_alone_is_accepted_noop | — | --rerank-top-k alone exit 0 | default-surface | BEHAVIOR-ONLY | MERGE→T3 default_flows_exact (deep-equal inert proof subsumes) |
| 11 | index_neural_embed_degrades_to_local_on_default_build | not(neural-embed) | index --neural-embed degrades exit 0 semantic-v2 | default-surface | gate-inversion, stamp-regression | KEEP (only index-side degrade pin; search/index asymmetry contract) |
| 12 | capabilities_advertises_tuning_surface | — | capabilities lists 5 flags + 3 env subset | default-surface | BEHAVIOR-ONLY | MERGE→T3 capabilities_exact (full lists subsume subset) |

## tests/cli/topology_pass2.rs — T2 2x2 matrix (11 tests)

| # | test | cfg | intent | category | kills | verdict |
|---|---|---|---|---|---|---|
| 1 | default_exit_taxonomy_missing_root_and_missing_query | not(any) | missing root exit 2, missing query exit 1 | matrix-cell | exit-code-swap | MERGE→T3 exit_taxonomy (ungated strict superset) |
| 2 | default_index_dry_run_reports_plan_without_writing | not(any) | dry-run plan shape + no index file written | matrix-cell | BEHAVIOR-ONLY | MERGE→T3 index_dry_run_and_outline_exact (exact counts subsume) |
| 3 | default_doctor_healthy_envelope | not(any) | doctor healthy envelope on fresh index | matrix-cell | BEHAVIOR-ONLY | MERGE→T4 default-cell drill doctor stage (identical assertions) |
| 4 | default_lib_validation_rejects_optional_paths | not(any) | lib truth row (ok,err,err,err) | matrix-cell | gate-inversion | MERGE→core T2 matrix_joint_gate_pair (one home for lib truth table) |
| 5 | nodefault_version_envelope_key_set_matches_default | not(any) | version envelope exact 8-key set | matrix-cell | BEHAVIOR-ONLY | DELETE (byte-identical assertion exists ungated in T3 version test) |
| 6 | neural_only_lib_validation_accepts_neural_rejects_rerank | neural-only | lib truth row (ok,ok,err,err) | matrix-cell | gate-inversion | MERGE→core T2 matrix_joint_gate_pair |
| 7 | neural_only_flag_parsed_rerank_closed_default_search_offline | neural-only | 4-in-1: dry-run parse x2, rerank closed, search offline | matrix-cell | gate-inversion, leak-across-sets | DELETE (every atom subsumed: dry-run→T4 deep-equal, rerank→T1 not(rerank), search→T3 ungated) |
| 8 | rerank_only_accept_entry_points_offline | rerank-only | keyword/bare/env rerank accept exit 0 | matrix-cell | gate-inversion | MERGE→T3 rerank_degrade (extend with bare+env accept atoms) |
| 9 | rerank_only_neural_still_fails_closed | rerank-only | neural closed at binary + lib under rerank-only | matrix-cell | leak-across-sets | DELETE (binary atoms in T1 not(neural) + T3 rerank-cell; lib atom in core T2 joint) |
| 10 | all_features_lib_validation_accepts_both | all | lib truth row all-ok | matrix-cell | gate-inversion | MERGE→core T2 matrix_joint_gate_pair |
| 11 | all_features_offline_surface_without_model_load | all | 4-in-1: search offline, keyword rerank, dry-run x2 | matrix-cell | BEHAVIOR-ONLY | DELETE (atoms subsumed: search→T3 ungated, keyword→T3 degrade, dry-run→T4 deep-equal) |

## tests/cli/topology_pass3.rs — T3 cross-set deltas (12 tests)

| # | test | cfg | intent | category | kills | verdict |
|---|---|---|---|---|---|---|
| 1 | version_envelope_values_identical_across_sets | — | version exact keys+values + JSON-text cross-check | delta | BEHAVIOR-ONLY | KEEP (target: subsumes T1 version x2, T2 nodefault) |
| 2 | capabilities_exact_surface_identical_across_sets | — | tuning + env lists element-exact | delta | BEHAVIOR-ONLY | KEEP (target: subsumes T1 capabilities) |
| 3 | help_feature_markers_pinned_in_every_build | — | search-help marker counts + top contains | delta | BEHAVIOR-ONLY | KEEP (only search-help count pin; note split with T4 other-subcommand counts) |
| 4 | default_flows_exact_and_topk_inert_equivalence | — | index/status/search exact + top-k deep-equal | delta | gate-inversion, knob-leak | KEEP (target: subsumes T1 default search + top-k noop) |
| 5 | neural_ignored_when_embed_off_equivalence | — | 3-way deep-equal with embed off | delta | gate-ordering | KEEP (target: subsumes T1 no-embed test) |
| 6 | semantic_channel_equivalence | — | semantic channel vs search --semantic-only deep-equal | delta | channel-divergence | KEEP (target: subsumes T1 semantic test) |
| 7 | index_dry_run_and_outline_exact_across_sets | — | dry-run exact + outline determinism bytes | delta | BEHAVIOR-ONLY | KEEP (target: subsumes T2 dry-run) |
| 8 | exit_taxonomy_and_parse_layer_invariant | — | parse layer feature-independent, usage envelopes key-identical | delta | exit-code-swap, parse-gate-confusion | KEEP (target: subsumes T1 help discriminator + T2 taxonomy) |
| 9 | default_cell_both_rejections_share_operational_envelope | not(any) | both rejections key-identical to missing-root envelope | delta | envelope-shape-leak | MERGE→2 consolidated not(feature) envelope-identity tests (with #10 #11) |
| 10 | neural_cell_rerank_gate_unchanged_no_leak | neural-only | rerank rejections unchanged + key-identical under neural | delta | leak-across-sets | MERGE→same 2-test consolidation (rerank half spans not(rerank)) |
| 11 | rerank_cell_neural_gate_unchanged_no_leak | rerank-only | neural rejections unchanged + key-identical under rerank | delta | leak-across-sets | MERGE→same 2-test consolidation (neural half spans not(neural)) |
| 12 | rerank_degrade_preserves_local_hits | rerank | degrade path deep-equals plain results | delta | leak-across-sets | KEEP (extend with T2 bare+env accept atoms; absorbs T1 rerank-accept) |

## tests/cli/topology_pass4.rs — T4 flow drills (8 tests)

| # | test | cfg | intent | category | kills | verdict |
|---|---|---|---|---|---|---|
| 1 | default_cell_full_flow_and_combined_misuse | not(any) | 2-file transcript + combined misuse all reject | topology-drill | gate-inversion, conjunction-error | KEEP (unique: combined-flag + semantic --rerank reject) |
| 2 | neural_cell_full_flow_and_rerank_misuse | neural-only | transcript + combined rejects on rerank half + dry-run equal | topology-drill | gate-inversion, leak-across-sets | KEEP (unique: rerank-half rejection + neural dry-run deep-equal) |
| 3 | rerank_cell_full_flow_and_neural_misuse | rerank-only | transcript + neural-half misuse + semantic order marker | topology-drill | gate-inversion, order-regression | KEEP (unique: no-embed deep-equal + [alpha,beta,delta] marker) |
| 4 | all_features_cell_full_flow_and_guarded_use | all | transcript + guarded use + order marker + dry-run equal | topology-drill | gate-inversion | KEEP (unique: conjunction accept-side) |
| 5 | non_neural_cells_help_marker_surface | not(neural-embed) | help census via shared fn | topology-drill | BEHAVIOR-ONLY | MERGE→one ungated help census (pair bodies identical; split is ceremony) |
| 6 | neural_cells_help_marker_surface | neural-embed | help census via shared fn | topology-drill | BEHAVIOR-ONLY | MERGE→same ungated help census |
| 7 | non_rerank_cells_machine_surface | not(rerank) | capabilities census via shared fn | topology-drill | BEHAVIOR-ONLY | MERGE→one ungated machine census (pair bodies identical) |
| 8 | rerank_cells_machine_surface | rerank | capabilities census via shared fn | topology-drill | BEHAVIOR-ONLY | MERGE→same ungated machine census |

## tests/core/topology_pass1.rs — T1 default surface (12 tests)

| # | test | cfg | intent | category | kills | verdict |
|---|---|---|---|---|---|---|
| 1 | default_options_request_no_optional_backends | — | plain options map to Auto/Semantic, never neural | default-surface | default-flip | KEEP (only option-constructor semantic pin) |
| 2 | validate_flags_accept_plain_search_under_every_feature_set | — | plain + embed-off-neural validate Ok everywhere | default-surface | gate-inversion | MERGE→T2 joint (add plain row) + T3 neural-inert (inertness half already there) |
| 3 | neural_request_fails_closed_without_feature | — (cfg! branch) | neural verdict equals feature presence | default-surface | gate-inversion | MERGE→T2 matrix_joint_gate_pair (pair+conjunction+discriminants subsume) |
| 4 | rerank_request_fails_closed_without_feature | — (cfg! branch) | rerank verdict equals feature presence | default-surface | gate-inversion | MERGE→T2 matrix_joint_gate_pair |
| 5 | searcher_new_enforces_feature_gates | — (cfg! branch) | construction verdicts match features | default-surface | construction-gate-inversion | MERGE→T2 joint + T3 construction-equivalence composition |
| 6 | hashed_embed_path_is_deterministic_and_offline | — | hashed dim/cost/determinism/discrimination | default-surface | BEHAVIOR-ONLY | MERGE→T3 delta_local_embed_vectors (fold model_id/cost_hint atoms; rest subsumed) |
| 7 | neural_embedder_absent_without_feature | — (inner cfg) | embedder_for(Neural) is None OFF-side | default-surface | gate-inversion | KEEP (only direct OFF-side None pin) |
| 8 | embed_chain_backend_resolution | — (inner cfg) | chain resolves Semantic; Neural/Auto fall back OFF-side | default-surface | fallback-inversion | MERGE→T3 offline_resolution + T3 default-cell exact fallback (exact vectors subsume) |
| 9 | neural_config_and_backend_parsing_surface | — | model table, configured id, cache dir, parse round-trips | other | BEHAVIOR-ONLY | KEEP (cheap pure-data contract surface) |
| 10 | semantic_dim_and_embed_query_contract | — | SEMANTIC_DIM=256 + ok paths + 4 error paths | default-surface | error-omission | KEEP (only cloud/ollama/dim-mismatch rejection pin) |
| 11 | unavailable_non_hashed_embed_pins_neural_gap | — (inner cfg) | missing-backend report iff neural unavailable | default-surface | report-inversion | KEEP (only unavailable_non_hashed_embed pin) |
| 12 | rerank_off_search_ignores_rerank_knobs | — | rerank_top_k byte-inert when rerank off | default-surface | knob-leak | KEEP (only core-level knob-inertness pin; CLI envelope pin is coarser) |

## tests/core/topology_pass2.rs — T2 gate matrix (10 tests)

| # | test | cfg | intent | category | kills | verdict |
|---|---|---|---|---|---|---|
| 1 | matrix_joint_gate_pair_matches_feature_set | — (cfg!) | joint pair + conjunction + discriminants, every set | matrix-cell | gate-inversion, conjunction-error | KEEP (target for all lib-validation rows incl. CLI T2 copies) |
| 2 | matrix_cells_mutually_exclusive | — (cfg!) | exactly one of 4 cfg cells active | other | TAUTOLOGY-RISK | DELETE (partition holds by construction in every build; cannot fail unless edited) |
| 3 | cell_default_both_gates_fail_closed | not(any) | both gates err + stored neural unresolvable | matrix-cell | gate-inversion | MERGE→joint (gates) + T4 default cell (stored-neural superset) |
| 4 | cell_no_default_features_equivalent_to_default | not(any) | Searcher::new verdict equals validate for all 4 option sets | matrix-cell | construction-gate-divergence | MERGE→T3 delta_searcher_construction (identical assertion ungated) |
| 5 | cell_neural_only_gate_pair | neural-only | neural ok, rerank+both err at validate + construct | matrix-cell | gate-inversion | MERGE→joint (validate half) + T3 construction-equivalence (construct half) |
| 6 | cell_neural_only_api_presence_without_load | neural-only | size_of NeuralEmbedder, from_env well-formed, semantic intact | matrix-cell | BEHAVIOR-ONLY | MERGE→T3 deltas (fold from_env into offline_resolution; DROP size_of atom, presence proven by compile) |
| 7 | cell_rerank_only_gate_pair | rerank-only | rerank ok, neural+both err at validate + construct | matrix-cell | gate-inversion | MERGE→joint + T3 construction-equivalence |
| 8 | cell_rerank_only_api_offline | rerank-only | empty-docs rerank ok + RerankScore shape | matrix-cell | BEHAVIOR-ONLY | MERGE→T3 rerank-only cell (superset); DROP RerankScore literal atom (tautology: asserts its own literal) |
| 9 | cell_all_features_both_gates_open | all | all 4 option sets validate + construct ok | matrix-cell | gate-inversion | MERGE→joint + T3 construction-equivalence |
| 10 | cell_all_features_apis_present_without_load | all | NeuralEmbedder size, from_env, empty rerank, semantic intact | matrix-cell | BEHAVIOR-ONLY | MERGE→T3 deltas + T3 all-features cell (drop size_of atom) |

## tests/core/topology_pass3.rs — T3 deltas (12 tests)

| # | test | cfg | intent | category | kills | verdict |
|---|---|---|---|---|---|---|
| 1 | delta_defs_search_golden_identical_across_sets | — | defs golden bit-identical every set | delta | ranking-perturbation | KEEP (single-file corpus golden; values coincide with T4 rows = IDF tripwire) |
| 2 | delta_callers_search_golden_identical_across_sets | — | callers golden bit-identical every set | delta | ranking-perturbation | KEEP (same) |
| 3 | delta_hybrid_search_golden_identical_across_sets | — | hybrid golden bit-identical + embed-off purity | delta | ranking-perturbation, embed-leak | KEEP (same) |
| 4 | delta_neural_flag_inert_at_search_level_when_embed_off | — | neural flag moves zero search bytes, every set | delta | gate-ordering | KEEP (only search-bytes inertness pin) |
| 5 | delta_local_embed_vectors_bit_identical | — | 5 entry points x 4 inputs exact-vector agreement | delta | vector-divergence | KEEP (target: absorbs T1 hashed/chain pins) |
| 6 | delta_option_normalization_identical_across_sets | — | limit/top-k/context clamps identical every set | delta | clamp-divergence | KEEP (only clamp contract pin) |
| 7 | delta_searcher_construction_matches_validation_in_every_cell | — | construction equals validate every cell incl. conjunction | delta | construction-gate-divergence | KEEP (target: absorbs T2 no-default + all per-cell construct halves) |
| 8 | delta_offline_resolution_surface_stable | — | Auto-unconfigured local + model-id surface exact | delta | resolution-inversion | KEEP (only Auto-unconfigured + model-id pin) |
| 9 | cell_default_neural_fallback_exact_and_alias_rejected | not(any) | Neural exact fallback + batch + fastembed reject | delta | fallback-inversion | KEEP (only exact-fallback + batch + fastembed pin) |
| 10 | cell_neural_only_request_path_runs_local_on_rowless_corpus | neural-only | neural RUN offline rowless, goldens intact | delta | load-on-rowless, ranking-perturbation | KEEP (single-file RUN vs T4 multi-file RUN; distinct corpus contract) |
| 11 | cell_rerank_only_empty_shortlist_request_path_runs_offline | rerank-only | rerank RUN on proven-empty + empty-docs determinism | delta | load-on-empty, nondeterminism | MERGE→T4 rerank cell (fold direct empty-docs determinism atoms; search atoms subsumed by full-response equality) |
| 12 | cell_all_features_conjunction_request_paths_run_offline | all | conjunction RUN offline + no-leak both directions | delta | load-on-empty, leak-across-sets | MERGE→T4 all-features cell (fold direct-rerank atoms; rest subsumed by battery + full-response equality) |

## tests/core/topology_pass4.rs — T4 drills (7 tests)

| # | test | cfg | intent | category | kills | verdict |
|---|---|---|---|---|---|---|
| 1 | drill_full_flow_battery_golden_identical_across_sets | — | 12-row battery golden identical every set | topology-drill | ranking-perturbation, entry-divergence | KEEP (top consolidation target) |
| 2 | drill_fuse_stage_explicit_rrf_deterministic_across_sets | — | RRF exact math + weighted laws + fused order golden | topology-drill | fuse-regression, nondeterminism | KEEP (only fuse API pin) |
| 3 | drill_mixed_flow_local_unaffected_by_gated_requests | — (inner cfg probe) | interleave gated constructs; local battery before==after==golden | topology-drill | index-poisoning, gate-inversion | KEEP (only sequencing pin; absorbs T4 default-cell construction atoms) |
| 4 | cell_default_full_flow_misuse_fails_closed | not(any) | all gated fail closed + stored matrix + post-failure battery | topology-drill | gate-inversion, index-poisoning | MERGE→drill_mixed_flow (expand its cfg-gated probe to the 2x2 stored matrix; rest duplicates the drill under default) |
| 5 | cell_neural_only_full_flow_neural_runs_rerank_closed | neural-only | neural-open battery RUN + semantic empty + rerank closed | topology-drill | load-on-rowless, gate-inversion | KEEP (only neural-open full-battery RUN) |
| 6 | cell_rerank_only_full_flow_rerank_runs_neural_closed | rerank-only | rerank RUN full-response equality + literal entry + neural closed | topology-drill | load-on-empty, gate-inversion | KEEP (target: absorbs T3 rerank search atoms) |
| 7 | cell_all_features_full_flow_conjunction_runs_offline | all | all flows open + battery + conjunction equality + semantic empty | topology-drill | load-on-empty, gate-inversion | KEEP (target: absorbs T3 all-features search atoms) |

## Repeated helper patterns (testkit candidates)

CLI files (4 copies each, drifting):

- `asgrep_bin` + `run` + `run_json`: identical in T1/T2; T3/T4 add ASGREP_*
  scrub + HF_HUB_OFFLINE. Candidate: `testkit::cli::{run, run_json}`
  (always hermetic), migrate all 4 files.
- `Fixture::indexed` (1-file caller/callee a.rs): identical in T1/T2/T3;
  T4 has 2-file `build`. Candidate: `testkit::cli::Fixture::{one_file,
  two_file}`.
- One-off flow helpers that belong with the runner: `missing_root_envelope`
  (T3), `assert_rejected`, `assert_default_flow`, `assert_help_marker_surface`,
  `assert_machine_surface` (T4).

Core files (duplicated, and T1/T2 use env-sensitive `..Default`):

- Options builders: `plain_options` (T1/T2, inherits ambient ASGREP_* env)
  vs `hermetic_local_options` (T3/T4, field-by-field). Candidate:
  `testkit::core::hermetic_options`; migrate T1/T2 off `..Default`.
- `neural_req` / `rerank_req` / `both_req`: duplicated in T2/T3/T4
  (T3/T4 identical). Candidate: `testkit::core::gated_requests`.
- `indexed_corpus` single-file auth.rs: identical in T1/T2/T3; T4 two-file
  variant. Candidate: `testkit::core::{corpus_1file, corpus_2file}`.
- `HitId` + `hit_ids` + `active_cell`: near-identical in T3/T4; goldens
  share score_bits values. Candidate: `testkit::core::{HitId,
  active_cell}` + shared golden constructors (single source for the
  coinciding defs/callers/hybrid values).
- `assert_other_discriminant` (T2), `with_paths` closures (T2/T3/T4):
  fold into `testkit::core::searcher_for` (already in T3/T4; migrate T2)
  + a shared discriminant assert.

## Proposed consolidated structure

Per-surface files replace per-pass files. Passes T1-T4 become sections, not
files; total tests drop ~84 to ~45 with zero intent lost (every DELETE/MERGE
above names its absorbing target).

- `tests/cli/topology_gates.rs`: rerank 5-entry fail-closed (T1#6) +
  neural fail-closed (T1#8) + 2 not(feature) envelope-identity tests
  (from T3#9-11) + index degrade (T1#11) + combined misuse atoms (from
  T4 drills, kept per-cell only where the verdict differs).
- `tests/cli/topology_identity.rs`: all ungated exact/equivalence pins
  (T3#1-8 + T4 help/machine censuses ungated + T1 help-exit-0 atom).
- `tests/cli/topology_flows.rs`: 4 T4 cell drills (unique misuse/order
  atoms) + rerank degrade accept-side (T3#12 extended with T2 bare+env).
- `tests/core/topology_gates.rs`: options semantics (T1#1,11) + joint
  gate pair (T2#1, add plain row from T1#2) + construction equivalence
  (T3#7) + embedder_for None (T1#7).
- `tests/core/topology_local.rs`: vector agreement (T3#5 + T1#6 atoms) +
  fallback exact (T3#9) + normalization (T3#6) + resolution (T3#8) +
  embed_query contract (T1#10) + parsing surface (T1#9) + knob inertness
  (T1#12) + neural search-bytes inertness (T3#4).
- `tests/core/topology_runs.rs`: single-file goldens (T3#1-3) + battery
  (T4#1) + fuse (T4#2) + mixed flow (T4#3, expanded probe) +
  neural/rerank/conjunction RUN cells (T3#10 absorbed into T4#5-7 RUN
  coverage; T3#11-12 direct-rerank atoms folded into T4#6-7).
- `testkit` additions as listed above; delete the 5 DELETE tests outright
  (4 CLI T2 redundancies + 1 core tautology).

## Top candidates

DELETE (5, highest confidence first):

1. `core T2 matrix_cells_mutually_exclusive` — tautology; partition true by
   construction, cannot fail unless edited.
2. `cli T2 nodefault_version_envelope_key_set_matches_default` — byte-identical
   assertion exists ungated in T3.
3. `cli T2 neural_only_flag_parsed_rerank_closed_default_search_offline` —
   all 4 atoms subsumed stronger (T4 deep-equal, T1, T3 ungated).
4. `cli T2 rerank_only_neural_still_fails_closed` — binary atoms in T1+T3,
   lib atom in core T2 joint.
5. `cli T2 all_features_offline_surface_without_model_load` — all atoms
   subsumed (T3 ungated/degrade, T4 deep-equal).

MERGE (largest bundles):

1. CLI T1's 9 looser pins → T3 exact/equivalence counterparts (same file
   count, strictly stronger assertions already written).
2. CLI T2's 4 lib-validation rows → core T2 joint gate test (lib truth
   table gets one home; kills the CLI/core duplication).
3. CLI T3 envelope-identity triple → 2 not(feature)-spanning tests
   (wider coverage, fewer tests).
4. CLI T4 negative-guard pairs (2x2) → 2 ungated tests (identical bodies;
   the cfg split proves nothing the ungated run does not).
5. Core T1 single-gate pins (4 tests) → T2 joint + T3 deltas.
6. Core T2 per-cell gate/API sextet → joint + construction-equivalence +
   T3/T4 RUN cells (drop size_of + RerankScore-literal tautological atoms).

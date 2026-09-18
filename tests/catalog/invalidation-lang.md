# invalidation-lang catalog (lang cache-invalidation surface)

Source: `tests/lang/invalidation_pass1.rs` (I1 reuse-is-unobservable), `pass2.rs` (I2 per-key deltas),
`pass3.rs` (I3 rebuild-parity relations), `pass4.rs` (I4 e2e cache-consistency drills).
Contract under test: seven append-only memo/reuse caches (thread-local parser maps in
`extract`/`templates`, process-wide `SUPPORTED` gate memo + compiled-`Query` cache, per-thread
general/literal/if-cond template maps) with NO invalidation API — every cache is a pure function
of its key, pinned through the public API only. Discriminants are lines/names/verdicts/rows/scores,
never message text.
Consolidation rule (strict): KEEP only standalone per-cache contract intents; every per-key /
per-order / per-schedule variant MERGEs into its cache's contract test.

## Pass 1 — I1 reuse-is-unobservable

- parser_reuse_returns_identical_extraction: INTENT=sequential same-source parses return identical extraction with exact symbols; CAT=cache-contract; KILLS=stale-parser-reuse / parse-nondeterminism; VERDICT=KEEP
- interleaved_sources_do_not_leak_symbols: INTENT=A-B-A parses keep per-source symbol sets isolated; CAT=cache-contract; KILLS=cross-source-symbol-leak; VERDICT=MERGE→parser_reuse_returns_identical_extraction
- interleaved_languages_do_not_pollute_parsers: INTENT=Rust-Python-Rust parses keep per-language parsers isolated; CAT=cache-contract; KILLS=cross-language-parser-poisoning; VERDICT=MERGE→parser_reuse_returns_identical_extraction
- match_pattern_tracks_source_mutations: INTENT=same pattern follows current source across v1→v2→v1 with exact line sets; CAT=cache-contract; KILLS=stale-parse-reuse-on-source-mutation; VERDICT=KEEP
- match_pattern_repeatable_across_interleaved_patterns: INTENT=repeat query identical after interleaved literal+structural queries; CAT=cache-contract; KILLS=cross-template-slot-poisoning; VERDICT=MERGE→general_template_keys_are_pattern_isolated
- same_pattern_text_is_keyed_by_language: INTENT=identical pattern text answers per-language correctly under both population orders incl fresh thread; CAT=cache-contract; KILLS=(language,pattern)-key-collapse; VERDICT=MERGE→literal_keys_are_case_isolated
- fallback_gate_memo_is_stable_under_repetition: INTENT=SUPPORTED gate verdicts stable under repetition with interleaved patterns; CAT=cache-contract; KILLS=gate-memo-verdict-flip; VERDICT=MERGE→supported_gate_holds_per_pattern_verdicts
- signature_helpers_are_pure_across_repetition: INTENT=signature rows + index verdicts pure across repetition with shape discriminants; CAT=cache-contract; KILLS=signature-memo-row-drift; VERDICT=MERGE→signature_keys_are_per_pattern_stable_under_growth
- native_answerability_stable_across_languages: INTENT=answerability verdicts stable under interleaved cross-language consults; CAT=cache-contract; KILLS=answerability-verdict-flip; VERDICT=MERGE→native_answerability_is_per_language_keyed

## Pass 2 — I2 per-key deltas

- parser_keys_are_per_language_under_growth: INTENT=per-language parser keys isolated and stable after growth to further languages; CAT=isolation; KILLS=per-language-parser-key-collapse / growth-eviction; VERDICT=MERGE→parser_reuse_returns_identical_extraction
- literal_keys_are_case_isolated: INTENT=foo/Foo/FOO literal keys isolated under reverse order + growth; CAT=isolation; KILLS=case-folding-key-collapse; VERDICT=KEEP
- literal_keys_are_language_isolated: INTENT=(language,literal) keys isolated under poison/reverse order + growth; CAT=isolation; KILLS=(language,literal)-key-collapse; VERDICT=MERGE→literal_keys_are_case_isolated
- class_query_keys_are_keyword_isolated: INTENT=struct/interface/type query keys isolated under reverse order + growth; CAT=isolation; KILLS=keyword-index-key-merge; VERDICT=MERGE→general_template_keys_are_pattern_isolated
- class_query_keys_are_language_isolated: INTENT=(language,class-query) keys isolated with empty-slot-first poisoning + growth; CAT=isolation; KILLS=(language,class-query)-key-collapse; VERDICT=MERGE→general_template_keys_are_pattern_isolated
- supported_gate_holds_per_pattern_verdicts: INTENT=supported/unsupported per-key verdicts hold under both population orders; CAT=isolation; KILLS=supported/unsupported-verdict-swap; VERDICT=KEEP
- supported_gate_stable_under_cache_growth: INTENT=6-key verdict panel identical after 20-key growth; CAT=isolation; KILLS=growth-eviction-verdict-drift; VERDICT=MERGE→supported_gate_holds_per_pattern_verdicts
- native_answerability_is_per_language_keyed: INTENT=covered-JS vs uncovered-Swift verdicts pinned under reverse order + growth; CAT=isolation; KILLS=covered/uncovered-verdict-collapse; VERDICT=KEEP
- general_template_keys_are_pattern_isolated: INTENT=return/throw general keys isolated under reverse order + growth; CAT=isolation; KILLS=general-template-key-merge; VERDICT=KEEP
- if_cond_keys_are_cond_isolated: INTENT=alpha/beta/meta cond keys isolated under reverse order + growth; CAT=isolation; KILLS=cond-key-merge; VERDICT=KEEP
- signature_keys_are_per_pattern_stable_under_growth: INTENT=ident/decl/call/kind signature rows + serve verdicts exact and stable under growth; CAT=isolation; KILLS=signature-key-merge / growth-row-drift; VERDICT=KEEP

## Pass 3 — I3 rebuild-parity relations

- panel_order_permutations_agree_across_fresh_threads: INTENT=same pattern panel agrees under forward/reverse/rotated orders in cold threads; CAT=parity; KILLS=population-order-dependence; VERDICT=KEEP
- warmed_caches_match_fresh_thread_results: INTENT=junk-warmed thread agrees with cold thread on same panel; CAT=parity; KILLS=warmed-vs-cold-divergence; VERDICT=KEEP
- match_pattern_repeat_n_equals_once: INTENT=25 uses of general- and literal-lane keys equal 1 use; CAT=parity; KILLS=repeat-use-result-drift; VERDICT=MERGE→general_template_keys_are_pattern_isolated
- match_literal_repeat_n_equals_once: INTENT=25 uses of hit and empty literal keys equal 1 use without flipping; CAT=parity; KILLS=literal-repeat-drift / empty-flip; VERDICT=MERGE→literal_keys_are_case_isolated
- fallback_gate_repeat_n_equals_once_panel: INTENT=gate verdict panel reproduces exactly across 25 rounds; CAT=parity; KILLS=gate-repeat-use-flip; VERDICT=MERGE→supported_gate_holds_per_pattern_verdicts
- signature_helpers_repeat_n_equals_once: INTENT=signature rows + index verdicts identical across 25 rounds; CAT=parity; KILLS=signature/index-repeat-drift; VERDICT=MERGE→signature_keys_are_per_pattern_stable_under_growth
- round_robin_interleave_matches_batched_schedule: INTENT=batched vs round-robin schedules agree per (language,pattern) cell in cold threads; CAT=parity; KILLS=schedule/interleave-dependence; VERDICT=MERGE→panel_order_permutations_agree_across_fresh_threads
- parser_round_robin_matches_batched: INTENT=batched vs round-robin parser schedules agree per language cell; CAT=parity; KILLS=parser-population-order-dependence; VERDICT=MERGE→parser_reuse_returns_identical_extraction
- answerability_panel_permutation_invariant: INTENT=answerability panel agrees under 3 consult orders in cold threads; CAT=parity; KILLS=consult-order-dependence; VERDICT=MERGE→panel_order_permutations_agree_across_fresh_threads
- pairwise_commutativity_in_fresh_threads: INTENT=A-then-B equals B-then-A per key for 3 template-sharing pairs; CAT=parity; KILLS=pair-order-noncommutativity; VERDICT=MERGE→panel_order_permutations_agree_across_fresh_threads

## Pass 4 — I4 e2e cache-consistency drills

- single_lang_heavy_reuse_canonical_vs_reversed: INTENT=full detect→rank workload identical under canonical vs reversed file order; CAT=drill; KILLS=file-order-dependence; VERDICT=KEEP
- single_lang_heavy_reuse_warmed_vs_cold: INTENT=junk-saturated thread workload equals cold thread workload; CAT=drill; KILLS=warmed-vs-cold-workload-divergence; VERDICT=MERGE→warmed_caches_match_fresh_thread_results
- many_lang_batched_vs_round_robin: INTENT=polyglot workload identical under batched vs stride-2 schedule; CAT=drill; KILLS=polyglot-schedule-dependence; VERDICT=MERGE→adversarial_interleave_matches_canonical_pipeline
- many_lang_forward_vs_reverse_pipeline: INTENT=polyglot workload identical under forward vs reverse file order; CAT=drill; KILLS=polyglot-file-order-dependence; VERDICT=MERGE→adversarial_interleave_matches_canonical_pipeline
- unsupported_mix_supported_first_vs_unsupported_first: INTENT=mixed covered/uncovered workload identical under supported-first, unsupported-first, canonical orders; CAT=drill; KILLS=unsupported-first-poisoning; VERDICT=KEEP
- unsupported_mix_warmed_caches_match_cold: INTENT=unsupported-first-warmed + junk-growth workload equals cold canonical; CAT=drill; KILLS=warmed-mixed-workload-divergence; VERDICT=MERGE→warmed_caches_match_fresh_thread_results
- adversarial_interleave_matches_canonical_pipeline: INTENT=hostile lane-alternating order with per-step junk genus-poisoning equals canonical run incl ranking; CAT=drill; KILLS=hostile-schedule-/-junk-poisoning-divergence; VERDICT=KEEP
- end_to_end_pipeline_repeat_is_stable: INTENT=full workload identical across 3 rounds on one warmed thread, both corpora; CAT=drill; KILLS=TAUTOLOGY-RISK+no-hand-oracle-repetition-equality-with-only-nonempty-asserts; VERDICT=MERGE→adversarial_interleave_matches_canonical_pipeline

## Helper patterns

- `symbol_names(&ExtractionResult) -> Vec<...>` (pass1/2/3): symbol-name projection for equality + exact-name asserts; pass1 borrows `&str`, pass2/3 own `String`.
- `lines(&[PatternMatch]) -> Vec<u32>` (pass2/3): line_start projection; the dominant discriminant for hit equality.
- `run_in_fresh_thread(f)` (pass3/4; inline `thread::scope` in pass1 `same_pattern_text`): cold thread-local cache slots per order permutation; the core I3/I4 isolation primitive.
- Panel + before/after vectors (pass2 gate/signature, pass3 repeat-N): verdict/row panels captured once, re-asserted after growth/repetition/permutation.
- Reverse-order + growth + re-read triple (pass2): every per-key test populates reversed, injects fresh growth keys, then re-reads pinned keys.
- `Cell` / `FileOutcome` / `run_cell` / `run_workload` / `score` / `rank` (pass4): full detect→parse→match→gates→score→rank pipeline harness; `score` = symbols*1000 + hits*10 + fallback*2 + answerable; `rank` = score-desc/path-asc.
- `canonical_order` / `reversed_order` + hostile schedules (pass4): order vectors remapped to canonical cell order for direct outcome comparison.
- Junk warm-up loops (pass3 `warmed_*`, pass4 warmed/adversarial): unrelated languages/patterns saturate every cache genus before the pinned run.
- Constant corpora `RUST_FNS`, `RUST_PANEL`/`RUST_CORPUS`, `POLYGLOT_CORPUS`, `MIXED_CORPUS` (pass3/4): shared fixtures with hand-known lines/symbols/verdicts.

## Counts

- Total tests: 38 (pass1: 9, pass2: 11, pass3: 10, pass4: 8).
- By CAT: cache-contract 9, isolation 11, parity 10, drill 8, other 0.
- By VERDICT: KEEP 13, MERGE 25, DELETE 0.
- KEEP targets (13): parser_reuse_returns_identical_extraction, match_pattern_tracks_source_mutations, literal_keys_are_case_isolated, supported_gate_holds_per_pattern_verdicts, native_answerability_is_per_language_keyed, general_template_keys_are_pattern_isolated, if_cond_keys_are_cond_isolated, signature_keys_are_per_pattern_stable_under_growth, panel_order_permutations_agree_across_fresh_threads, warmed_caches_match_fresh_thread_results, single_lang_heavy_reuse_canonical_vs_reversed, unsupported_mix_supported_first_vs_unsupported_first, adversarial_interleave_matches_canonical_pipeline.
- Merge fan-in: parser_reuse←4, general_template←4, literal_case←3 (incl same_pattern_text language-key delta), gate_holds←3, signature_keys←2, panel_order←3, warmed_caches←2, adversarial←3, answerability←1.
- Tautology risks: 1 (end_to_end_pipeline_repeat_is_stable — repetition equality only, no hand oracle); BEHAVIOR-ONLY: 0 standalone.

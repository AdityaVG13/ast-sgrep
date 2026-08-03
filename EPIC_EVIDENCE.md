# Epic evidence — fusion normalization + search correctness (PR #22)

Branch: `fix/fusion-normalization-e2hc-14`  
Evidence date: 2026-08-03  
Primary test crate: `ast-sgrep-core` (`tests/search_correctness_epics.rs` + lib unit tests)

## Commands run

```bash
export CARGO_BUILD_RUSTC_WRAPPER= RUSTC_WRAPPER=
/usr/local/cargo/bin/cargo test -p ast-sgrep-core --test search_correctness_epics
/usr/local/cargo/bin/cargo test -p ast-sgrep-core --lib
/usr/local/cargo/bin/cargo test -p ast-sgrep-core --test literal_glob --test chain_case --test graph_oracle
```

### Observed results (2026-08-03)

| Suite | Result |
|-------|--------|
| `search_correctness_epics` | **10 passed** |
| `ast-sgrep-core --lib` (incl. intent + search unit tests) | **26 passed** |
| `literal_glob` | **3 passed** |
| `chain_case` | **1 passed** |
| `graph_oracle` | **1 passed** |

---

## Epic `ast-sgrep-s7jw` (Ranking / RRF / Tantivy integrity)

| Kid | Requirement | Implementation | Hard evidence |
|-----|-------------|----------------|---------------|
| `cbnw` | Align Asgrep `channel_ceiling` with single-list lexical RRF | `intent::channel_ceiling` uses `rrf_score(0,RRF_K)*LEXICAL_RRF_SCALE` (no `terms *` multiplier); fusion normalization from e2hc.14 retained | `cbnw_asgrep_ceiling_is_single_list_rrf`; `intent::tests::{asgrep_ceiling_does_not_scale_with_term_count, multi_term_lexical_hit_not_capped_at_inverse_terms}` |
| `hkdi` | Empty auto-created Tantivy sidecar must not be “ready” | `TantivySidecar::open_existing_for_search` + `is_search_ready` require `meta.lines > 0`; zero-byte refused | `hkdi_empty_lexical_sidecar_not_ready` |
| `s7jw.1` | Lexical pool hard LIMIT 100 must respect `options.limit` | `lexical_pool_limit = max(100, options.limit)` for FTS + sidecar | `s7jw1_lexical_pool_respects_options_limit` |
| `s7jw.2` | Auto/Tantivy path never empty-success when SQL lexical has hits | Search opens only ready sidecars; empty sidecar results fall through to SQL FTS | `s7jw2_empty_sidecar_falls_back_to_sql_lexical` |

---

## Epic `ast-sgrep-search-correctness-iva9`

| Kid | Requirement | Implementation | Hard evidence |
|-----|-------------|----------------|---------------|
| `.2` | Reject invalid `file_filter` globs (never silent unfiltered) | `finish_response` returns `Err` on invalid/empty/control-char globs | `iva9_2_invalid_file_filter_errors_via_searcher`; `search::tests::invalid_file_filter_errors_instead_of_unfiltered` |
| `.4` | Filter zero-score hits before cap/truncate | `finish_response` retains only finite `score > 0` before keep/limit | `search::tests::zero_score_hits_are_dropped_before_limit` |
| `.5` | Do not path-LIMIT literal SQL before lang filter | `literal_sql` pushes `f.language = ?` into SQL when `lang_filter` set (before `ORDER BY path LIMIT`) | `iva9_5_literal_lang_filter_not_starved_by_path_limit` |
| `.6` | Fall back from ANN to flat when empty/under-filled | `ann_result_is_sufficient`; lazy IVF returns `None` when under-filled; `rank_chunk_indices_flat` falls through to brute-force | `iva9_6_ann_sufficiency_contract` |
| `.7` | Pattern empty + exotic → no silent empty (no subprocess reintroduction) | Three-layer stack documented; exotic shapes fail-closed when `ASGREP_DISABLE_AST_GREP=1` or binary missing; classifiable native empty is authoritative match-none | `iva9_7_exotic_pattern_fail_closed_without_ast_grep`; `iva9_7_classifiable_native_empty_is_match_none` |
| `.8` | Fix rerank score/order; chain seed/edge contracts | `apply_rerank_order` writes rerank scores; chain seeds prefer `callee`/`caller`; edges re-filtered after node truncate | `search::tests::rerank_reorders_prefix_and_writes_rerank_scores`; `iva9_8_chain_edges_subset_and_callee_seed` |

### iva9.7 policy note

Production keeps optional external `ast-grep` for **exotic** `$` shapes only. Set `ASGREP_DISABLE_AST_GREP=1` for explicit no-subprocess / fail-closed mode. Classifiable native patterns do **not** reintroduce a subprocess on empty hits.

---

## Fusion normalization (this PR) remains intact

e2hc.14 / u9fj behavior preserved and re-asserted:

- Asgrep ceiling does not scale with term count
- Single-char substantive terms remain live on Hybrid
- Def/Caller ceilings use matched-term counts only

Evidence: `intent::tests::*` (5 passed) + `cbnw_asgrep_ceiling_is_single_list_rrf`.

---

## Notes

- `.beads` was **not** modified (per task instructions).
- Lexical SQL also applies lang filter before LIMIT (sibling hardening to iva9.5).

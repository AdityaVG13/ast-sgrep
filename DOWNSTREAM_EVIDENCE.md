# Downstream evidence — fusion/normalization follow-ons (PR #22)

Branch: `fix/fusion-normalization-e2hc-14`  
Evidence date: 2026-08-03  
Note: `.beads` was **not** modified.

## Commands run

```bash
export PATH="/usr/local/cargo/bin:$PATH" CARGO_BUILD_RUSTC_WRAPPER= RUSTC_WRAPPER=
cargo test -p ast-sgrep-core --lib
cargo test -p ast-sgrep-core --test downstream_correctness \
  --test search_correctness_epics --test semantic_ivf_roundtrip \
  --test parity --test chain_case --test graph_oracle \
  --test literal_glob --test resolve_module
cargo test -p ast-sgrep-cli --test surface_equivalence
cargo clippy -p ast-sgrep-core -p ast-sgrep-cli -p ast-sgrep-testkit --all-targets -- -D warnings
cargo fmt -p ast-sgrep-core -p ast-sgrep-cli -p ast-sgrep-testkit
```

### Observed results

| Suite | Result |
|-------|--------|
| `ast-sgrep-core --lib` | **32 passed** |
| `downstream_correctness` | **6 passed** |
| `search_correctness_epics` | **10 passed** |
| `semantic_ivf_roundtrip` | **3 passed** |
| `parity` | **5 passed** |
| `chain_case` / `graph_oracle` / `literal_glob` / `resolve_module` | **all passed** |
| `surface_equivalence` | **2 passed** |

---

## Bead closure matrix

| Bead | Requirement | Implementation | Hard evidence |
|------|-------------|----------------|---------------|
| `2hhq` | Chain drops edges to truncated-out nodes | Already on tip (iva9.8); retained `all_edges.retain` after truncate | `bead_2hhq_chain_drops_edges_to_truncated_nodes`; `iva9_8_chain_edges_subset_and_callee_seed` |
| `50hx` | Hybrid Literal intent runs `literal_pass` | `hybrid_literal_parsed` + literal pass in serial/parallel hybrid | `bead_50hx_hybrid_quoted_runs_literal_pass` |
| `8mb8` | Pre-truncate keeps coverage in sort key | Truncate to `keep*4` with coverage in comparator (not score-only → `keep`) | `search::tests::pre_truncate_keeps_high_coverage_lower_score_hit` |
| `noik` | Structural index score inflation | `STRUCTURAL_INDEX_FRACTION=0.35`; documented score units | `search::tests::structural_index_score_bounded_after_fusion` |
| `firi` | Unify IVF vs flat `MIN_SIMILARITY` | IVF `score_members` uses `top_k_similarity(..., Some(MIN_SIMILARITY))` | `bead_firi_ivf_and_flat_min_similarity_agree`; CE-003 IVF≡flat |
| `ql1u` | CLI chain top_n; no invent `first_symbol_in_file` | CLI `top_n` from `--limit`/default; `hit_symbol` returns `None` instead of inventing | `bead_ql1u_chain_seed_skips_first_symbol_invention`; CLI `run_chain` |
| `eh5a` | `mode_query` must not lowercase regex/literal | Literal/Regex terms preserve case | `query::tests::literal_and_regex_terms_preserve_case` |
| `54if` | Unify `ParsedQuery.raw` prefix keep | All prefixed modes keep full query in `raw` | `query::tests::raw_keeps_mode_prefix_across_all_modes` |
| `hhca` | `excerpt_term_coverage` case policy | Lowercases both excerpt and terms | `search::tests::excerpt_coverage_lowercases_terms_and_excerpt` |
| `vwga` | Wire `ranking/cases.json` CI self-oracle | Integration oracle; hybrid ranks among same-kind | `bead_vwga_ranking_cases_json_self_oracle` |
| `am6l` | Hoist query-term normalization | `normalize_query_terms` + `*_normalized` scorers; used in `caller_rows_to_hits` | `rank::tests::normalized_term_apis_match_public_scorers`; bench `coverage_symbol_score` |
| `6dx9` | Differential parallel-128 / tantivy-1000 | Both threshold sides exercised; tantivy force HitKey set ≡ FTS | `bead_6dx9_threshold_sides_differentially_exercised` |
| `x1p5` | Expand surface_equivalence multi-mode + both-error | Multi-mode sorted rich HitKeys; both-error invalid regex | `surface_equivalence_multi_mode_hit_keys`; `surface_equivalence_both_error_table` |
| `tius` | Edge-case fall-through audit + real fixes | Audit doc + fixes for silent-empty sites found | `docs/EDGE_CASE_FALLTHROUGH_AUDIT.md` + 50hx/eh5a/firi/ql1u |

---

## Policy notes

- **`ParsedQuery.raw` (54if):** always the full user string including mode prefix when parsed from prefixed input. Constructors `literal()`/`regex()`/`word()` keep payload-only `raw` (no synthetic prefix).
- **Structural index (noik):** raw score = `SCORE_PATTERN * 0.35`; after `route_hits` ≤ `0.35 * pattern_weight`.
- **MIN_SIMILARITY (firi):** both IVF member scoring and flat paths use `exceeds_threshold` via `Some(MIN_SIMILARITY)`.
- **Chain seeds (ql1u):** never invent via `first_symbol_in_file`; imports expansion may still label resolved modules with a first symbol for edge display.

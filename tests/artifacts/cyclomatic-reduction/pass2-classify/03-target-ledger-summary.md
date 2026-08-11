# Target Ledger

Run ID: `2026-08-10T235424Z-baseline`
Target ceiling: `10` (hard) / preferred `8` / aspirational `5`
Scope: product code under `crates/` + `packages/pi/extension/src` + `packages/pi/launcher/src`
ΣCC baseline: **6022** · functions: **1927** · hotspots CC>10: **91**

**Pass 2 status:** top 30 classified (analysis cards in `04-analysis-cards/`). No product transforms yet.

## Classification tallies (top 30)

| Classification | Count |
|---|---|
| `essential_domain` | 11 |
| `accidental_structure` | 5 |
| `extractable` | 14 |
| `dead_path` | 0 |

## Hotspots (top ~30 by CC)

| Rank | File | Function | Line | CC | Cognitive | Classification | Technique | Wave | Risk | Resolve |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | `packages/pi/extension/src/runtime.ts` | `parseEnvelope` | 366 | 31 | n/a | `essential_domain` | `extract_method` | 7 (optional error-path extract) | high | Keep |
| 2 | `crates/ast-sgrep-cli/src/bench.rs` | `run_bench_suite` | 231 | 29 | n/a | `extractable` | `extract_method` | 4 | high | Cut (extract) |
| 3 | `packages/pi/launcher/src/index.js` | `resolveHost` | 76 | 29 | n/a | `accidental_structure` | `guard_clause` | 3 | high | Cut |
| 4 | `crates/ast-sgrep-core/src/semantic_ivf.rs` | `read_header` | 397 | 25 | n/a | `essential_domain` | `none (Keep)` | keep | high | Keep |
| 5 | `crates/ast-sgrep-core/src/semantic_ivf.rs` | `save_semantic_ivf_with_publication` | 179 | 25 | n/a | `extractable` | `extract_method` | 4 | high | Cut (extract) |
| 6 | `packages/pi/extension/src/code-mode.ts` | `readLineWindow` | 277 | 25 | n/a | `essential_domain` | `none (Keep)` | keep | high | Keep |
| 7 | `crates/ast-sgrep-core/src/bench_suite.rs` | `measure_semantic_ivf_open_p99` | 198 | 24 | n/a | `extractable` | `extract_method` | 4 | high | Cut (extract) |
| 8 | `crates/ast-sgrep-core/src/index.rs` | `index_all` | 231 | 23 | n/a | `extractable` | `extract_method` | 4 | high | Cut (extract) |
| 9 | `packages/pi/extension/src/runtime.ts` | `ensureFresh` | 265 | 23 | n/a | `essential_domain` | `decompose_conditional` | 6 (optional) | high | Keep |
| 10 | `packages/pi/launcher/src/index.js` | `resolveCodemodeAddon` | 128 | 23 | n/a | `accidental_structure` | `guard_clause` | 3 | high | Cut |
| 11 | `crates/ast-sgrep-core/src/semantic_ann.rs` | `read_clusters_bounded` | 104 | 22 | n/a | `essential_domain` | `none (Keep)` | keep | high | Keep |
| 12 | `packages/pi/extension/src/codemode/dispatch.ts` | `argvFor` | 231 | 22 | n/a | `accidental_structure` | `lookup_table` | 5 | high | Cut |
| 13 | `packages/pi/launcher/src/index.js` | `resolveBinary` | 99 | 22 | n/a | `accidental_structure` | `early_return` | 3 | high | Cut |
| 14 | `crates/ast-sgrep-core/src/fusion.rs` | `apply_weighted_rrf` | 207 | 21 | n/a | `essential_domain` | `none (Keep)` | keep | high | Keep |
| 15 | `packages/pi/extension/src/code-mode.ts` | `parseSearchHit` | 173 | 21 | n/a | `extractable` | `extract_method` | 4 | high | Cut (extract) |
| 16 | `crates/ast-sgrep-core/src/index.rs` | `index_content_at` | 732 | 20 | n/a | `extractable` | `extract_method` | 4 | high | Cut (extract) |
| 17 | `crates/ast-sgrep-core/src/search/passes/embed.rs` | `embed_pass_lazy_ivf` | 113 | 20 | n/a | `essential_domain` | `none (Keep)` | keep | high | Keep |
| 18 | `crates/ast-sgrep-lang/src/pattern.rs` | `classify_native` | 138 | 20 | n/a | `essential_domain` | `none (Keep)` | keep | high | Keep |
| 19 | `crates/ast-sgrep-mcp/src/lib.rs` | `read_node` | 955 | 20 | n/a | `extractable` | `extract_method` | 4 | high | Cut (extract) |
| 20 | `crates/ast-sgrep-cli/src/lib.rs` | `run_codemode_batch` | 229 | 19 | n/a | `extractable` | `extract_method` | 4 | medium | Cut (extract) |
| 21 | `crates/ast-sgrep-core/src/store/sqlite.rs` | `refresh_lines_only` | 657 | 19 | n/a | `essential_domain` | `none (Keep)` | keep | medium | Keep |
| 22 | `crates/ast-sgrep-lang/src/signature.rs` | `cached_pattern_signatures` | 15 | 19 | n/a | `essential_domain` | `none (Keep)` | keep | medium | Keep |
| 23 | `crates/ast-sgrep-core/src/index.rs` | `update_paths` | 638 | 18 | n/a | `extractable` | `guard_clause` | 3 | medium | Cut (extract+guards) |
| 24 | `crates/ast-sgrep-core/src/pipeline_parts.rs` | `measure_index_update` | 231 | 18 | n/a | `extractable` | `extract_method` | 4 | medium | Cut (extract) |
| 25 | `crates/ast-sgrep-core/src/search/passes/literal.rs` | `literal_sql` | 67 | 18 | n/a | `extractable` | `lookup_table` | 5 | medium | Cut |
| 26 | `crates/ast-sgrep-core/src/search/passes/regex.rs` | `regex_pass` | 26 | 18 | n/a | `extractable` | `extract_method` | 4 | medium | Cut (extract) |
| 27 | `crates/ast-sgrep-core/src/store/sql.rs` | `delete_file_lines` | 271 | 18 | n/a | `extractable` | `extract_method` | 4 | medium | Cut (extract) |
| 28 | `crates/ast-sgrep-embed/src/embedder.rs` | `embed_url_is_allowed` | 27 | 17 | n/a | `essential_domain` | `none (Keep)` | keep | medium | Keep |
| 29 | `packages/pi/extension/src/index.ts` | `searchToolCall` | 535 | 17 | n/a | `accidental_structure` | `lookup_table` | 5 | medium | Cut |
| 30 | `crates/ast-sgrep-cli/src/bench.rs` | `run_bench_batch` | 452 | 16 | n/a | `extractable` | `extract_method` | 4 | medium | Cut (extract) |

## Classification Definitions

- **essential_domain:** Branching is inherent to the problem domain. Keep and test.
- **accidental_structure:** Branching is an artifact of poor structure. Safe to refactor.
- **dead_path:** Branch appears unreachable. Remove only with evidence.
- **extractable:** A branch block has a clear, nameable responsibility. Extract method.

## Pass wave map

| Wave | Pass # | Technique family | Cut batch (first) |
|---|---|---|---|
| Guards | 3 | guard_clause, early_return, replace_nested_cond_with_guard | resolveHost, resolveBinary, resolveCodemodeAddon, update_paths |
| Extract | 4 | extract_method | index_all, index_content_at, run_codemode_batch, delete_file_lines, parseSearchHit, read_node, … |
| Table | 5 | lookup_table | argvFor, searchToolCall, literal_sql |
| Boolean | 6 | combine_predicates, decompose_conditional | ensureFresh (optional Keep helpers only) |
| Error-path | 7 | extract_method on failure ladders | parseEnvelope optional; launcher residuals |

## Full hotspot backlog (CC>10)

Total: **91** functions. Full ranked list in `02-baseline-raw.json` key `hotspots`.
Ranks 31–91 remain `pending_classify` until a later classify sweep (not blocking pass 3).

## Exclusions (this campaign baseline)

| Path | Reason |
|---|---|
| `target/`, `node_modules/` | Not product source; measure tool cannot exclude when pointing at repo root |
| `packages/**/dist` | Build output |
| Skill / `.cyclomatic-reduction` | Meta |

## Next-Pass Queue

| Item | Why Deferred |
|---|---|
| Pass 3 guard-clause wave | First accidental batch (launcher resolve* + update_paths) |
| Pass 4 extract wave | Product extracts (index_*, delete_file_lines, …) then bench |
| Pass 5 lookup tables | argvFor, searchToolCall, literal_sql |
| Classify ranks 31–91 | After first transform waves prove protocol |
| Bench/test-only carve-out policy | Still open: whether benches stay in ΣCC gate |

# Target Ledger

Run ID: `2026-08-10T235424Z-baseline`
Target ceiling: `10` (hard) / preferred `8` / aspirational `5`
Scope: product code under `crates/` + `packages/pi/extension/src` + `packages/pi/launcher/src`
ΣCC baseline: **6022** · functions: **1927** · hotspots CC>10: **91**

## Hotspots (top ~30 by CC)

Classification deferred to **Pass 2** (classify). Risk heuristic: CC≥20 high, ≥15 medium, else low among hotspots.

| Rank | File | Function | Line | CC | Cognitive | Classification | Risk | Action |
|---|---|---|---|---|---|---|---|---|
| 1 | `packages/pi/extension/src/runtime.ts` | `parseEnvelope` | 366 | 31 | n/a | pending_pass2 | high | classify then technique |
| 2 | `crates/ast-sgrep-cli/src/bench.rs` | `run_bench_suite` | 231 | 29 | n/a | pending_pass2 | high | classify then technique |
| 3 | `packages/pi/launcher/src/index.js` | `resolveHost` | 76 | 29 | n/a | pending_pass2 | high | classify then technique |
| 4 | `crates/ast-sgrep-core/src/semantic_ivf.rs` | `read_header` | 397 | 25 | n/a | pending_pass2 | high | classify then technique |
| 5 | `crates/ast-sgrep-core/src/semantic_ivf.rs` | `save_semantic_ivf_with_publication` | 179 | 25 | n/a | pending_pass2 | high | classify then technique |
| 6 | `packages/pi/extension/src/code-mode.ts` | `readLineWindow` | 277 | 25 | n/a | pending_pass2 | high | classify then technique |
| 7 | `crates/ast-sgrep-core/src/bench_suite.rs` | `measure_semantic_ivf_open_p99` | 198 | 24 | n/a | pending_pass2 | high | classify then technique |
| 8 | `crates/ast-sgrep-core/src/index.rs` | `index_all` | 231 | 23 | n/a | pending_pass2 | high | classify then technique |
| 9 | `packages/pi/extension/src/runtime.ts` | `ensureFresh` | 265 | 23 | n/a | pending_pass2 | high | classify then technique |
| 10 | `packages/pi/launcher/src/index.js` | `resolveCodemodeAddon` | 128 | 23 | n/a | pending_pass2 | high | classify then technique |
| 11 | `crates/ast-sgrep-core/src/semantic_ann.rs` | `read_clusters_bounded` | 104 | 22 | n/a | pending_pass2 | high | classify then technique |
| 12 | `packages/pi/extension/src/codemode/dispatch.ts` | `argvFor` | 231 | 22 | n/a | pending_pass2 | high | classify then technique |
| 13 | `packages/pi/launcher/src/index.js` | `resolveBinary` | 99 | 22 | n/a | pending_pass2 | high | classify then technique |
| 14 | `crates/ast-sgrep-core/src/fusion.rs` | `apply_weighted_rrf` | 207 | 21 | n/a | pending_pass2 | high | classify then technique |
| 15 | `packages/pi/extension/src/code-mode.ts` | `parseSearchHit` | 173 | 21 | n/a | pending_pass2 | high | classify then technique |
| 16 | `crates/ast-sgrep-core/src/index.rs` | `index_content_at` | 732 | 20 | n/a | pending_pass2 | high | classify then technique |
| 17 | `crates/ast-sgrep-core/src/search/passes/embed.rs` | `embed_pass_lazy_ivf` | 113 | 20 | n/a | pending_pass2 | high | classify then technique |
| 18 | `crates/ast-sgrep-lang/src/pattern.rs` | `classify_native` | 138 | 20 | n/a | pending_pass2 | high | classify then technique |
| 19 | `crates/ast-sgrep-mcp/src/lib.rs` | `read_node` | 955 | 20 | n/a | pending_pass2 | high | classify then technique |
| 20 | `crates/ast-sgrep-cli/src/lib.rs` | `run_codemode_batch` | 229 | 19 | n/a | pending_pass2 | medium | classify then technique |
| 21 | `crates/ast-sgrep-core/src/store/sqlite.rs` | `refresh_lines_only` | 657 | 19 | n/a | pending_pass2 | medium | classify then technique |
| 22 | `crates/ast-sgrep-lang/src/signature.rs` | `cached_pattern_signatures` | 15 | 19 | n/a | pending_pass2 | medium | classify then technique |
| 23 | `crates/ast-sgrep-core/src/index.rs` | `update_paths` | 638 | 18 | n/a | pending_pass2 | medium | classify then technique |
| 24 | `crates/ast-sgrep-core/src/pipeline_parts.rs` | `measure_index_update` | 231 | 18 | n/a | pending_pass2 | medium | classify then technique |
| 25 | `crates/ast-sgrep-core/src/search/passes/literal.rs` | `literal_sql` | 67 | 18 | n/a | pending_pass2 | medium | classify then technique |
| 26 | `crates/ast-sgrep-core/src/search/passes/regex.rs` | `regex_pass` | 26 | 18 | n/a | pending_pass2 | medium | classify then technique |
| 27 | `crates/ast-sgrep-core/src/store/sql.rs` | `delete_file_lines` | 271 | 18 | n/a | pending_pass2 | medium | classify then technique |
| 28 | `crates/ast-sgrep-embed/src/embedder.rs` | `embed_url_is_allowed` | 27 | 17 | n/a | pending_pass2 | medium | classify then technique |
| 29 | `packages/pi/extension/src/index.ts` | `searchToolCall` | 535 | 17 | n/a | pending_pass2 | medium | classify then technique |
| 30 | `crates/ast-sgrep-cli/src/bench.rs` | `run_bench_batch` | 452 | 16 | n/a | pending_pass2 | medium | classify then technique |

## Classification Definitions

- **essential_domain:** Branching is inherent to the problem domain. Keep and test.
- **accidental_structure:** Branching is an artifact of poor structure. Safe to refactor.
- **dead_path:** Branch appears unreachable. Remove only with evidence.
- **extractable:** A branch block has a clear, nameable responsibility. Extract method.

## Full hotspot backlog (CC>10)

Total: **91** functions. Full ranked list in `02-baseline-raw.json` key `hotspots`.

### Rust crates (CC≥15) — quick index

| CC | Function | File:line |
|---|---|---|
| 29 | `run_bench_suite` | `crates/ast-sgrep-cli/src/bench.rs:231` |
| 25 | `read_header` | `crates/ast-sgrep-core/src/semantic_ivf.rs:397` |
| 25 | `save_semantic_ivf_with_publication` | `crates/ast-sgrep-core/src/semantic_ivf.rs:179` |
| 24 | `measure_semantic_ivf_open_p99` | `crates/ast-sgrep-core/src/bench_suite.rs:198` |
| 23 | `index_all` | `crates/ast-sgrep-core/src/index.rs:231` |
| 22 | `read_clusters_bounded` | `crates/ast-sgrep-core/src/semantic_ann.rs:104` |
| 21 | `apply_weighted_rrf` | `crates/ast-sgrep-core/src/fusion.rs:207` |
| 20 | `index_content_at` | `crates/ast-sgrep-core/src/index.rs:732` |
| 20 | `embed_pass_lazy_ivf` | `crates/ast-sgrep-core/src/search/passes/embed.rs:113` |
| 20 | `classify_native` | `crates/ast-sgrep-lang/src/pattern.rs:138` |
| 20 | `read_node` | `crates/ast-sgrep-mcp/src/lib.rs:955` |
| 19 | `run_codemode_batch` | `crates/ast-sgrep-cli/src/lib.rs:229` |
| 19 | `refresh_lines_only` | `crates/ast-sgrep-core/src/store/sqlite.rs:657` |
| 19 | `cached_pattern_signatures` | `crates/ast-sgrep-lang/src/signature.rs:15` |
| 18 | `update_paths` | `crates/ast-sgrep-core/src/index.rs:638` |
| 18 | `measure_index_update` | `crates/ast-sgrep-core/src/pipeline_parts.rs:231` |
| 18 | `literal_sql` | `crates/ast-sgrep-core/src/search/passes/literal.rs:67` |
| 18 | `regex_pass` | `crates/ast-sgrep-core/src/search/passes/regex.rs:26` |
| 18 | `delete_file_lines` | `crates/ast-sgrep-core/src/store/sql.rs:271` |
| 17 | `embed_url_is_allowed` | `crates/ast-sgrep-embed/src/embedder.rs:27` |
| 16 | `run_bench_batch` | `crates/ast-sgrep-cli/src/bench.rs:452` |
| 16 | `run_process` | `crates/ast-sgrep-cli/src/lib.rs:52` |
| 16 | `load_semantic_context` | `crates/ast-sgrep-core/src/search/passes/embed.rs:41` |
| 15 | `run_bench` | `crates/ast-sgrep-cli/src/bench.rs:379` |
| 15 | `call_tool` | `crates/ast-sgrep-codemode/src/tools.rs:75` |
| 15 | `search_pattern` | `crates/ast-sgrep-core/src/pattern.rs:66` |
| 15 | `apply_kind_rule` | `crates/ast-sgrep-lang/src/extract.rs:273` |

### TypeScript / JS packages (CC≥15)

| CC | Function | File:line |
|---|---|---|
| 31 | `parseEnvelope` | `packages/pi/extension/src/runtime.ts:366` |
| 29 | `resolveHost` | `packages/pi/launcher/src/index.js:76` |
| 25 | `readLineWindow` | `packages/pi/extension/src/code-mode.ts:277` |
| 23 | `ensureFresh` | `packages/pi/extension/src/runtime.ts:265` |
| 23 | `resolveCodemodeAddon` | `packages/pi/launcher/src/index.js:128` |
| 22 | `argvFor` | `packages/pi/extension/src/codemode/dispatch.ts:231` |
| 22 | `resolveBinary` | `packages/pi/launcher/src/index.js:99` |
| 21 | `parseSearchHit` | `packages/pi/extension/src/code-mode.ts:173` |
| 17 | `searchToolCall` | `packages/pi/extension/src/index.ts:535` |
| 16 | `#start` | `packages/pi/extension/src/codemode/session-pool.ts:149` |
| 16 | `indexHealth` | `packages/pi/extension/src/runtime.ts:204` |
| 15 | `summarizeCodemode` | `packages/pi/extension/src/index.ts:580` |

## Exclusions (this campaign baseline)

| Path | Reason |
|---|---|
| `target/`, `node_modules/` | Not product source; measure tool cannot exclude when pointing at repo root |
| `packages/**/dist` | Build output |
| Skill / `.cyclomatic-reduction` | Meta |

## Next-Pass Queue

| Item | Why Deferred |
|---|---|
| Classify all top-30 hotspots | Pass 2 — essential vs accidental vs extractable |
| Analysis cards + technique pick | Pass 3+ after classify |
| First transform wave | Requires classify + technique; parity plan |
| Bench/test-only carve-out policy | Decide whether benches stay in ΣCC gate |

# Cyclomatic Reduction — Baseline Report

Run ID: `2026-08-10T235424Z-baseline`
Target root: `/Users/aditya/Developer/ast-sgrep`
Analyzer: `lizard` (version: `1.23.0`)
Measure tool: `measure_complexity.py --threshold 10`
Date: `2026-08-10T23:56:42Z`
Mode: **baseline-only** (no transforms)

## Command(s)

```bash
# Preflight
bash /Users/aditya/AI/JeffreySkills/_custom/cyclomatic-reduction/scripts/preflight.sh \
  /Users/aditya/Developer/ast-sgrep --run-id 2026-08-10T235424Z-baseline --mode baseline --skip-tests \
  --output /Users/aditya/Developer/ast-sgrep/.cyclomatic-reduction/runs/2026-08-10T235424Z-baseline/00-preflight-report.md

# Product scopes (whole-repo path not used: lizard would walk target/ + node_modules;
# measure_complexity.py has no exclude flags; 120s subprocess timeout)
python3 /Users/aditya/AI/JeffreySkills/_custom/cyclomatic-reduction/scripts/measure_complexity.py /Users/aditya/Developer/ast-sgrep/crates --threshold 10 \
  --output /Users/aditya/Developer/ast-sgrep/.cyclomatic-reduction/runs/2026-08-10T235424Z-baseline/02-baseline-raw-crates.json
python3 /Users/aditya/AI/JeffreySkills/_custom/cyclomatic-reduction/scripts/measure_complexity.py /Users/aditya/Developer/ast-sgrep/packages/pi/extension/src --threshold 10 \
  --output /Users/aditya/Developer/ast-sgrep/.cyclomatic-reduction/runs/2026-08-10T235424Z-baseline/02-baseline-raw-packages-ext.json
python3 /Users/aditya/AI/JeffreySkills/_custom/cyclomatic-reduction/scripts/measure_complexity.py /Users/aditya/Developer/ast-sgrep/packages/pi/launcher/src --threshold 10 \
  --output /Users/aditya/Developer/ast-sgrep/.cyclomatic-reduction/runs/2026-08-10T235424Z-baseline/02-baseline-raw-packages-launcher.json
# Merged → 02-baseline-raw.json
```

## Scope parts

- `/Users/aditya/Developer/ast-sgrep/crates` → functions=1684, ΣCC=5097, max=29, hotspots=75
- `/Users/aditya/Developer/ast-sgrep/packages/pi/extension/src` → functions=233, ΣCC=819, max=31, hotspots=13
- `/Users/aditya/Developer/ast-sgrep/packages/pi/launcher/src` → functions=10, ΣCC=106, max=29, hotspots=3

## Summary (merged product scope)

| Metric | Value |
|---|---|
| Files with ≥1 function | 166 |
| Functions scanned | 1927 |
| Functions above ceiling (CC > 10) | 91 |
| **ΣCC (total decision points)** | **6022** |
| Mean CC | 3.13 |
| Median CC | 2 |
| Max CC | 31 |
| Total NLOC (sum of function NLOC) | 34020 |
| Max cognitive complexity | n/a (lizard path; cognitive not populated) |
| Median cognitive complexity | n/a |

## CC histogram (function counts)

| Bucket | Count |
|---|---|
| 1-5 | 1627 |
| 6-10 | 209 |
| 11-15 | 57 |
| 16-20 | 19 |
| 21-25 | 12 |
| 26+ | 3 |

## Hotspots by crate / package area

| Area | Hotspots (CC>10) |
|---|---|
| `ast-sgrep-core` | 44 |
| `packages/pi/extension/src` | 13 |
| `ast-sgrep-cli` | 13 |
| `ast-sgrep-lang` | 8 |
| `ast-sgrep-codemode` | 4 |
| `packages/pi/launcher/src` | 3 |
| `ast-sgrep-mcp` | 2 |
| `ast-sgrep-plugins` | 2 |
| `ast-sgrep-embed` | 1 |
| `ast-sgrep-testkit` | 1 |

## Top 50 functions above ceiling

| File | Function | Line | CC | Cognitive | NLOC | Nesting Depth | Status |
|---|---|---|---|---|---|---|---|
| `packages/pi/extension/src/runtime.ts` | `parseEnvelope` | 366 | 31 | n/a | 37 | n/a | above_threshold |
| `crates/ast-sgrep-cli/src/bench.rs` | `run_bench_suite` | 231 | 29 | n/a | 147 | n/a | above_threshold |
| `packages/pi/launcher/src/index.js` | `resolveHost` | 76 | 29 | n/a | 23 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/semantic_ivf.rs` | `read_header` | 397 | 25 | n/a | 34 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/semantic_ivf.rs` | `save_semantic_ivf_with_publication` | 179 | 25 | n/a | 60 | n/a | above_threshold |
| `packages/pi/extension/src/code-mode.ts` | `readLineWindow` | 277 | 25 | n/a | 96 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/bench_suite.rs` | `measure_semantic_ivf_open_p99` | 198 | 24 | n/a | 75 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/index.rs` | `index_all` | 231 | 23 | n/a | 109 | n/a | above_threshold |
| `packages/pi/extension/src/runtime.ts` | `ensureFresh` | 265 | 23 | n/a | 51 | n/a | above_threshold |
| `packages/pi/launcher/src/index.js` | `resolveCodemodeAddon` | 128 | 23 | n/a | 33 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/semantic_ann.rs` | `read_clusters_bounded` | 104 | 22 | n/a | 62 | n/a | above_threshold |
| `packages/pi/extension/src/codemode/dispatch.ts` | `argvFor` | 231 | 22 | n/a | 29 | n/a | above_threshold |
| `packages/pi/launcher/src/index.js` | `resolveBinary` | 99 | 22 | n/a | 28 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/fusion.rs` | `apply_weighted_rrf` | 207 | 21 | n/a | 76 | n/a | above_threshold |
| `packages/pi/extension/src/code-mode.ts` | `parseSearchHit` | 173 | 21 | n/a | 33 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/index.rs` | `index_content_at` | 732 | 20 | n/a | 87 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/search/passes/embed.rs` | `embed_pass_lazy_ivf` | 113 | 20 | n/a | 63 | n/a | above_threshold |
| `crates/ast-sgrep-lang/src/pattern.rs` | `classify_native` | 138 | 20 | n/a | 51 | n/a | above_threshold |
| `crates/ast-sgrep-mcp/src/lib.rs` | `read_node` | 955 | 20 | n/a | 86 | n/a | above_threshold |
| `crates/ast-sgrep-cli/src/lib.rs` | `run_codemode_batch` | 229 | 19 | n/a | 68 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/store/sqlite.rs` | `refresh_lines_only` | 657 | 19 | n/a | 43 | n/a | above_threshold |
| `crates/ast-sgrep-lang/src/signature.rs` | `cached_pattern_signatures` | 15 | 19 | n/a | 39 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/index.rs` | `update_paths` | 638 | 18 | n/a | 45 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/pipeline_parts.rs` | `measure_index_update` | 231 | 18 | n/a | 46 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/search/passes/literal.rs` | `literal_sql` | 67 | 18 | n/a | 67 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/search/passes/regex.rs` | `regex_pass` | 26 | 18 | n/a | 92 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/store/sql.rs` | `delete_file_lines` | 271 | 18 | n/a | 32 | n/a | above_threshold |
| `crates/ast-sgrep-embed/src/embedder.rs` | `embed_url_is_allowed` | 27 | 17 | n/a | 65 | n/a | above_threshold |
| `packages/pi/extension/src/index.ts` | `searchToolCall` | 535 | 17 | n/a | 34 | n/a | above_threshold |
| `crates/ast-sgrep-cli/src/bench.rs` | `run_bench_batch` | 452 | 16 | n/a | 87 | n/a | above_threshold |
| `crates/ast-sgrep-cli/src/lib.rs` | `run_process` | 52 | 16 | n/a | 58 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/search/passes/embed.rs` | `load_semantic_context` | 41 | 16 | n/a | 56 | n/a | above_threshold |
| `packages/pi/extension/src/codemode/session-pool.ts` | `#start` | 149 | 16 | n/a | 47 | n/a | above_threshold |
| `packages/pi/extension/src/runtime.ts` | `indexHealth` | 204 | 16 | n/a | 13 | n/a | above_threshold |
| `crates/ast-sgrep-cli/src/bench.rs` | `run_bench` | 379 | 15 | n/a | 72 | n/a | above_threshold |
| `crates/ast-sgrep-codemode/src/tools.rs` | `call_tool` | 75 | 15 | n/a | 56 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/pattern.rs` | `search_pattern` | 66 | 15 | n/a | 37 | n/a | above_threshold |
| `crates/ast-sgrep-lang/src/extract.rs` | `apply_kind_rule` | 273 | 15 | n/a | 138 | n/a | above_threshold |
| `packages/pi/extension/src/index.ts` | `summarizeCodemode` | 580 | 15 | n/a | 31 | n/a | above_threshold |
| `crates/ast-sgrep-cli/src/search_cmd.rs` | `run_chain` | 14 | 14 | n/a | 45 | n/a | above_threshold |
| `crates/ast-sgrep-cli/src/watch.rs` | `run_watch` | 9 | 14 | n/a | 76 | n/a | above_threshold |
| `crates/ast-sgrep-codemode/src/batch.rs` | `run_serve` | 258 | 14 | n/a | 80 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/search/mod.rs` | `search` | 386 | 14 | n/a | 45 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/store/sqlite.rs` | `init_schema` | 165 | 14 | n/a | 43 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/store/sqlite.rs` | `persist_embed_metadata` | 861 | 14 | n/a | 28 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/store/sqlite.rs` | `upsert_file` | 578 | 14 | n/a | 49 | n/a | above_threshold |
| `crates/ast-sgrep-cli/src/agent.rs` | `clap_catalog` | 112 | 13 | n/a | 73 | n/a | above_threshold |
| `crates/ast-sgrep-cli/src/bench.rs` | `update_bench_history` | 149 | 13 | n/a | 62 | n/a | above_threshold |
| `crates/ast-sgrep-cli/src/search_cmd.rs` | `run_search` | 79 | 13 | n/a | 37 | n/a | above_threshold |
| `crates/ast-sgrep-core/src/chain.rs` | `expand_one` | 136 | 13 | n/a | 77 | n/a | above_threshold |

## Full hotspot list

See `02-baseline-raw.json` → `hotspots` (91 entries) and `03-target-ledger.md` (top ~30 for next passes).

## Notes

- Status = `above_threshold` if CC > 10.
- Cognitive complexity not reported by this lizard invocation path (`cognitive: null`).
- Crates measure includes `crates/**/tests`, examples, benches — product-adjacent code still in ΣCC.
  Later waves may carve bench-only / test-only exclusions with explicit bill notes.
- Numbers come only from lizard via `measure_complexity.py`; no invented perf claims.
- Prior runs leveraged: **none** (first baseline pass).

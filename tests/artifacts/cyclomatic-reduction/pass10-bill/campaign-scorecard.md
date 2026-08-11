# 08 — Complexity scorecard (campaign bill, Pass 10 / 12)

Run ID: `2026-08-11Tpass10-bill`  
Mode: **bill-remeasure only** (zero product edits)  
Analyzer: `lizard` 1.23.0 via `measure_complexity.py --threshold 10`  
Scope: `crates/` + `packages/pi/extension/src` + `packages/pi/launcher/src`  
Baseline: `2026-08-10T235424Z-baseline`

## Campaign headline (full-scope re-measure)

| Metric | Baseline (pass 1) | After pass 10 re-measure | Δ |
|---|---:|---:|---:|
| **ΣCC** | **6022** | **5994** | **−28** |
| Max CC | 31 | 26 | −5 |
| Median CC | 2 | 2 | 0 |
| Mean CC | 3.13 | 3.07 | −0.06 |
| Functions | 1927 | 1953 | +26 |
| Hotspots CC>10 | 91 | 83 | **−8** |
| Files w/ functions | 166 | 166 | 0 |
| Total NLOC (fn sum) | 34020 | 34192 | +172 |

**Displacement check: PASS** — full-scope ΣCC did not rise (5994 ≤ 6022). Function count rose because prior waves extracted helpers (honest base cost); net decision points still fell.

## Parts (after re-measure)

| Scope | Functions | ΣCC | Max | Hotspots |
|---|---:|---:|---:|---:|
| `crates/` | 1698 | 5090 | 25 | 70 |
| `packages/pi/extension/src` | 241 | 802 | 25 | 10 |
| `packages/pi/launcher/src` | 14 | 102 | 26 | 3 |
| **Merged** | **1953** | **5994** | **26** | **83** |

Baseline parts for comparison: crates 1684 / Σ 5097 / max 29 / hs 75; extension 233 / 819 / 31 / 13; launcher 10 / 106 / 29 / 3.

| Scope | ΣCC baseline → now | ΔΣCC | Hotspots baseline → now |
|---|---|---:|---|
| crates | 5097 → 5090 | **−7** | 75 → 70 |
| extension | 819 → 802 | **−17** | 13 → 10 |
| launcher | 106 → 102 | **−4** | 3 → 3 |

## Histogram (functions)

| Bucket | Baseline | Now | Δ |
|---|---:|---:|---:|
| 1–5 | 1627 | 1644 | +17 |
| 6–10 | 209 | 226 | +17 |
| 11–15 | 57 | 59 | +2 |
| 16–20 | 19 | 17 | −2 |
| 21–25 | 12 | 6 | −6 |
| 26+ | 3 | 1 | −2 |

Head of distribution compressed: only one function remains CC≥26 (`resolveHost` 26); baseline max was `parseEnvelope` 31.

## Hotspots by area (now)

| Area | Hotspots CC>10 |
|---|---:|
| `ast-sgrep-core` | 42 |
| `ast-sgrep-cli` | 10 |
| `packages/pi/extension/src` | 10 |
| `ast-sgrep-lang` | 8 |
| `ast-sgrep-codemode` | 4 |
| `packages/pi/launcher/src` | 3 |
| `ast-sgrep-mcp` | 2 |
| `ast-sgrep-plugins` | 2 |
| `ast-sgrep-testkit` | 1 |
| `ast-sgrep-embed` | 1 |

## Wave ledger (passes 1–9, evidence from prior scorecards)

| Pass | Focus | Touched / package ΣCC Δ | Notes |
|---|---|---|---|
| 1 | baseline | — | ΣCC 6022, max 31, hs 91 |
| 2 | classify | 0 product | ledger + tallies |
| 3 | guard clauses | launcher+index **−3** touched | resolve\* −3/−5/−5; update_paths −3 |
| 4 | extract method | touched **−4** | index_all, delete_file_lines, parseSearchHit, … |
| 5 | lookup_table | touched **−15** | argvFor, searchToolCall, literal_sql |
| 6 | boolean / nesting | extension package **−2** | ensureFresh 23→10 |
| 7 | error-path | extension package **−2** | parseEnvelope 31→17 |
| 8 | core residual | touched **0** (bill-neutral shared collapse) | literal + IVF write extract; pure dumps refused |
| 9 | surface residual | cli package **−2** | run_bench\* shared collapse; pure extracts refused |
| **10** | **full Bill re-measure** | **product 0** | **campaign ΣCC 5994 (−28)** |

Per-wave Σ are not additive (different scopes / baselines); campaign gate is this full re-measure only.

## Max-CC migration (headline functions)

| Function | Baseline CC | Now CC | Resolve |
|---|---:|---:|---|
| `parseEnvelope` | 31 | **17** | residual Keep (protocol) |
| `resolveHost` | 29 | **26** | Defer (pure extract Refuse +6) |
| `run_bench_suite` | 29 | **24** | Defer residual case loop |
| `readLineWindow` | 25 | **25** | Keep |
| `read_header` | 25 | **25** | Keep (format parser) |
| `ensureFresh` | 23 | **10** | under hard ceiling |
| `run_bench` | 15 | **9** | under hard ceiling (pass 9) |
| `run_search` | 13 | **10** | at hard ceiling (pass 9) |

## Pass 10 product edits

**ZERO product change.** Measure-only bill pass.

## Metric-gaming auditor (campaign)

- Helpers added only when parent decisions collapsed or shared trees merged (passes 3–9).
- Multiple pure-extract attempts **refused** when touched ΣCC rose (pass 8 walk/regex/update_paths; pass 9 run_process / launcher / measure_suite_case).
- No public API redesign; no domain scatter of KindRule / IVF format / allowlists.
- Full-scope ΣCC **down** with more functions → not dump-to-flatten gaming.

**METRIC_GAMING_RESULT: pass**

## Campaign status

`CUT_BRANCHES_RESULT: partial` — bill re-measure complete; 83 hotspots remain above hard ceiling; residual Keep/Defer ledger written. Not campaign-complete (ceiling not cleared for authorized full scope).

## Artifacts

- Slim metrics: `bill-summary.json`, `02-remeasure-merged-summary.json`
- Raw (run dir only, not mirrored to tests/artifacts full): `02-remeasure-raw-*.json`
- Residual: `residual-hotspots.md`, `09-final-reduction-report.md`
- Next: `NEXT_PASS.md` → pass 11 parity + beads
- Mirror: `tests/artifacts/cyclomatic-reduction/pass10-bill/`

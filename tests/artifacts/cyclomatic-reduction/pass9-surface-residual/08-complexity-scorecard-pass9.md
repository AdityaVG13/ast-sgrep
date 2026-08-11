# 08 — Complexity scorecard (Pass 9 surface residual)

| Metric | Value |
|---|---|
| Wave | Pass 9 / 12 |
| Mode | module-pass residual (surface) |
| Touched ΣCC | 151 → **149** (Δ **-2**) |
| CLI package ΣCC | 632 → **630** (Δ **-2**) |
| Hotspots cut | 3 functions (`run_bench_suite`, `run_bench`, `run_search`) |
| Under hard ceiling (new this wave) | `run_bench` 15→**9** |
| At ceiling | `run_search` 13→**10** |
| Refuse (measured +ΣCC) | 3 pure-extract attempts |
| Launcher resolve\* | **ZERO-CHANGE** this wave (Refuse dump) |
| LSP | already max CC 9 — no action |
| Lang Keep | `classify_native`, signatures, kind rules |
| Parity | pass (cli machine_contracts bench + smoke + lib; launcher floor) |
| Public API | stable |

## Per-function (transformed)

| Function | Before | After | Technique |
|---|---:|---:|---|
| `run_bench_suite` | 29 | 24 | shared collapse |
| `run_bench` | 15 | 9 | shared collapse |
| `run_search` | 13 | 10 | consolidate predicates |

## Campaign status

`CUT_BRANCHES_RESULT: partial` — surface residual wave complete; many surface Keep/Defer rows remain for pass 10 full-scope Bill re-measure.

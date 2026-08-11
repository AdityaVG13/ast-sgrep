# 09 — Final reduction report (through Pass 12)

## Identity

| Field | Value |
|---|---|
| Target | `/Users/aditya/Developer/ast-sgrep` |
| Branch | `perf/software-optimization` |
| Skill | cyclomatic-reduction |
| Authorized scope | `crates/` · `packages/pi/extension/src` · `packages/pi/launcher/src` |
| Baseline | `2026-08-10T235424Z-baseline` |
| Bill | `2026-08-11Tpass10-bill` (frozen metrics) |
| Parity | `2026-08-11Tpass11-parity` |
| Convergence | `2026-08-11Tpass12-convergence` |
| Analyzer | lizard 1.23.0 + `measure_complexity.py --threshold 10` |

## Objective gates

| Gate | Result | Evidence |
|---|---|---|
| ΣCC not increased vs baseline | **PASS** | 6022 → 5994 |
| Displacement | **PASS** | net −28; refused +Σ dumps |
| Max CC reduced | **PASS** | 31 → 26 |
| Hotspot count reduced | **PASS** | 91 → 83 |
| Pass 12 re-measure matches pass 10 | **PASS** | all headline metrics Δ 0 |
| Product source edits pass 10–12 | **ZERO** each | git + pass docs |
| Full-scope ceiling cleared (≤10) | **FAIL (expected)** | 83 still CC>10 |
| Absolute convergence | **CONVERGED** | no fundable accidental cut without +ΣCC |
| Campaign complete (ceiling) | **no / partial** | residual Keep/Defer |

## Product files changed — Pass 12

**ZERO** under `crates/`, `packages/pi/extension/src`, `packages/pi/launcher/src`.

## Residual

- D1–D3 markdown queue remains **Defer** (shared-collapse only)
- Keep ledger frozen (IVF/ANN/fusion/lang/protocol/security)
- Named refuse evidence: `named-checks.md`

## RESULT

```
CUT_BRANCHES_RESULT: partial
go_ahead: complete-with-residuals
Scope: crates/ + packages/pi/extension/src + packages/pi/launcher/src
Target ceiling: 10
Hotspots addressed: multi-wave (3–9); residual 83 above ceiling (Keep/Defer)
Before median/max CC: 2 / 31
After median/max CC: 2 / 26
Total decision points (ΣCC) before/after: 6022 / 5994
Function count before/after: 1927 / 1953
Cognitive complexity before/after: n/a (not tracked campaign-wide)
Displacement check: pass
Absolute convergence: CONVERGED (pass10+11+12 product zero; re-measure Δ0 vs pass10; no fundable pure-extract remaining)
Differential parity: pass (pass 11 joint-allowed floors; not re-run workspace suite this pass)
Cognitive complexity delta: n/a
1000-point scorecard: ~780 / 1000 (B+ partial estimate) | not complete
Parity evidence: pass 11 matrix + per-wave characterization; pass 12 measure-only
Artifacts: .cyclomatic-reduction/runs/2026-08-11Tpass12-convergence/
Mirror: tests/artifacts/cyclomatic-reduction/pass12-convergence/
Prior runs leveraged: baseline, pass3–11 under tests/artifacts/cyclomatic-reduction/
Next pass: none (campaign residual permanent Keep/Defer unless authorized ceiling redefined or proven shared-collapse funded)
```

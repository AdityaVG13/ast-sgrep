# 09 — Final reduction report (through Pass 11)

## Identity

| Field | Value |
|---|---|
| Target | `/Users/aditya/Developer/ast-sgrep` |
| Branch | `perf/software-optimization` |
| Skill | cyclomatic-reduction |
| Authorized scope | `crates/` · `packages/pi/extension/src` · `packages/pi/launcher/src` |
| Baseline | `2026-08-10T235424Z-baseline` |
| Bill | `2026-08-11Tpass10-bill` |
| Parity | `2026-08-11Tpass11-parity` |

## Objective gates

| Gate | Result | Evidence |
|---|---|---|
| ΣCC not increased | **PASS** | 6022 → 5994 |
| Displacement | **PASS** | net −28; refused +Σ dumps |
| Max CC reduced | **PASS** | 31 → 26 |
| Hotspot count reduced | **PASS** | 91 → 83 |
| Public API stable | **PASS** | waves 3–9 notes |
| Full-scope ceiling cleared | **FAIL (expected)** | 83 still CC>10 |
| Differential parity | **PASS (pass 11)** | joint-allowed floors green |
| Campaign complete | **no** | residual Keep/Defer remain |

## Product files changed — Pass 11

**ZERO.**

## Residual

- Markdown queue D1–D3 (hardened pass 11)
- Keep ledger frozen
- Pre-existing F1 inventory / F2 keyword matrix — out of CC scope

## RESULT

```
CUT_BRANCHES_RESULT: partial
go_ahead: pass12-zero-change-convergence
Scope: crates/ + packages/pi/extension/src + packages/pi/launcher/src
Target ceiling: 10
Hotspots addressed: multi-wave (3–9); residual 83 above ceiling
Before median/max CC: 2 / 31
After median/max CC: 2 / 26
Total decision points (ΣCC) before/after: 6022 / 5994
Function count before/after: 1927 / 1953
Cognitive complexity before/after: n/a (not tracked campaign-wide)
Displacement check: pass
Differential parity: pass: joint-allowed floors (ext 88, launcher 13, cli 32, core 32+); pre-existing F1/F2 documented
Cognitive complexity delta: n/a
1000-point scorecard: ~780 / 1000 (B+ partial estimate) | not complete
Parity evidence: tests (pass 11 matrix) + per-wave characterization
Artifacts: .cyclomatic-reduction/runs/2026-08-11Tpass11-parity/
Prior runs leveraged: 2026-08-10T235424Z-baseline, pass3–10 mirrors under tests/artifacts/cyclomatic-reduction/
Next pass: pass 12 ZERO-CHANGE convergence scan (expect no product cuts unless accidental fundable shared-collapse appears during scan — default none)
```

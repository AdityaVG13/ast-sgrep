# CONVERGENCE.md — Pass 12 absolute convergence

| Field | Value |
|---|---|
| Run ID | `2026-08-11Tpass12-convergence` |
| Branch | `perf/software-optimization` |
| Mode | re-measure + residual refuse documentation only |
| Product source edits | **ZERO** |
| Bill frozen (pass 10) | ΣCC **5994**, max **26**, hotspots **83** |
| Pass 12 re-measure | ΣCC **5994**, max **26**, hotspots **83** |
| Match pass 10 | **YES** (all headline metrics Δ 0) |
| Pass 10 product edits | ZERO |
| Pass 11 product edits | ZERO |
| Pass 12 product edits | ZERO |

## Verdict

# **CONVERGED**

**Definition used:** two consecutive product-zero passes (10 bill, 11 parity) plus this absolute re-scan with product-zero and **no new fundable accidental cuts** without raising ΣCC.

Not **PRODUCTIVE**: no bill-negative shared-collapse discovered during scan that would authorize a transform exception.

## Campaign headline (stable)

| Metric | Baseline | Pass 10/12 now | Δ |
|---|---:|---:|---:|
| ΣCC | 6022 | **5994** | **−28** |
| Max CC | 31 | **26** | −5 |
| Hotspots CC>10 | 91 | **83** | −8 |
| Functions | 1927 | 1953 | +26 |

Parts (identical pass 10 ↔ pass 12):

| Scope | Functions | ΣCC | Max | Hotspots |
|---|---:|---:|---:|---:|
| crates/ | 1698 | 5090 | 25 | 70 |
| packages/pi/extension/src | 241 | 802 | 25 | 10 |
| packages/pi/launcher/src | 14 | 102 | 26 | 3 |

## Residual posture (frozen)

- **Keep ledger:** domain parsers/validators/security/ranking — do not cut for score.
- **D1–D3:** open **Defer** only; pure extracts historically **Refuse** (+6 / +3 / +4-class). Shared-collapse only with pre/post −ΣCC proof.
- **Ceiling:** full-scope hard ceiling 10 **not** cleared (83 residual) — expected; campaign remains partial on ceiling gate.

## Commands run (evidence)

```bash
python3 …/measure_complexity.py crates --threshold 10 \
  --output .cyclomatic-reduction/runs/2026-08-11Tpass12-convergence/02-remeasure-raw-crates.json
python3 …/measure_complexity.py packages/pi/extension/src --threshold 10 \
  --output …/02-remeasure-raw-packages-ext.json
python3 …/measure_complexity.py packages/pi/launcher/src --threshold 10 \
  --output …/02-remeasure-raw-launcher.json
git diff --stat -- crates packages/pi/extension/src packages/pi/launcher/src  # empty
```

Merged: functions 1953, total_cc 5994, max 26, median 2, mean 3.07, hotspots 83, nloc 34192, files 166.

## RESULT posture

`CUT_BRANCHES_RESULT: partial` — residual Keep/Defer above ceiling is honest residual, not unfinished accidental work.  
`go_ahead: complete-with-residuals` (blocked on full ceiling clear; not blocked on unknown fundable cuts).

## Out of scope notes

- Pre-existing workspace noise (`.beads/`, `Cargo.lock`, `packages/pi/extension/dist/*`, target-pass* dirs) is **not** this pass and **not** authorized product **source** under measure scope.
- F1 pack inventory / F2 keyword matrix remain out of CC campaign (pass 11).

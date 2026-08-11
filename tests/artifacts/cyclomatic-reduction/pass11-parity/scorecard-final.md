# Campaign scorecard (final narrative) — through Pass 11

Run ID: `2026-08-11Tpass11-parity`  
Bill numbers: frozen at **pass 10** (`2026-08-11Tpass10-bill`)  
Pass 11 mode: **parity + residual queue + narrative** (zero product edits)

## Headline (canonical — do not invent alternate ΣCC)

| Metric | Baseline (pass 1) | Pass 10 bill | Δ |
|---|---:|---:|---:|
| **ΣCC** | 6022 | **5994** | **−28** |
| Max CC | 31 | **26** | −5 |
| Median CC | 2 | 2 | 0 |
| Functions | 1927 | 1953 | +26 |
| Hotspots CC>10 | 91 | **83** | −8 |

Provenance: `.cyclomatic-reduction/runs/2026-08-11Tpass10-bill/bill-summary.json`  
Mirror: `tests/artifacts/cyclomatic-reduction/pass10-bill/`

**Displacement check: PASS** (ΣCC down; helper count up from honest extracts; pure-extract dumps refused in waves 8–9).

## Part breakdown (pass 10)

| Scope | ΣCC baseline → now | Δ | Hotspots |
|---|---|---:|---:|
| crates/ | 5097 → 5090 | −7 | 75 → 70 |
| packages/pi/extension/src | 819 → 802 | −17 | 13 → 10 |
| packages/pi/launcher/src | 106 → 102 | −4 | 3 → 3 |

## Technique scoreboard (what actually moved ΣCC)

| Technique family | Waves | Campaign contribution (qualitative) |
|---|---|---|
| Lookup tables | 5 | Largest single-wave touched Δ (−15 class) |
| Shared collapse / consolidate | 8–9 | Bill-neutral to −2 package; refused vanity extracts |
| Error-path extract | 7 | parseEnvelope 31→17; package −2 |
| Boolean / nesting | 6 | ensureFresh under hard ceiling |
| Guard clauses | 3 | launcher resolve\* + update_paths |
| Extract with decision elimination | 4 | index_all / delete_file_lines / parseSearchHit |

## Parity spine (pass 11 re-proof)

| Floor | Result |
|---|---|
| cargo check core/cli/mcp | PASS |
| extension npm test (88) | PASS |
| launcher resolve floor (13) | PASS |
| cli machine_contracts + smoke + lib | PASS |
| core parity / e2e / regex / IVF / epics / prose | PASS |
| Pre-existing pack inventory + mode `keyword` matrix | FAIL documented (non-campaign) |

**Campaign differential parity: PASS** (joint-allowed floors).  
See `parity-matrix.md` and `07-parity-report-pass11.md`.

## Metric-gaming auditor (campaign)

| Check | Result |
|---|---|
| ΣCC conservation | PASS (−28) |
| No dump-to-flatten (refused +Σ extracts) | PASS |
| No domain scatter (Keep ledger) | PASS |
| Public API redesign | none |
| Pass 11 product edits | **0** |

**METRIC_GAMING_RESULT: pass**

## Residual policy (frozen for pass 12)

- **Keep** essential domain / format / protocol / security (see residual-queue-INDEX Keep table)
- **Defer** D1–D3 only with bill-negative shared-collapse
- **Refuse** pure extract / API redesign / ceiling games

Residual CC>10: **83** — campaign **not** complete under hard ceiling 10 for full authorized scope.

## 1000-point scorecard (campaign honest grade)

Not claiming formal `scripts/score_scorecard.py` ≥900 **complete**. Approximate narrative bands:

| Dimension | Assessment | Notes |
|---|---|---|
| Measure / bill | strong | Full-scope re-measure + JSON bill |
| Transform craft | strong | Multiple techniques; refuses documented |
| Parity | strong after pass 11 | Floors re-green; level-4 per-wave characterization |
| Residual honesty | strong | Keep vs Defer vs Refuse explicit |
| Ceiling clearance | incomplete | 83 hotspots remain |
| Complete claim | **no** | Must stay `partial` |

Effective campaign posture: **solid partial** — good ΣCC bill + parity, not ceiling-complete.

If a numeric grade is required for artifacts without running the scorer:

`1000-point scorecard: ~780 / 1000 (B+ partial) | n/a complete`  
(Estimate only; pass 12 may run `score_scorecard.py` / `validate_cut_branches.py` if present — must not invent green.)

## Campaign status

```
CUT_BRANCHES_RESULT: partial
go_ahead: pass12-zero-change-convergence
SigmaCC: 6022 -> 5994 (delta -28)
max_cc: 31 -> 26
hotspots_gt_10: 91 -> 83
product_edits_pass_11: 0
displacement_check: pass
differential_parity: pass (joint-allowed floors; see parity-matrix)
```

## Artifacts

| Path | Role |
|---|---|
| this file | final narrative scorecard |
| `parity-matrix.md` | command evidence |
| `residual-queue-INDEX.md` + `work-queue/D*.md` | implementer queue |
| `NEXT_PASS.md` | pass 12 absolute |
| pass10 bill dir | canonical numbers |

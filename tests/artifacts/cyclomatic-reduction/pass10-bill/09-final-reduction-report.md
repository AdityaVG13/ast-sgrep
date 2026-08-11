# 09 — Final reduction report (draft bill fields) — Pass 10

> **Draft for campaign close.** Passes 11–12 still own parity re-check and residual bead/queue polish. This is the **Bill** chapter after full-scope re-measure.

## Identity

| Field | Value |
|---|---|
| Target | `/Users/aditya/Developer/ast-sgrep` |
| Authorized scope | `crates/` · `packages/pi/extension/src` · `packages/pi/launcher/src` |
| Skill | cyclomatic-reduction |
| Analyzer | lizard 1.23.0 + `measure_complexity.py` |
| Baseline run | `2026-08-10T235424Z-baseline` |
| Bill run | `2026-08-11Tpass10-bill` |
| Branch | `perf/software-optimization` |

## Objective gates (Bill)

| Gate | Result | Evidence |
|---|---|---|
| ΣCC did not increase vs baseline | **PASS** | 6022 → **5994** (Δ **−28**) |
| Displacement justified | **PASS** | net −28; refused +Σ pure extracts in waves 8–9 |
| Max CC reduced or held | **PASS** | 31 → **26** |
| Hotspot count reduced | **PASS** | 91 → **83** (−8) |
| Public API stable | **PASS** (waves 3–9 notes) | no redesign authorized / none shipped |
| Full-scope ceiling cleared | **FAIL (expected partial)** | 83 still CC>10 |
| Differential parity campaign-wide | **DEFER to pass 11** | per-wave parity green; full re-parity pending |
| Scorecard ≥900 complete claim | **N/A** | still `partial`, not `complete` |

## Numbers (canonical for docs)

Do **not** invent other campaign ΣCC figures. Use this row:

| Label | ΣCC | Max | Median | Functions | Hotspots CC>10 |
|---|---:|---:|---:|---:|---:|
| Baseline | 6022 | 31 | 2 | 1927 | 91 |
| Pass 10 bill | 5994 | 26 | 2 | 1953 | 83 |
| Δ | −28 | −5 | 0 | +26 | −8 |

Provenance: `.cyclomatic-reduction/runs/2026-08-11Tpass10-bill/bill-summary.json` and raw part JSON under same run dir.

## What worked (technique families)

1. **Shared collapse / consolidate predicates** — pass 9 bench ratchet + human print; pass 8 `content_matches_literal`.
2. **Lookup tables** — pass 5 argvFor / searchToolCall / SQL template (large touched −15).
3. **Error-path extract** — pass 7 parseEnvelope / run / rebuild family (−2 package).
4. **Boolean decompose** — pass 6 ensureFresh under ceiling.
5. **Guard clauses** — pass 3 launcher resolve\* + watch skip.
6. **Extract with decision elimination** — pass 4 index_all, delete_file_lines, parseSearchHit, read_node.

## What was refused (Ashby / Kolmogorov)

- Pure extract of launcher resolve\* (+6 measured) and CLI `run_process` (+3).
- Walk / regex multi-helper fan-out that raised file ΣCC (pass 8).
- Scattering KindRule / IVF header / URL allowlist / platform security codes.

## Residual policy

- **Keep** = essential domain / format / protocol variety (do not cut for score).
- **Defer** = fundable only with shared-collapse or decision elimination, not dump.
- **Refuse** = metric games or API redesign.

See `residual-hotspots.md` and `work-queue/` packets.

## Pass 10 product files changed

**None (ZERO).**

## CUT_BRANCHES_RESULT

```
CUT_BRANCHES_RESULT: partial
go_ahead: pass11-parity-and-residual-queue
SigmaCC: 6022 -> 5994 (delta -28)
max_cc: 31 -> 26
hotspots_gt_10: 91 -> 83
product_edits_this_pass: 0
displacement_check: pass
```

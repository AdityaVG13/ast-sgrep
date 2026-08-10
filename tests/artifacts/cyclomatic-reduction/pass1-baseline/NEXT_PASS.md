# NEXT_PASS.md

Run ID: `2026-08-10T235424Z-baseline`
Completed: **Pass 1 — Preflight + baseline census** (baseline-only)
Next: **Pass 2 — Classify**

## Do next

1. Load this run via `.cyclomatic-reduction/LATEST` → `2026-08-10T235424Z-baseline`.
2. **Do not re-baseline** unless scope changes; reuse `02-baseline-raw.json` / `02-baseline-report.md`.
3. Classify each hotspot in `03-target-ledger.md` (top ~30 minimum):
   - `essential_domain` | `accidental_structure` | `dead_path` | `extractable`
4. Write analysis stubs under `04-analysis-cards/` (or card index) with **Resolve** intent: Cut / Keep / Defer / Refuse.
5. Propose first transform wave (small, high-CC accidental/extractable only) for a later pass — **no product edits in pass 2** unless mode gate upgrades.
6. Prefer beads upgrade-in-place for actionable P0–P1 items when creating work packets (per skill FINDINGS handoff).

## Inputs ready

| Artifact | Path |
|---|---|
| Baseline raw | `runs/2026-08-10T235424Z-baseline/02-baseline-raw.json` |
| Baseline report | `runs/2026-08-10T235424Z-baseline/02-baseline-report.md` |
| Ledger | `runs/2026-08-10T235424Z-baseline/03-target-ledger.md` |
| Intent | `runs/2026-08-10T235424Z-baseline/01-intent-statement.md` |

## Key numbers to beat later

- ΣCC: **6022**
- Max CC: **31** (`parseEnvelope` @ `packages/pi/extension/src/runtime.ts`)
- Hotspots CC>10: **91**
- Ceiling: 10

## Mode reminder

Campaign is multipass repo-sweep. Pass 2 = classify only unless orchestrator expands gate.

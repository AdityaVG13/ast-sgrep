# Pass 7 — Error/Result path consolidation (artifacts mirror)

Campaign run: `.cyclomatic-reduction/runs/2026-08-10T235424Z-baseline`

## Transforms (`packages/pi/extension/src/runtime.ts`)

| Function | CC before → after |
|---|---|
| parseEnvelope | 31 → 17 |
| run | 12 → 6 |
| rebuildIncompatibleIndex | 11 → 7 |

Helpers: `throwNonzeroProcessFailure` (10), `rethrowExecFailure` (7), `throwIndexRebuildFailed` (5).

## Bill

- `runtime.ts` ΣCC 196 → 194 (−2)
- extension `src/` total_cc 804 → 802 (−2)
- Parity: `npm test` in `packages/pi/extension` → 88 passed

## Files here

- `runtime-error-path-regions.ts` — extracted regions snapshot
- Full before/after under campaign `06-transformed-code/pass7-*`

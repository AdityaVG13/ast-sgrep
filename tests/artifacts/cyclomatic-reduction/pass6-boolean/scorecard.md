# Complexity scorecard — Pass 6 (boolean / nesting)

Analyzer: lizard via `measure_complexity.py`
Scope files: `runtime.ts` + `index.ts` + `code-mode.ts` (touched product surface)

## Per-target CC

| Function | File | Before | After | Δ | Technique |
|---|---|---:|---:|---:|---|
| `ensureFresh` | runtime.ts | 23 | **10** | **−13** | combine_predicates + decompose (`runIndex`, `probeIndexHealth`, `needsIncrementalIndex`) |
| `assertVersionTriple` | runtime.ts | 7 | 7 | 0 | compound guard (nesting only) |
| `asSearchResponse` | code-mode.ts | 10 | 10 | 0 | replace nested compound with guard |
| `needsIncrementalIndex` (new) | runtime.ts | — | 4 | +4 | named predicate |
| `probeIndexHealth` (new) | runtime.ts | — | 1 | +1 | extract probe |
| `runIndex` (new) | runtime.ts | — | 3 | +3 | extract dispatch |
| `refresh` (IIFE) | runtime.ts | 1 | 4 | +3 | body retains health branch |

## File ΣCC (Bill)

| File | Before | After | Δ |
|---|---:|---:|---:|
| `packages/pi/extension/src/runtime.ts` | 198 | **196** | **−2** |
| `packages/pi/extension/src/index.ts` | 140 | 140 | 0 |
| `packages/pi/extension/src/code-mode.ts` | 145 | 145 | 0 |
| **Touched scope Σ** | **483** | **481** | **−2** |

Extension package total_cc: **806 → 804 (−2)**.

**Displacement check:** justified — helpers hold shared dispatch/predicates once; duplicate force:false arm removed. No ΣCC rise.

## Residual (still > hard ceiling 10) — for pass 7+

| Function | CC | Note |
|---|---:|---|
| parseEnvelope | 31 | pass 7 error-path extract |
| readLineWindow | 25 | Keep essential |
| isValidHitShape | 18 | Keep / name only |
| indexHealth | 16 | Keep status-shape ORs |
| #start (session-pool) | 16 | extract residual |
| summarizeCodemode | 15 | nested ternary; sequential ifs **raised** lizard CC — leave |
| rebuildIncompatibleIndex | 11 | residual |
| run (AstSgrepRuntime) | 12 | residual |
| argvFor / searchToolCall | 11 | form interpreters from pass 5 |
| resolveHost / resolveBinary / resolveCodemodeAddon | 26/17/18 | launcher residual |
| literal_sql | 16 | word_mode essential residual |

## CUT_BRANCHES_RESULT

`partial` — pass-6 primary target under hard ceiling (10); wave complete for boolean focus.

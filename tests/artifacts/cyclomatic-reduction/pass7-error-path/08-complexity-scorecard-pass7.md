# Complexity scorecard — Pass 7 (error-path)

Analyzer: `measure_complexity.py` (lizard-backed) · language typescript  
Scope file: `packages/pi/extension/src/runtime.ts`  
Package: `packages/pi/extension/src`

## File bill (`runtime.ts`)

| Metric | Before | After | Δ |
|---|---:|---:|---:|
| ΣCC | 196 | 194 | **−2** |
| functions | 37 | 40 | +3 |
| max CC | 31 | 17 | −14 |
| median CC | 4 | (see JSON) | — |

## Touched function family

| Function | CC before | CC after | Notes |
|---|---:|---:|---|
| `parseEnvelope` | 31 | **17** | limit+record cuts; nonzero ladder extracted |
| `throwNonzeroProcessFailure` | — | 10 | new error-path helper |
| `run` | 12 | **6** | under hard ceiling |
| `rethrowExecFailure` | — | 7 | new |
| `rebuildIncompatibleIndex` | 11 | **7** | under hard ceiling |
| `throwIndexRebuildFailed` | — | 5 | new |
| **Family Σ** (parents before / family after) | 54 | 52 | **−2** |

## Extension package `src/`

| Metric | Before | After | Δ |
|---|---:|---:|---:|
| total_cc | 804 | 802 | **−2** |
| functions_scanned | 238 | 241 | +3 |
| max_cc | 31 | 25 | −6 (`readLineWindow` now max) |
| functions_above_threshold (10) | 12 | 10 | −2 |

## Displacement check

**pass** — file and package ΣCC both −2; new helpers justified (named failure ladders); no unbilled dump.

## Residual above hard ceiling 10 (runtime.ts)

| Function | CC | Note |
|---|---:|---|
| parseEnvelope | 17 | protocol Keep residual |
| indexHealth | 16 | Keep |
| ensureFresh | 10 | at hard ceiling (pass 6) |
| throwNonzeroProcessFailure | 10 | at hard ceiling |
| migrateConfig | 9 | under |

# Pass 3 — before/after CC

Analyzer: `scripts/measure_complexity.py` (lizard)  
Measure JSON: `.cyclomatic-reduction/runs/2026-08-10T235424Z-baseline/06-transformed-code/*-{before,after}.json`

## Target functions

| Function | File | Before CC | After CC | Δ |
|---|---|---:|---:|---:|
| `resolveHost` | `packages/pi/launcher/src/index.js` | 29 | 26 | −3 |
| `resolveBinary` | same | 22 | 17 | −5 |
| `resolveCodemodeAddon` | same | 23 | 18 | −5 |
| `update_paths` | `crates/ast-sgrep-core/src/index.rs` | 18 | 15 | −3 |
| `should_skip_watch_path` (new) | same | — | 4 | +4 |

## New helpers (launcher)

| Function | After CC |
|---|---:|
| `isPathFallbackError` | 1 |
| `isOptionalHostMiss` | 2 |
| `readJsonFile` | 2 |
| `assertPackageFileChecksum` | 4 |

## Scope ΣCC

| Scope | Before | After | Δ |
|---|---:|---:|---:|
| launcher `index.js` | 106 | 102 | −4 |
| core `index.rs` | 241 | 242 | +1 (justified free-fn base) |
| **combined touched** | **347** | **344** | **−3** |

Repo baseline ΣCC **6022** not re-scanned this wave.

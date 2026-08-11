# Parity report — Pass 6 (boolean / nesting)

## Commands run

| Check | Command | Result |
|---|---|---|
| Extension suite | `cd packages/pi/extension && npm test` | **88 passed** (0 fail) |
| Freshness paths | covered by `test/runtime.test.ts` ▶ per-root index freshness (11 cases) | **green** |
| ensureFresh tools path | `test/tools.test.ts` BACKEND_UNAVAILABLE from ensureFresh | **green** |
| Code-mode hit shape | `test/code-mode.test.ts` (asSearchResponse / hit parsing) | **green** |

## Behavior preserved (ensureFresh)

From existing suite (differential via characterization, not hand-rolled golden):

- Lazy index missing root + dedupe immediate repeats
- Safe reindex only when explicitly incompatible
- Lease expiry reconciles external create/modify/delete via **incremental** index (not force)
- Dirty after write paths; concurrent alias coalescing; failed/cancelled in-flight clear + retry
- First use indexes even when status reports ready
- Dirtiness recorded during in-flight refresh preserved
- Symlink cwd + non-existent affected path canonicalize
- Unknown status refuses silent query (`INDEX_STATUS_UNKNOWN`)

Combined branch `needsIncrementalIndex` includes expired — same force:false `runIndex` as former separate expired arm.

## Pre-existing (not pass-6)

Core lib fixture drift (`SearchHit.resolution`) noted in pass 3–5; not re-run as out of extension scope.

## CUT_BRANCHES_RESULT

`partial` — boolean wave on ensureFresh (+ nesting polish); ΣCC down; residuals deferred.

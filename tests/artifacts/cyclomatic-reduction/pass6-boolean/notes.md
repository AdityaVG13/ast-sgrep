# Pass 6 — Boolean / nesting density transforms

Techniques: `combine_predicates`, `decompose_conditional`, replace nested conditional with compound guard.
Scope: `packages/pi/extension/src` (boolean-heavy residual; no lookup/extract wave re-do).

## Transforms

### 1. `ensureFresh` — `packages/pi/extension/src/runtime.ts` (primary)

- **Before:** CC 23 — health ladder + duplicated incremental-index branches (missing/dirty vs expired) each with nativeCall/CLI dispatch.
- **After:** CC 10.
- **combine_predicates:** `needsIncrementalIndex(health, wasInitialized, dirty, expired)` folds the two identical force:false paths (missing/first-run/dirty **or** lease expiry). Domain varieties kept: incompatible still force-rebuilds; ready+clean+unexpired still no-ops.
- **decompose_conditional / accidental dispatch:**
  - `probeIndexHealth` — inspect hook + status probe + incompat operational map
  - `runIndex(force)` — single native vs CLI argv dispatcher (force true → reindex, false → index)
- **Ashby Keep:** health states `incompatible | missing | ready` and lease/dirty/inFlight concurrency not removed.

### 2. `assertVersionTriple` — same file

- Nested `if (present) { if (mismatch) throw }` → compound guard `(present || required) && mismatch`.
- CC 7 → 7 (nesting depth only).

### 3. `asSearchResponse` — `packages/pi/extension/src/code-mode.ts`

- Compound `hit_count !== undefined && (invalid…)` → outer guard + inner validity if.
- CC 10 → 10 (nesting density only).

## Rejected attempts (metric / nesting non-wins)

| Attempt | Evidence | Resolve |
|---|---|---|
| `summarizeCodemode` extract `pluralLabel` + lead/via helpers | file ΣCC +3 to +5; parent 15→17 with sequential ifs | **Refuse** (Kolmogorov dump) |
| `summarizeCodemode` sequential ifs only (no extract) | parent 15→17 under lizard | **Refuse** |
| `wireLinesValid` early-return form of && chain | CC 6→12 | **Refuse** (revert) |
| `isInvalidHitCount` extract | parent −4 + helper +5 = Σ +1 | **Refuse** |
| `isCleanLease` / `isLeaseExpired` named predicates | pure re-home; removed after measure | **Refuse** as helpers (logic kept inline) |

## Public API

Unchanged: `FreshnessCoordinator.ensureFresh`, internal helpers private to module.

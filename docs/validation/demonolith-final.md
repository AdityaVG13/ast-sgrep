# Demonolith final report (PR-facing)

**Run:** `2026-08-13-ast-sgrep-wt-demonolith-1`  
**Branch:** `refactor/de-monolithize-isomorphic`  
**Mode:** Standard (execute confirmed extractions)  
**Workspace:** `ast-sgrep-wt-demonolith__demonolith_workspace`  
**Pass:** 14 — finalize leave-alone + run report (docs only; no further extracts)

## Executive summary

Eight evidence-backed seams (EXP-001..008) were confirmed and extracted isomorphically onto this branch. Full-suite behavior held at **488 passed / 0 failed / 4 ignored** and public-api set diffs stayed **0 removals / 0 additions** after each extract. Compile-time / peak-RSS / criterion benches were **SKIPPED** (Phase 3 incomplete) — no invented metrics. Remaining `index.rs` (1034 `wc -l`) is **leave-alone-with-rationale** (B4 Indexer hub; F-003 injects stay with hub); further split would be aesthetic Indexer-method slicing. B9 catalogs remain leave-alone as previously decided.

## EXP-001..008

| EXP | Target | Verdict | Extract commit | Suite | API (rem/add) |
|---|---|---|---|---|---|
| EXP-001 | sqlite query/read → `sqlite/queries.rs` (+ dir façade) | SEAM_CONFIRMED | `4c89f4c` (dir `4cd1c7a`) | 488/0/4 | 0/0 |
| EXP-002 | index recovery → `index_recovery.rs` | SEAM_CONFIRMED | `05994e4` | 488/0/4 | 0/0 |
| EXP-003 | search finish/ranking → `search/finish.rs` | SEAM_CONFIRMED | `070537d` | 488/0/4 | 0/0 |
| EXP-004 | MCP sandbox leaf → `sandbox.rs` | SEAM_CONFIRMED | `182e76f` | 488/0/4 | 0/0 |
| EXP-005 | metamorphic `mr_pred_*` → `metamorphic_preds.rs` | SEAM_CONFIRMED | `c32f377` | 488/0/4 | 0/0 |
| EXP-006 | sqlite writes → `sqlite/writes.rs` | SEAM_CONFIRMED | `620336d` | 488/0/4 | 0/0 |
| EXP-007 | index prepare/hash → `index_prepare.rs` | SEAM_CONFIRMED | `2eaa414` | 488/0/4 | 0/0 |
| EXP-008 | index watch-path → `index_watch.rs` | SEAM_CONFIRMED | `2152e30` | 488/0/4 | 0/0 |

Per-EXP write-ups: `docs/validation/demonolith-exp-00N.md`. Workspace logs: `phase5_experiment_results/EXP-00N.*`.

Proof class: `--quick` (suite + `cargo +nightly public-api`); GATE 4/5 skipped uniformly.

## Before / after `wc -l`

Measured **now** on this branch vs `git show origin/main:<path> | wc -l` (2026-08-13).

| Path | `origin/main` | Now | Notes |
|---|---:|---:|---|
| `crates/ast-sgrep-core/src/store/sqlite.rs` | **1745** | — (absent) | Became directory module |
| `crates/ast-sgrep-core/src/store/sqlite/mod.rs` | — (absent) | **800** | Façade + open/schema/tx hub |
| `crates/ast-sgrep-core/src/index.rs` | **1503** | **1034** | After EXP-002/007/008 |
| `crates/ast-sgrep-core/src/search/mod.rs` | **1196** | **846** | After EXP-003 |
| `crates/ast-sgrep-mcp/src/lib.rs` | **1155** | **1005** | After EXP-004 |

Satellites (now only; not on `origin/main` as separate files): `sqlite/queries.rs` 505, `sqlite/writes.rs` 461, `index_recovery.rs` 168, `index_prepare.rs` 271, `index_watch.rs` 71, `search/finish.rs` 366, `mcp/sandbox.rs` 162, `tests/core/metamorphic_preds.rs` 53.

## Leave-alone list

Full rationale: [`demonolith-leave-alone.md`](demonolith-leave-alone.md).

| File | Decision |
|---|---|
| `store/sqlite/mod.rs` (+ queries/writes satellites) | leave-alone (under / post-EXP hub) |
| `search/mod.rs` | leave-alone (F-005 caches; under soft) |
| **`index.rs`** | **leave-alone-with-rationale** (B4 hub; F-003 injects stay; 1034 after confirmed cuts; no aesthetic Indexer-method slicing) |
| `mcp/lib.rs` | borderline leave-alone |
| `tests/core/metamorphic.rs` | leave-alone (B9 catalog; already decided) |
| `tests/cli/machine_contracts.rs` | borderline leave-alone (B9; already decided) |
| `semantic_ann.rs`, `packages/pi/extension/src/runtime.ts` | leave-alone (under soft) |
| `packages/pi/extension/dist/**` | B10 generated — never hand-split |

**Residual still-split:** empty. F-003 FORCE_SIDECAR remains escalate/leave-alone with the Indexer hub (not a product extract).

## Compile / bench

| Gate | Status |
|---|---|
| Full suite (per EXP) | **488 / 0 / 4** (matched Phase 3 baseline) |
| Public API | **0 removals / 0 additions** (per EXP) |
| Criterion benches | **SKIPPED** — Phase 3 incomplete; no invented numbers |
| Compile time + peak RSS | **SKIPPED** — Phase 3 incomplete; no invented numbers |

## No invented metrics

This report records only measured `wc -l`, documented EXP suite/API outcomes, and explicit SKIPPED gaps. Do not backfill RSS/bench deltas without a SAME-MACHINE Phase 3 re-baseline.

# Demonolith EXP-002 — index.rs recovery helpers seam

**Verdict:** SEAM_CONFIRMED  
**Run:** 2026-08-13-ast-sgrep-wt-demonolith-1  
**Branch:** `refactor/de-monolithize-isomorphic`  
**Finding:** F-001 (index.rs C5+C6 corrupt recovery / quarantine)

## What moved

| Step | Commit | Change |
|---|---|---|
| 1 | `05994e4` | Recovery leaf helpers → `crates/ast-sgrep-core/src/index_recovery.rs`; `mod index_recovery;` in `lib.rs` |

**Moved into `index_recovery.rs`:** `suffixed_path`, `SQLITE_SIDECAR_SUFFIXES`, `remove_file_if_present`, `remove_derived_sidecars`, `quarantine_corrupt_index`, `recovery_lock`, `replacement_generation_seed`, `recover_corrupt_index` (`pub(crate)`).

**Left in `index.rs`:** `open_index_store`, `quick_check` (Indexer main open path; widened `private → pub(crate)` so recovery can re-check), `FORCE_SIDECAR_REBUILD_ERR` / `ForceSidecarRebuildErr` / `force_sidecar_rebuild_err` (F-003), Indexer hub, prepare/hash (F-002), watch path helpers (F-004), public types and `indexed_rel_path`.

`lib.rs` still `pub use index::{Indexer, IndexOptions, ...}` — no import-path change for consumers.

## Evidence

### Behavior
- Command: `rch exec -- cargo test --workspace --no-fail-fast` (spark-1672; no `--offline`)
- Result: **488 passed / 0 failed / 4 ignored** (exit 0)
- Matches Phase 3 baseline `baseline_tests.json` (488/0/4)

### Public API
- Command: `rch exec -- cargo +nightly public-api --simplified -p ast-sgrep-core` and `-p ast-sgrep-mcp`
- Diff vs workspace `api_snapshot_before.txt`: **0 removals, 0 additions** (set compare of package bodies)
- `index_recovery` is crate-private (`mod index_recovery;`) — no public leak

### Structural
- No new `Box<dyn` / `Arc<dyn` / trait-object indirection
- Visibility widenings (crate-private only): `open_index_store`, `quick_check`, `recover_corrupt_index`

### Gate script / SKIPPED
- `isomorphism-gate.sh --quick` GATE 4/5: **SKIPPED** (Phase 3 benches/compile-RSS incomplete; `--quick`)
- Script overall FAIL from environment mismatches (same class as EXP-001: gate1 parsed 51 tests under private target / exclusion, api-snapshot format ≠ Phase 3 composed baseline, missing dep_graph_baseline for pre-existing workspace cycle, gate6 docs prose vs `origin/main`); binding proof is the manual suite + public-api runs above
- Workspace log: `ast-sgrep-wt-demonolith__demonolith_workspace/phase5_experiment_results/EXP-002.log`

## Non-goals (this pass)
- F-002 prepare/hash extraction  
- F-003 FORCE_SIDECAR ownership  
- F-004 watch path helpers  
- sqlite (already EXP-001), search/mod.rs, mcp, tests  

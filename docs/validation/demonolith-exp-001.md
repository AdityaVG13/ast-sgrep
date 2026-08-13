# Demonolith EXP-001 — sqlite query/read seam

**Verdict:** SEAM_CONFIRMED  
**Run:** 2026-08-13-ast-sgrep-wt-demonolith-1  
**Branch:** `refactor/de-monolithize-isomorphic`  
**Finding:** F-001 (sqlite.rs query/read vs writers)

## What moved

| Step | Commit | Change |
|---|---|---|
| 1 | `4cd1c7a` | `store/sqlite.rs` → `store/sqlite/mod.rs` (`git mv`); `#[path]` depth +1 for existing unit-test includes |
| 2 | `4c89f4c` | Read/query `impl IndexStore` methods → `store/sqlite/queries.rs`; `mod queries;` in façade |

**Moved into `queries.rs`:** `file_hash`, `all_file_paths`, `has_file_with_prefix`, `status`, `indexed_line_count`, `indexed_line_count_at_least`, `all_indexed_lines`, `indexed_excerpt_in_range`, semantic chunk/legacy embedding readers, symbol/call/import/pattern/file text readers, `map_sorted_files`, `file_exists`.

**Left in `sqlite/mod.rs`:** open/schema/meta/tx, upsert/write pipeline through `refresh_lines_only`, `remove_file`, `remove_files_with_prefix`, DTOs/types, `FORCE_*` thread-locals.

`store/mod.rs` `pub use sqlite::{IndexStore, ...}` unchanged.

## Evidence

### Behavior
- Command: `rch exec -- cargo test --workspace --no-fail-fast` (spark-1672; no `--offline`, same as Phase 3)
- Result: **488 passed / 0 failed / 4 ignored** (exit 0)
- Matches Phase 3 baseline `baseline_tests.json` (488/0/4)

### Public API
- Command: `rch exec -- cargo +nightly public-api --simplified -p ast-sgrep-core` and `-p ast-sgrep-mcp`
- Diff vs workspace `api_snapshot_before.txt`: **0 removals, 0 additions** (set compare of package bodies)

### Structural
- No new `Box<dyn` / `Arc<dyn` / trait-object indirection; only pre-existing `&dyn ToSql` sites relocated with their methods
- One inherent `impl IndexStore` split across sibling modules (no new traits)

### Gate script / SKIPPED
- `isomorphism-gate.sh --quick` GATE 4/5: **SKIPPED** (Phase 3 benches/compile-RSS incomplete; `--quick`)
- Script overall FAIL from environment mismatches (concurrent rch exclusion, api-snapshot format vs Phase 3 composed baseline, missing dep_graph_baseline, gate6 docs prose vs `origin/main`); binding proof is the manual suite + public-api runs above
- Workspace log: `ast-sgrep-wt-demonolith__demonolith_workspace/phase5_experiment_results/EXP-001.log`

## Non-goals (this pass)
- F-002 upsert/write extraction  
- F-003 semantic-only further split  
- index.rs / search / mcp / test monoliths  

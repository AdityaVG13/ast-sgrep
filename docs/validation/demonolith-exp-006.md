# Demonolith EXP-006 — sqlite write pipeline seam

**Verdict:** SEAM_CONFIRMED  
**Run:** 2026-08-13-ast-sgrep-wt-demonolith-1  
**Branch:** `refactor/de-monolithize-isomorphic`  
**Finding:** F-002 (sqlite.rs upsert/write vs open/schema/tx)

## What moved

| Step | Commit | Change |
|---|---|---|
| 1 | `620336d` | Write `impl IndexStore` methods → `store/sqlite/writes.rs`; `mod writes;` in façade |

**Moved into `writes.rs`:** `upsert_file`, `persist_embed_cache_side_effects`, `refresh_lines_only`, `upsert_file_inner`, `upsert_file_row`, `insert_each`, `insert_lines`, `insert_symbols`, `insert_semantic_chunks`, `persist_embed_metadata`, `insert_callers`, `insert_pattern_nodes`, `insert_imports`, `remove_file`, `remove_files_with_prefix`.

**Left in `sqlite/mod.rs`:** open/schema/meta/lexicon/tx (`begin`/`commit`/`rollback`/`with_file_tx`/bulk), `clear_all_data`, `FORCE_*` thread-locals, DTOs/types, connection accessors.

`store/mod.rs` `pub use sqlite::{IndexStore, ...}` unchanged. `index.rs` untouched. Tests untouched.

**Path-depth only:** `super::sql::escape_glob_literal` → `super::super::sql::escape_glob_literal` inside moved `remove_files_with_prefix`.

## Under-1k leave-alone (notes only)

| File | Code LOC | Action |
|---|---|---|
| `crates/ast-sgrep-core/src/semantic_ann.rs` | ~631 | UNDER 1000 — not split |
| `packages/pi/extension/src/runtime.ts` | ~775 | UNDER 1000 — not split |

## Evidence

### Behavior
- Command: `rch exec -- cargo test --workspace --no-fail-fast` (spark-1672)
- Result: **488 passed / 0 failed / 4 ignored** (exit 0)
- Matches Phase 3 baseline 488/0/4

### Public API
- Command: `cargo +nightly public-api --simplified -p ast-sgrep-core` and `-p ast-sgrep-mcp`
- Diff vs workspace `api_snapshot_before.txt` (set compare of package bodies): **0 removals, 0 additions**

### Structural
- No new `Box<dyn` / `Arc<dyn` / trait-object indirection
- Second inherent `impl IndexStore` sibling to `queries.rs` (no new traits)
- Post-extract `wc -l`: `sqlite/mod.rs` **800**, `sqlite/writes.rs` **461**

### Gate script / SKIPPED
- GATE 4/5: **SKIPPED** (Phase 3 benches/compile-RSS incomplete; `--quick` class)
- Binding proof is the manual suite + public-api runs above (same class as EXP-001..005)
- Workspace log: `ast-sgrep-wt-demonolith__demonolith_workspace/phase5_experiment_results/EXP-006.log`

## Non-goals (this pass)
- `index.rs` (do not touch)
- Tests / reformatting / pub API changes / new dyn
- Further F-003 semantic-only split inside writes

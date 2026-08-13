# Demonolith EXP-007 — index.rs prepare/hash helpers seam

**Verdict:** SEAM_CONFIRMED  
**Run:** 2026-08-13-ast-sgrep-wt-demonolith-1  
**Branch:** `refactor/de-monolithize-isomorphic`  
**Finding:** F-002 (index.rs prepare/hash/extract cluster)

## What moved

| Step | Commit | Change |
|---|---|---|
| 1 | (this commit) | Prepare/hash leaf helpers → `crates/ast-sgrep-core/src/index_prepare.rs`; `mod index_prepare;` in `lib.rs` |

**Moved into `index_prepare.rs`:** `ExtractedRows`, `PreparedFile`, `PrepareOutcome`, `UpsertMaterial`, `body_structure_hash`, `is_trailing_trivia_line` (module-private), `hash_content`, `materialize_upsert`, `prepare_file`, `rows_from_extraction`, `should_prune_missing_files`, `system_time_to_parts` (`pub(crate)` as needed).

**Left in `index.rs`:** `Indexer` impl and public types, `FORCE_SIDECAR*` (F-003), `indexed_rel_path`, `split_content_lines` / `SplitLines`, watch helpers `normalize_watch_path` / `canonicalize_affected_path` (pub) / `should_skip_watch_path` (F-004), `open_index_store` / `quick_check`, unit-test `#[path]` includes.

`lib.rs` still `pub use index::{Indexer, IndexOptions, ...}` — no import-path change for consumers. `index_prepare` is crate-private (`mod index_prepare;`).

Post-extract `wc -l`: `index.rs` **1095**, `index_prepare.rs` **270**.

## Evidence

### Behavior
- Command: `rch exec -- cargo test --workspace --no-fail-fast` (spark-1672)
- Result: **488 passed / 0 failed / 4 ignored** (exit 0)
- Matches Phase 3 baseline 488/0/4

### Public API
- Command: `cargo +nightly public-api --simplified -p ast-sgrep-core` and `-p ast-sgrep-mcp`
- Diff vs workspace `api_snapshot_before.txt` (set compare of package bodies): **0 removals, 0 additions**
- `index_prepare` does not appear in public API

### Structural
- No new `Box<dyn` / `Arc<dyn` / trait-object indirection
- Indexer fields remain private; no field visibility widening
- Supporting types (`PreparedFile` / `PrepareOutcome` / `UpsertMaterial` / `ExtractedRows`) are `pub(crate)` only
- Tests untouched; `super::body_structure_hash` / `super::should_prune_missing_files` remain via private `use` re-imports in `index.rs`

### Gate script / SKIPPED
- GATE 4/5: **SKIPPED** (Phase 3 benches/compile-RSS incomplete; `--quick` class)
- Binding proof is the manual suite + public-api runs above (same class as EXP-001..006)
- Workspace log: `ast-sgrep-wt-demonolith__demonolith_workspace/phase5_experiment_results/EXP-007.log`

## Non-goals (this pass)
- F-003 FORCE_SIDECAR ownership  
- F-004 watch path helpers  
- sqlite / search / mcp / tests  

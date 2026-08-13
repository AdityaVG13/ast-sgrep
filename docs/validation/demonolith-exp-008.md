# Demonolith EXP-008 — index.rs watch-path helpers seam

**Verdict:** SEAM_CONFIRMED  
**Run:** 2026-08-13-ast-sgrep-wt-demonolith-1  
**Branch:** `refactor/de-monolithize-isomorphic`  
**Finding:** F-004 (index.rs watch-path helpers)

## What moved

| Step | Commit | Change |
|---|---|---|
| 1 | (this) | Watch-path leaf helpers → `crates/ast-sgrep-core/src/index_watch.rs`; `mod index_watch;` in `lib.rs` |

**Moved into `index_watch.rs`:** `normalize_watch_path` (`pub(crate)`), `canonicalize_affected_path` (`pub`), `should_skip_watch_path` (`pub(crate)`).

**Left in `index.rs`:** `Indexer` impl hub including `update_paths` (uses helpers via private import), FORCE_SIDECAR* (F-003 escalate), `indexed_rel_path`, `split_content_lines` / `SplitLines`, `open_index_store` / `quick_check`, public types, unit-test `#[path]` includes.

**Façade:** `index.rs` has `pub use crate::index_watch::canonicalize_affected_path` so `crate::index::canonicalize_affected_path` and `lib.rs` `pub use index::{canonicalize_affected_path, ...}` stay valid. `index_watch` is crate-private (`mod index_watch;` — not `pub`).

Post-extract `wc -l`: `index.rs` **1034**, `index_watch.rs` **71**.

## Evidence

### Behavior
- Command: `rch exec -- cargo test --workspace --no-fail-fast` (spark-1672)
- Result: **488 passed / 0 failed / 4 ignored** (exit 0)
- Matches Phase 3 baseline 488/0/4

### Public API
- Command: `cargo +nightly public-api --simplified -p ast-sgrep-core` and `-p ast-sgrep-mcp`
- Diff vs workspace `api_snapshot_before.txt` (set compare of package bodies): **0 removals, 0 additions**
- `canonicalize_affected_path` remains at:
  - `ast_sgrep_core::canonicalize_affected_path`
  - `ast_sgrep_core::index::canonicalize_affected_path`
- `index_watch` does not appear in public API

### Structural
- No new `Box<dyn` / `Arc<dyn` / trait-object indirection
- Indexer fields remain private; no field visibility widening
- Hub method `Indexer::update_paths` not moved
- FORCE_SIDECAR* not moved (F-003 escalate)

### Gate script / SKIPPED
- GATE 4/5: **SKIPPED** (Phase 3 benches/compile-RSS incomplete; `--quick` class)
- Binding proof is the manual suite + public-api runs above (same class as EXP-001..007)
- Workspace log: `ast-sgrep-wt-demonolith__demonolith_workspace/phase5_experiment_results/EXP-008.log`

## Non-goals (this pass)
- F-003 FORCE_SIDECAR ownership  
- Aesthetic Indexer impl splits / `types.rs` dump  
- sqlite / search / mcp / tests  

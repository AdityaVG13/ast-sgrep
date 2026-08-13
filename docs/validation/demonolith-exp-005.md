# Demonolith EXP-005 — metamorphic mr_pred_* leaf helpers

**Verdict:** SEAM_CONFIRMED  
**Run:** 2026-08-13-ast-sgrep-wt-demonolith-1  
**Branch:** `refactor/de-monolithize-isomorphic`  
**Finding:** F-001 in phase2_findings_tests-core-metamorphic-rs.md (C5; cohesion 0.88)

## What moved

| Step | Commit | Change |
|---|---|---|
| 1 | `c32f377` | `HitKey` + eight `mr_pred_*` helpers → `tests/core/metamorphic_preds.rs`; `#[path]` include from `metamorphic.rs` |

**Moved into `metamorphic_preds.rs` (`pub(super)`):** `HitKey`, `mr_pred_limit_subset`, `mr_pred_probe_monotone`, `mr_pred_scale_invariance`, `mr_pred_lang_filter_subset`, `mr_pred_reindex_idempotent`, `mr_pred_search_flat_prefix`, `mr_pred_term_order_equiv`, `mr_pred_corpus_add_orthogonal`.

**Left in `metamorphic.rs`:** MR catalog tests (`fn mr_*`), mutation-kill matrix / suite tests, proptest config / fixture helpers (`mr_proptest_config`, `ensure_nonzero_rows`, `index_and_searcher`, …). Cargo `[[test]]` path stays `../../tests/core/metamorphic.rs` (no directory conversion).

Wire-up:

```rust
#[path = "metamorphic_preds.rs"]
mod metamorphic_preds;
use metamorphic_preds::*;
```

## Evidence

### Behavior
- Command: `rch exec -- cargo test -p ast-sgrep-core --test metamorphic -- --nocapture` (spark-1672)
- Result: **22 passed / 0 failed / 0 ignored** (exit 0)
- Command: `rch exec -- cargo test --workspace --no-fail-fast` (spark-1672)
- Result: **488 passed / 0 failed / 4 ignored** (exit 0)
- Matches Phase 3 baseline 488/0/4; no new `#[test]` (helpers only)

### Public API
- Command: `cargo +nightly public-api --simplified -p ast-sgrep-core` and `-p ast-sgrep-mcp`
- Diff vs workspace `api_snapshot_before.txt` (set compare of package bodies): **0 removals, 0 additions**
- Test-only `#[path]` module; no crate `pub` surface change

### Structural
- No product-crate edits
- No reformat of moved bodies (visibility `pub(super)` + module header/import only)
- F-003 MR catalog leave-alone honored; `machine_contracts.rs` untouched (F-002 later)

### Gate script / SKIPPED
- GATE 4/5: **SKIPPED** (Phase 3 benches/compile-RSS incomplete; `--quick` class)
- Binding proof is the manual suite + public-api runs above (same class as EXP-001..004)
- Workspace log: `ast-sgrep-wt-demonolith__demonolith_workspace/phase5_experiment_results/EXP-005.log`

## Non-goals (this pass)
- Splitting the MR test catalog (F-003 leave-alone)  
- `machine_contracts.rs` (F-002)  
- Product sqlite/index/search/mcp monoliths  

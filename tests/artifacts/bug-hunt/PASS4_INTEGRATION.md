# Pass 4 — Integration (after surface + core)

**Date:** 2026-08-07  
**Scope:** supervisor worker nonce; semantic_ann `mark_semantic_ivf_stale`; store `restore_synchronous` / `with_file_tx` poison / `clear_all_data` / `semantic_chunks_by_ids`; `bench_suite` `percentile_99`.

## Commands + results

Isolated target dir used for the final full re-run under concurrent workspace cargo lock contention (`CARGO_TARGET_DIR=target-pass4`). Earlier partial runs on default `target/` matched the same outcomes where they completed.

| Command | Result |
|---|---|
| `cargo test -p ast-sgrep-core --test durability_epics` | **ok** 18 passed |
| `cargo test -p ast-sgrep-core --test store_pragmas` | **ok** 5 passed |
| `cargo test -p ast-sgrep-core --test store_delete` | **ok** 6 passed, 1 ignored (timing quarantine) |
| `cargo test -p ast-sgrep-cli --lib worker_nonce` | **ok** 1 passed |
| `cargo test -p ast-sgrep-core --lib pass3_deep_core` | **ok** 2 passed |
| `cargo test -p ast-sgrep-core --lib restore_synchronous` | **ok** 4 passed |
| `cargo test -p ast-sgrep-core --lib percentile_99` | **ok** 3 passed |

**Totals for this gate:** 39 passed, 0 failed, 1 ignored.

## Regression fixed during this pass

Compile break blocked core lib tests:

- `SearchHit` gained `confidence` (vh65) but several struct initializers omitted the field.
- **Root fix:** set `confidence: 0.0` in `SearchHit::base` (production constructors) and matching test fixtures in:
  - `crates/ast-sgrep-core/src/search/types.rs`
  - `crates/ast-sgrep-core/src/fusion.rs`
  - `crates/ast-sgrep-core/tests/search_correctness_epics.rs`
  - `crates/ast-sgrep-plugins/tests/capsule_format.rs`

No commit (per mission).

## Related integration surfaces reviewed (search gen fence + store)

1. **`Searcher::fenced` + `cached`** (`search/mod.rs`)  
   - `BEGIN DEFERRED` fail-closed; gen re-read after compute; response cache only stores when pre/post gen match.  
   - Cache keys include full options identity; poison path clears cache.  
   - No new bug: gen fence + response cache interaction looks consistent with durability_epics `hybrid_response_cache_invalidates_on_index_generation`.

2. **Semantic cache versioning** (`search/passes/embed.rs` + store gens)  
   - Cache key binds `index_data_version` + `semantic_data_version` + `max_id` + backend + lang filter.  
   - Mutations under review bump both gens where semantic content changes; `clear_all_data` bumps both inside the same `with_file_tx` as the wipe.  
   - No new bug.

3. **IVF stale gate vs store mutations** (`semantic_ann::mark_semantic_ivf_stale`, `remove_file`, upsert path)  
   - Stale mark fails closed (meta + sidecar delete + session cache clear).  
   - Covered by durability_epics + store_delete.  
   - **Observation (not filed):** `clear_all_data` bumps generations but does not call `mark_semantic_ivf_stale`. Fingerprint includes generation, so load fails closed / falls back; residual hygiene asymmetry vs `remove_file`, not a proven wrong-result path. No bead filed.

## Beads

None filed this pass (no new confirmed correctness bugs beyond the compile fix above).

# Commands — Wave 2 Pass 6

```text
rch check
# ✓ RCH ready

RCH_CANONICAL_PROJECT_ROOT=/Users/aditya \
rch exec -- env CARGO_TARGET_DIR=…/target-wave2-pass6 \
  cargo test -p ast-sgrep-core --test generation_swap -- --nocapture
# test result: ok. 5 passed; 0 failed
#   missing_active_generation_refuses_stale_legacy_fallthrough ... ok
#   a_destroyed_candidate_build_leaves_the_active_generation_serving ... ok
#   empty_candidate_is_refused_and_active_pointer_survives ... ok
#   reindex_activates_a_new_generation_and_retains_the_previous ... ok
#   corrupt_candidate_sidecar_blocks_activation ... ok
```

Dual evidence for R-MISSING-GEN-FALLTHROUGH:

1. **Source:** `crates/ast-sgrep-core/src/store/mod.rs` `try_index_db_path` previously fell through to flat `index.db` when `active.json` named a missing generation.
2. **Executable:** new `missing_active_generation_refuses_stale_legacy_fallthrough` (RCH green).

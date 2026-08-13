# Commands — Loop 27 independent pins

Freeze: `5ddd43b8cf5c3aa394bae163375242a8ed5e4ddc` (dirty tree; Pi/beads leftover untouched).
Env: `PATH=$HOME/.local/bin:$PATH` · `RCH_CANONICAL_PROJECT_ROOT=/Users/aditya`
Timestamp UTC: 2026-08-12T18:20–18:37Z

```bash
rch exec -- cargo test -p ast-sgrep-mcp --lib invalidates -- --nocapture
# ok. 3 passed (index_repo_invalidates_searcher_on_index_err,
#               external_writer_generation_invalidates_warm_searcher,
#               index_repo_invalidates_searcher_after_disk_mutation)

rch exec -- cargo test -p ast-sgrep-codemode --lib -- --nocapture
# ok. 3 passed (index_repo_invalidates_searcher_on_index_err,
#               external_writer_generation_invalidates_warm_searcher,
#               foreign_root_is_rejected_under_session_workspace)

rch exec -- cargo test -p ast-sgrep-core --lib writer_generation -- --nocapture
# ok. 3 passed

rch exec -- cargo test -p ast-sgrep-cli --lib doctor_surfaces -- --nocapture
# ok. 3 passed (FastUnsafe status/cli + balanced silent)

rch exec -- cargo test -p ast-sgrep-core --test generation_swap -- --nocapture
# ok. 5 passed (incl. missing_active_generation_refuses_stale_legacy_fallthrough)

rch exec -- cargo test -p ast-sgrep-core --test semantic_chunk_migration -- --nocapture
# ok. 4 passed (incl. newer_than_binary_schema_refuses_open)

rch exec -- cargo test -p ast-sgrep-cli --test watch_incremental -- --nocapture
# ok. 2 passed (incl. update_paths_refuses_symlink_escape_into_index)
```

All remote exits **0**. No product edits this pass.

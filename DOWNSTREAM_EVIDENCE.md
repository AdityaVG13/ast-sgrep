# Downstream evidence — P1 correctness beads (PR #20)

Branch: `fix/p1-correctness-batch`  
Evidence date: 2026-08-03  
Do not touch `.beads` (tracker updates deferred to orchestrator).

## Scope

| Bead | Status | Hard evidence |
|------|--------|---------------|
| `ast-sgrep-28vo` | **implemented** | `p1_correctness_batch::clear_all_data_wipes_embed_meta_keeps_root_whitelist`, `is_unchanged_auto_does_not_match_concrete_backend`; durability `clear_all_data_is_transactional_and_complete` |
| `ast-sgrep-naiv` | **implemented** | unit `store::sqlite::rollback_tests::*` |
| `ast-sgrep-21pn` | **implemented** | unit `search::passes::embed::tests::ivf_integrity_mismatch_errors_not_ok` |
| `ast-sgrep-hdwh` | **implemented** | unit `search::tests::response_cache_may_insert_rejects_gen_skew_and_pragma_fail`; PR21 tip still used unlock-compute-reinsert without post-gen check / could set cache under wrong gen — fixed here |
| `ast-sgrep-kqhp` | **implemented** | `p1_correctness_batch::indexed_rel_path_rejects_non_utf8`; policy in `docs/index-consistency.md` |

Store-adjacent from the naiv/28vo/21pn cluster: no additional open beads beyond the listed five clearly belonged on this PR beyond what was already closed in prior durability work.

## hdwh vs PR21

PR21 tip (`test/quality-batch-e2hc-19-oxbj`) still has ResponseCache unlock-compute-reinsert that can adopt a pre-compute gen after concurrent reindex, and `index_gen` uses `unwrap_or(0)`. This branch already failed closed on gen read (jiyy.4); this change adds gen-safe insert (`response_cache_may_insert` + re-read after compute). **Not skipped.**

## Commands

```bash
export CARGO_BUILD_RUSTC_WRAPPER= RUSTC_WRAPPER=
/usr/local/cargo/bin/cargo test -p ast-sgrep-core --test p1_correctness_batch --test durability_epics --test response_cache_version --test semantic_cache_version --test semantic_ivf_roundtrip --test store_pragmas --test store_delete
/usr/local/cargo/bin/cargo test -p ast-sgrep-core --lib
```

## Implementation notes

- **28vo:** `CLEAR_ALL_SQL` deletes all meta except whitelist `root`; `is_unchanged` requires exact `embed_backend` string match (Auto ≠ concrete).
- **naiv:** `execute_rollback` propagates errors; file/bulk tx flags cleared only after successful COMMIT/ROLLBACK.
- **21pn:** `check_ivf_embed_integrity` returns `Err` on count/dim/ids skew; missing sidecar remains `Ok(None)` for flat fallback.
- **hdwh:** insert only when pre/post gen match and cache is not a newer populated generation; PRAGMA failure disables insert.
- **kqhp:** `indexed_rel_path` rejects non-UTF8; walk/watch record failures instead of lossy keys.

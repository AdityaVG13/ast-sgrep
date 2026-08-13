# RESULT — Wave 2 / Pass 4 (HARDEN R-XPROC-MULTIWRITER Option C lite)

```text
SPIN_THE_BLOCK_RESULT:
status: complete
mode: harden
target_root: /Users/aditya/Developer/ast-sgrep
prior_state_leveraged: true
wave: 2
campaign_pass: 4
iteration: 16
product_safe: true
product_source_edits: yes
residual_closed: R-XPROC-MULTIWRITER
product_decision: Option C lite (durable writer_generation stamp + peer poll)
bead: ast-sgrep-rca-residuals-sp6p.3
technique: Indexer bumps stamp after durable mutation; MCP/CM Searcher caches poll and reopen; status reports writer_generation
axes_changed: 4
axes: representation:interleaving | observer:scheduler | time:reorder | scale:process
frozen_revision_pass1: 62ee4b4595ad2433bd16b0ac14747dada612b4d6
head_at_verify: 3eefcd93f3488bab4a53c4b7f5a45766958c4b20
dirty: true
dirty_note: product xproc stamp + incidental SearchHit/SearchResponse test-field fixes for compile; no Pi leftover; no root-jail rework
zerostack: unavailable-fszero-codemode
independent: n/a-this-pass (originator harden; pass 11 dual-evidence CONFIRMED)
braid_resolve: Continue
NEXT_PASS: Harden R-OPS-DOCS-FOOTGUNS (optional hygiene) or seal wave-2 residuals
PRODUCTIVE: true
void_fixture_outcome: n/a mid-wave harden
north_star_probe_outcome: n/a product harden
independent_loop27: n/a
```

## Product decision (recorded)

**Option C lite accepted:** Writers bump `.asgrep/writer_generation` (or stamp beside pinned `ASGREP_INDEX_PATH`). MCP and Code Mode warm Searcher caches read the stamp before reuse and drop/reopen when it changes. Not a flock, lease, or IPC bus.

## Gate

- [x] Writers bump generation artifact (`index_all` after bulk commit, `update_paths` on mutation, deferred sidecar flush, generation activation)
- [x] MCP + CM detect stamp change and invalidate warm Searcher (+ MCP path/snippet maps)
- [x] Cross-generation invalidate test (same-process stamp bump simulating external writer)
- [x] `status` / docs surface `writer_generation`
- [x] RCH verify core writer_generation + mcp cache_tests + codemode --lib
- [x] Axes ≥2 vs pass 3
- [x] No full IPC bus; no re-jail roots; no Pi leftover

## Diff summary (product)

| File | Change |
|------|--------|
| `crates/ast-sgrep-core/src/store/writer_generation.rs` | stamp read/bump + unit tests |
| `crates/ast-sgrep-core/src/store/mod.rs` | export + `IndexStatus.writer_generation` |
| `crates/ast-sgrep-core/src/store/sqlite.rs` | status reports stamp |
| `crates/ast-sgrep-core/src/index.rs` | Indexer advertises after mutations / activation |
| `crates/ast-sgrep-core/src/lib.rs` | re-exports |
| `crates/ast-sgrep-mcp/src/lib.rs` | poll stamp in `searcher_for`; external-writer test |
| `crates/ast-sgrep-codemode/src/session.rs` | poll stamp; external-writer test |
| `docs/index-consistency.md` | Option C lite contract |
| `crates/ast-sgrep-cli/...` | status print + robot-docs note + machine shape |

## Verify

```text
rch exec -- cargo test -p ast-sgrep-core --lib writer_generation
  ok. 3 passed
rch exec -- cargo test -p ast-sgrep-mcp --lib cache_tests
  ok. 4 passed (incl. external_writer_generation_invalidates_warm_searcher)
rch exec -- cargo test -p ast-sgrep-codemode --lib
  ok. 3 passed (incl. external_writer_generation_invalidates_warm_searcher)
```

## Braid

**Freeze(retained) → Axis(interleaving+scheduler+reorder+process) → Enact(Option C lite stamp) → Independent n/a → Residual(R-XPROC closed; R-OPS-DOCS open) → Resolve Continue**

## Failure modes (named)

1. Peer that never polls (third-party Searcher holder) stays stale — out of scope.
2. Nested `index_repo` root without shared `ASGREP_INDEX_PATH` may stamp a different home than MCP workspace root — prefer shared index path.
3. Stamp bump is best-effort (IO failure logs; index still succeeds).

# RESULT — Wave 2 / Pass 2 (HARDEN R-INDEX-ERR-CACHE-SYNC)

```text
SPIN_THE_BLOCK_RESULT:
status: complete
mode: harden
target_root: /Users/aditya/Developer/ast-sgrep
prior_state_leveraged: true
wave: 2
campaign_pass: 2
iteration: 14
product_safe: true
product_source_edits: 4
residual_closed: R-INDEX-ERR-CACHE-SYNC
bead: ast-sgrep-rca-residuals-sp6p.2
technique: A-surface + core caller-duty rustdoc + thread_local mid-sidecar inject
axes_changed: 3
axes: representation:exception-graph | time:degradation | observer:failure-handler
frozen_revision_pass1: 06a6e944e7d1ea826ade9a0c9b7bbd659117d48c
head_at_verify: 06a6e944e7d1ea826ade9a0c9b7bbd659117d48c
dirty: true
dirty_note: beads + Pi leftover untouched; product edit only MCP/CM/core index invalidate
zerostack: unavailable-fszero-codemode
independent: n/a-this-pass (originator harden; pass 10 dual-evidence)
braid_resolve: Continue
NEXT_PASS: Harden R-CM-ROOT-POLICY (jail CM/NAPI root like MCP)
PRODUCTIVE: true
```

## Gate

- [x] Invalidate Searcher/registries on MCP index Err (after Indexer::new + index_all/reindex_all return)
- [x] CM invalidates on index Err
- [x] Core rustdoc caller-cache duty on `index_all`
- [x] Unit test Err-path pin (MCP + CM) via `force_sidecar_rebuild_err`
- [x] Ok-path tests still green
- [x] RCH verify (with `RCH_CANONICAL_PROJECT_ROOT=/Users/aditya`)
- [x] Axes ≥2 vs pass 1 freeze
- [x] No CM-root / xproc / docs / Pi runtime edits

## Diff summary (product)

| File | Change |
|------|--------|
| `crates/ast-sgrep-mcp/src/lib.rs` | `invalidate_after_index_attempt` after Ok **and** Err; new Err-path unit test |
| `crates/ast-sgrep-codemode/src/session.rs` | invalidate before `?` on index result; Err-path unit test |
| `crates/ast-sgrep-core/src/index.rs` | caller-duty rustdoc; thread_local mid-sidecar inject |
| `crates/ast-sgrep-core/src/lib.rs` | export `force_sidecar_rebuild_err` / `ForceSidecarRebuildErr` |

## Verify

```text
rch exec -- cargo test -p ast-sgrep-mcp --lib
  ok. 4 passed (incl. index_repo_invalidates_searcher_on_index_err)
rch exec -- cargo test -p ast-sgrep-codemode --lib
  ok. 1 passed (index_repo_invalidates_searcher_on_index_err)
```

## Braid

**Freeze(retained) → Axis(exception-graph+degradation+failure-handler) → Enact(product invalidate-on-Err) → Independent n/a → Residual(R-INDEX closed; R-CM-ROOT/XPROC/OPS open) → Resolve Continue**

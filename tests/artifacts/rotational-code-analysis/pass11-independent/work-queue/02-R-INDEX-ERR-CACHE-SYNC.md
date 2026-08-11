# R-INDEX-ERR-CACHE-SYNC

| Field | Value |
|-------|-------|
| Residual ID | **R-INDEX-ERR-CACHE-SYNC** |
| Aggregates | CL-MID-SIDECAR-CACHE, RW-MCP-MID-SIDECAR, BY-REGISTRY-STALE, CL-INDEX-FAIL-REGISTRIES, CL-CM-POISON-INV (related), ESC-3 observability slice |
| Severity | **high** |
| Status | **FIX CANDIDATE** — dual-evidence CONFIRMED |
| Pass | 11 independent verification |
| Tracker | markdown only (open beads ≥50; promote to `br` only when queue not flooded **and** implement authorized) |

## Problem

`Indexer::index_all` commits the bulk SQLite transaction **before** `rebuild_dirty_sidecars`. If tantivy/IVF rebuild returns `Err`, `index_all` returns `Err` with **durable** file/symbol rows already committed.

MCP `tool_index_repo` (and CM `index_repo` on success path only) places `invalidate_searcher_cache` + `path_registry` / `emitted_snippets` clear **after** `index_all()?`. On Err, Rust `?` skips invalidation.

Operator/agent symptom: tool `isError: true` text; on-disk index advanced; warm Searcher may still answer from pre-mutation generation; compact path ids may resolve stale maps until success path clears them.

Comment above MCP invalidate claims "always drop cached Searcher" after mutation — control flow does not match the comment on Err.

## Evidence (pass 11)

1. **Order:** `index.rs` ~271–284 — `apply_bulk_write_result` then `rebuild_dirty_sidecars?`.
2. **Commit on Ok write:** `sqlite.rs` `apply_bulk_write_result` ~540–548 → `commit_bulk_tx`.
3. **MCP Ok-only invalidate:** `mcp/src/lib.rs` `tool_index_repo` ~882–897.
4. **CM Ok-only invalidate:** `codemode/src/session.rs` ~261 after successful index.
5. **Unit Ok-path pin (green):**
   ```text
   cargo test -p ast-sgrep-mcp --lib
   # cache_tests::index_repo_invalidates_searcher_after_disk_mutation ok
   # cache_tests::reindex_generation_rejects_in_flight_stale_searcher ok
   ```
6. **Unit Err-path pin:** ABSENT.
7. Full writeup: `dual-evidence-high-findings.md` §H2.

## Desired state

After any `index_repo` attempt that may have mutated disk (at minimum: after `Indexer::new` + any call into `index_all`/`reindex_all` that returns, success **or** error):

1. Searcher cache invalidated / generation advanced (MCP).
2. `path_registry` and `emitted_snippets` cleared (MCP).
3. CM searcher cache cleared on Err as well as Ok.
4. Error surface preferably distinguishes "sidecar rebuild failed after commit" vs "pre-commit failure" (observability; soft).

Preferred implementation shapes (pick one; smallest correct):

- **A (surface):** `defer`/scope-guard invalidate on MCP/CM around index body (even if core still returns mid-sidecar Err).
- **B (core):** make `index_all` not return bare Err after commit without signaling committed-dirty; or rebuild sidecars inside a documented best-effort + status flag (larger design).
- **C (hybrid):** A now; B later if semantic consistency of partial sidecar matters.

Recommend **A** for minimal blast radius under audit→fix authorization.

## Acceptance (done_when)

- [ ] On simulated/post-commit sidecar failure (or injected `rebuild_dirty_sidecars` Err), MCP generation advances and cache entry is None
- [ ] `path_registry` / `emitted_snippets` empty after that Err
- [ ] Existing Ok-path tests still pass (`index_repo_invalidates_searcher_after_disk_mutation`)
- [ ] New unit test name documents mid-sidecar / index Err invalidate (no full workspace suite required)
- [ ] CM Session invalidates on index Err (or documented intentional difference with test)
- [ ] Comment above MCP invalidate matches control flow

## Non-goals

- Redesigning sidecar atomicity with generation layout (related CL-PINNED-REINDEX is separate)
- Full multi-process watch fix (packet 03)
- Doctor/ops polish (packet 04)

## Verify

```bash
cargo test -p ast-sgrep-mcp --lib
cargo test -p ast-sgrep-mcp --lib -- index_repo  # or new Err-path test name
# Do not require full workspace cargo test
```

## Handoff

Highest-confidence **product fix** residual from campaign. Pass 12 may still seal ZERO-CHANGE on **audit books** if no implement auth; residual stays PENDING until fixed or explicitly deferred with date.

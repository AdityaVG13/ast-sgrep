# PASS11 — Concurrency / races

**Scope:** Rayon, sqlite single-Connection, cache races, TOCTOU, Mutex poison.  
**Surfaces:** core, cli, mcp, lsp.  
**Date:** 2026-08-07

## Verdict

**Mostly clean.** One integrity gap fixed (query-embed QCACHE poison). No correctness races found that can return mixed generations or corrupt the index under the current threading model.

## Fix this pass

| Issue | Severity | Action |
|-------|----------|--------|
| `QUERY_EMBED_CACHE` used `if let Ok(lock)` — after poison the mutex stayed poisoned forever and the cache was permanently skipped (diverged from sxjc matrix) | integrity | `lock_clear_on_poison` + clear map; test `query_embed_cache_poison_recovers_fail_closed` |
| `docs/panic-poison.md` missing QCACHE + LSP dirty_buffers rows | doc | Updated |

## ≥3 concurrency designs verified correct

### 1. Index: parallel prepare, serial SQLite upsert (Rayon × single Connection)

**Where:** `crates/ast-sgrep-core/src/index.rs` (`index_all`)

**Design:**
- `candidates.par_iter().map(prepare_file)` does **CPU-only** work (read, hash, tree-sitter via **per-worker `thread_local!` `ParserRegistry`** + lang `TS_PARSERS`).
- `file_hash` / status reads happen **before** the parallel section on the owning thread.
- After `collect`, a single-threaded loop runs `begin_bulk_tx` → `upsert_file` → `apply_bulk_write_result`.

**Why safe:** `rusqlite::Connection` is `!Sync`. No Connection is shared across Rayon workers. Upserts are strictly serial on one Connection.

**Evidence:** `index.rs` lines ~248–331; `prepare_file` ~1046–1050 (`thread_local! REGISTRY`).

### 2. Search: WAL read snapshot fence + generation-gated response cache

**Where:** `crates/ast-sgrep-core/src/search/mod.rs` (`fenced`, `cached`)

**Design:**
- Multi-pass search opens `BEGIN DEFERRED` so every pass observes one WAL snapshot (no mixed generations under concurrent reindex from another process).
- `BEGIN` failure fails closed (no unfenced search).
- Response cache keys include full `SearchOptions::cache_identity()`; stores only when pre/post `index_gen` (PRAGMA `data_version` + local `index_data_version`) match.
- Mutex poison on response / semantic caches: `clear_poison` + invalidate (sxjc).

**Why safe:** Cross-process writers commit new generations; readers either see a consistent snapshot or refuse to cache a crossed generation. Same-process Searcher is `!Sync` (holds Connection), so in-process concurrent methods on one Searcher cannot race.

**Evidence:** `search/mod.rs` ~179–235, ~305–352; `types.rs` `cache_identity`.

### 3. LSP: single `index_lock` + dirty re-apply + poison recovery

**Where:** `crates/ast-sgrep-lsp/src/backend.rs`

**Design:**
- All index-backed ops (`with_locked_indexer`, `with_store`, `with_locked_searcher`, background `index_all`) take one `Arc<Mutex<()>>`.
- Background index clones lock + dirty map; poison recovery marks `index_ready=false` and `clear_poison`.
- Full reindex: disk `index_all` then **snapshot dirty buffers under dirty mutex** and re-`index_content` so editor unsaved text is not clobbered.
- Lock order: `index_lock` then `dirty_buffers` (both paths).
- `dirty_buffers` poison fails closed by clearing the map (d2a1.15); unit test covers recovery.

**Why safe:** Document sync cannot interleave mid-`index_all` because it needs the same mutex. Dirty snapshot is taken after disk index while still holding `index_lock`.

**Evidence:** `backend.rs` ~89–225, dirty poison test ~490–520; LSP README mutex note.

### 4. MCP: generation take/restore Searcher cache + single-flight index

**Where:** `crates/ast-sgrep-mcp/src/lib.rs`

**Design:**
- Warm path: one cached `Searcher` taken out for a search and restored only if `generation` still matches.
- `index_repo` holds `index_lock` (single-flight), mutates disk, then **always** advances generation / clears path registry + elisions even if soft deadline trips after mutation (d2a1.13).
- stdio loop is single-threaded; mutexes still correct if a future multi-thread host appears.
- Poison recovery on all server mutexes via `lock_or_recover` + clear.

**Why safe:** In-flight searcher cannot be reinstalled after reindex (`generation` mismatch). Tests: `reindex_generation_rejects_in_flight_stale_searcher`, `index_repo_invalidates_searcher_after_disk_mutation`.

### 5. Semantic / ANN caches (keys + poison)

**Where:** `embed.rs` SemanticCache; `semantic_ann.rs` SESSION_CACHE

**Design:**
- SemanticCache keys: lang + max_id + **index_data_version** + **semantic_data_version** + backend (covers delete+re-add max_id reuse — 44a4).
- IVF fingerprint includes `index_data_version`; session map keyed by db path; poison clears entries.
- Rayon in ANN is over **owned flat vectors** / read-only slices (kmeans assign parallel, reduce serial for bit-identity); no shared Connection.

**Why safe:** Version pair prevents stale vectors after structural re-upsert; IVF disk publish is rename-based; kmeans parallelism is data-parallel without shared mutators.

## Designs reviewed and accepted (no change)

| Area | Notes |
|------|--------|
| Regex pass | Load lines on one Connection, then `thread::scope` over in-memory chunks; panic → fail-closed join |
| Active manifest / IVF publish | temp + fsync + rename; TEMP_SEQUENCE AtomicU64 |
| CLI watch | Single-threaded indexer + notify mpsc; no shared Connection races |
| Supervisor | AtomicBool signal flags; worker process isolation |

## Residual (not correctness races)

| Item | Notes |
|------|--------|
| LSP `index_ready` stored **after** releasing `index_lock` | Brief status flag lag; readiness is advisory for clients; search paths do not gate on the flag |
| Cross-process concurrent reindex | SQLite WAL + busy_timeout; fenced search or generation mismatch |
| META_CACHE file sizes | Estimate-only metric; can be stale vs disk |

## Beads

No new product bead: only QCACHE poison integrity (fixed this pass). Residual items above are documented, not filed as P0/P1 races.

## Verification

```text
cargo test -p ast-sgrep-core query_embed_cache_poison_recovers_fail_closed -- --nocapture
```
(see agent run output in session)

## Status

**PASS11 PRODUCTIVE** — one poison integrity fix; concurrency architecture verified correct for ≥3 designs.

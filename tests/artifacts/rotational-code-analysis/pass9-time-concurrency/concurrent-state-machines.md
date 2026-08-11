# Pass 9 — Concurrent state machines (warm Searcher / index_repo / watch)

| Field | Value |
|-------|-------|
| Loop | 9 / time-concurrency-perturbation (campaign) |
| Freeze | `fb932aac852f5496c0a7035cc5a0b508e05111cb` (retained; HEAD may hold books) |
| Axes | representation:**interleaving** · observer:**scheduler** · scale:**thread/task/process** · time:**race+crash+recovery** · perturbation:**cancel/reorder/partial-commit** · evidence:**source+tests** |
| Axes vs pass 8 | **≥4** (dataflow/data-owner → interleaving/scheduler/race) |
| Mode | audit (no product edits) |
| Evidence | source-level; zerostack engines missing (**B-ZS-ENGINES**) |

## Process topology (scheduler view)

| Process / task | Shared memory state | Disk state | Sync with others |
|----------------|---------------------|------------|------------------|
| MCP stdio server (single thread `run_stdio`) | `searcher_cache`, `index_lock`, `path_registry`, `emitted_snippets` | `.asgrep` / generation DBs | No cross-process lock |
| Code Mode session / NAPI `Mutex<Session>` / sticky serve | `searcher_cache` (Option, no gen), `calls` | same disk | NAPI serializes; batch may rayon |
| CLI `watch` loop | local `Indexer`, `pending`/`full` flags | same disk | No cross-process lock |
| CLI one-shot search/index | ephemeral Searcher/Indexer | same disk | WAL + busy_timeout only |
| LSP backend | `index_lock`, `index_ready`, `dirty_buffers` | same disk | **In-process** search/index share `index_lock`; bg thread same lock |
| Core `Searcher` | per-instance `response_cache`, `semantic_cache` | SQLite + sidecars | `BEGIN DEFERRED` fence + gen check |
| Process-global | `QUERY_EMBED_CACHE` (query vectors) | — | key includes model not corpus gen |

---

## SM-1 — MCP warm Searcher + generation

**States**

| State | Meaning |
|-------|---------|
| `CACHE_EMPTY` | `entry=None`, `generation=G` |
| `CACHE_WARM` | `entry=Some((key, Searcher))`, gen `G` |
| `SEARCHER_IN_FLIGHT` | Searcher taken out of cache (`searcher_for` `take()`), holder owns live Searcher tagged `G_take` |
| `INDEX_FLIGHT` | `index_lock` held; disk mutation in progress |
| `POISON_RECOVERED` | mutex was poisoned; `lock_or_recover` cleared entry / advanced gen |

**Events**

| Event | Transition | Linearization / commit point |
|-------|------------|------------------------------|
| `searcher_for` hit | `CACHE_WARM` → `SEARCHER_IN_FLIGHT` (entry taken) | Cache mutex drop after take |
| `searcher_for` miss | open Searcher → take | Open after disk read of active path |
| `restore_searcher(G_take)` | reinstall iff `cache.generation == G_take && entry.is_none` | **Linearization:** restore only if no invalidate since take |
| `invalidate` (index success / poison clear) | `generation = G+1`, `entry=None` | **Commit:** gen bump under `searcher_cache` lock |
| `index_repo` acquire | wait `index_lock` (counts toward 600s deadline) | Single-flight start (es7u) |
| `index_repo` disk Ok | invalidate + clear registries + clear snippets | **After** `index_all`/`reindex_all` Ok, **before** post-deadline ensure (d2a1.13) |
| `index_repo` disk Err | **no** invalidate / **no** registry clear | Early `?` — residual **CL-INDEX-FAIL-REGISTRIES** / mid-sidecar |
| deadline pre-start | refuse without mutate | Wait-only |
| deadline post-mutate | already invalidated; return Err | Soft deadline after durable mutation (ESC-3) |

**Evidence:** `crates/ast-sgrep-mcp/src/lib.rs` `SearcherCache` ~168–189, `lock_or_recover` ~581–594, `invalidate_searcher_cache` ~604–611, `searcher_for` ~614–641, `restore_searcher` ~644–660, `tool_index_repo` ~861–905; tests `reindex_generation_rejects_in_flight_stale_searcher`, `index_repo_invalidates_searcher_after_disk_mutation` ~1141–1199.

**Stdio note:** production `run_stdio` is **request-serial**. Generation/restore is still a real interleaving model for: unit-tested concurrent call patterns, future multi-thread hosts, and any reentrancy. Cross-process races (watch vs MCP) do not share these mutexes.

**Status vs INV-MCP-SEARCHER-INV:** success path **CONSISTENT** (tests). Failure-after-disk-mutation path **GAP** (see crash table).

---

## SM-2 — Core Indexer `index_all` / bulk / sidecars

**States**

| State | Meaning |
|-------|---------|
| `IDLE` | no bulk tx |
| `PREPARE_PAR` | rayon `prepare_file` over candidates (CPU/IO only; no SQLite writes) |
| `BULK_OPEN` | `BEGIN IMMEDIATE` + write durability pragma |
| `BULK_COMMITTED` | SQLite tables + `index_data_version` bumped |
| `BULK_ROLLED_BACK` | rollback + restore_synchronous (prefer restore err) |
| `SIDECAR_REBUILD` | tantivy / semantic IVF after commit |
| `DONE` / `ERR` | return to caller |

**Linearization points**

1. **SQLite durability of corpus:** `apply_bulk_write_result(Ok)` → `COMMIT` (`sqlite.rs` ~540–555).
2. **Sidecar visibility:** `rebuild_dirty_sidecars` completes (or fails after (1)).
3. **Caller-visible Ok:** only after (1)+(2)+hooks.

**Perturbation**

| Perturbation | Outcome | Status |
|--------------|---------|--------|
| Err in bulk body | rollback; prefer restore failure over write err | **CONSISTENT** (unit tests `apply_bulk_write_result_*`) |
| Crash mid-bulk Strict/Balanced | process-crash safe under WAL NORMAL/FULL | **CONSISTENT** (design + durability tests) |
| Crash mid-bulk FastUnsafe (`synchronous=OFF`) | possible tear / corruption on power loss | **GAP** by profile (documented unsafe) |
| Sidecar Err after COMMIT | `index_all` Err; SQLite already new gen; no MCP invalidate | **GAP** **CL-MID-SIDECAR-CACHE** |
| Concurrent second writer (other process) | `busy_timeout` 5s; may error or serialize at SQLite | **CONSISTENT** at SQLite layer; app-level single-flight only inside MCP/LSP |

**Evidence:** `index.rs` `index_all` ~231–280, `rebuild_dirty_sidecars` ~481–494; `sqlite.rs` bulk tx ~517–565; `sql.rs` busy_timeout; tests `store_pragmas`, `durability_epics`, bulk write unit tests.

---

## SM-3 — Generation reindex (`reindex_all` / jpbq)

**States**

| State | Disk meaning |
|-------|--------------|
| `ACTIVE_G` | `manifests/active.json` → generation G |
| `BUILD_CANDIDATE` | write only under `generations/G+1/` (active untouched) |
| `VERIFY_CAND` | integrity + smoke + sidecar peek |
| `ACTIVATE` | atomic rename of active pointer |
| `REOPEN` | Indexer reopens activated DB |
| `PINNED_LEGACY` | explicit `index_path` / `ASGREP_INDEX_PATH` → in-place `clear_all_data` + `index_all` |

**Happens-before**

- Readers with open connection to gen G keep G files (previous retained) — **fail-open retention**.
- New openers resolve via `try_index_db_path` → active manifest (**jpbq**).
- Activation: write temp → `fsync` → `rename` → parent `fsync` (unix) — `store/mod.rs` `write_active_manifest` ~115–171.

**Crash windows** — see `race-crash-window-table.md` (CW-GEN-*).

**Pinned path:** `clear_all_data` empties live DB before rebuild — **GAP** **CL-PINNED-REINDEX**.

---

## SM-4 — CLI `watch`

**States:** `IDLE_WAIT` → (`pending` grow | `full=true`) → debounce timeout → `UPDATE_PATHS` | `FULL_RESCAN` | `FLUSH_DEFERRED_SIDECARS`.

**Properties**

- Single-threaded event loop (`watch.rs` ~9–83); no concurrent search in same process.
- `update_paths` mutates live store file-by-file (not full bulk `index_all` path); marks sidecars dirty; deferred rebuild on idle timeout.
- **No** cross-process notification to MCP/CM warm caches.
- Multi-process schedule: watch writer ∥ MCP/CLI Searcher readers → SQLite isolation + Searcher `fenced` gen check.

**Status:** in-process serial **CONSISTENT**; multi-process cache coherence **GAP** (**GAP-WATCH-XPROC**).

---

## SM-5 — Code Mode session + batch

**Session**

| State | Notes |
|-------|-------|
| warm Option cache | **No** generation counter |
| `index_repo` Ok | `invalidate_searcher_cache` clears Option |
| poison on invalidate | **no-op** (`if let Ok`) — **CL-CM-POISON-INV** |
| poison on `searcher_for` | Err "searcher cache lock poisoned" (fail closed, no recover) |

**Batch scheduler**

| Mode | Condition | Shared Searcher? |
|------|-----------|------------------|
| serial | default / any non-`read_only` tool | one session, sequential |
| parallel | all `read_only` + mode Parallel or Auto N≥4 | **one session per call** (rayon); no shared Searcher |

**Linearization for mutator:** `choose_parallel` forces serial if any mutator — **CONSISTENT** INV-BATCH-NO-MUT-PAR (`batch.rs` ~146–159; test `batch_never_parallelizes_index_repo_with_readers`).

**Gap:** no CM unit test for invalidate after index (INV-CM-SEARCHER-INV **GAP**); no single-flight/deadline on CM `index_repo`.

---

## SM-6 — Searcher in-process fence (core)

| Mechanism | Role |
|-----------|------|
| `BEGIN DEFERRED` when autocommit | pin multi-pass read snapshot |
| `generation_before/after` | fail if gen moved under owned snapshot |
| `response_cache` gen tag | refuse cache hit across gen; re-check after compute (hdwh) |
| semantic IVF fingerprint | degrade on sidecar_generation_mismatch |
| `QUERY_EMBED_CACHE` | query vectors keyed by model — **not** corpus gen (OK) |

Module note admits hybrid **may** combine adjacent snapshots under concurrent reindex when fence cannot own snapshot (`search/mod.rs` ~62–63, `fenced` ~185–227). Nested-tx path `owns_snapshot=false` skips gen-mismatch hard fail — **GAP** residual **RW-NESTED-UNFENCED**.

---

## SM-7 — LSP (observer cross-check)

- `index_lock` wraps full index, store ops, and Searcher open+search (`with_locked_searcher`).
- Background `index_all` thread takes same lock; `index_ready` false until success.
- Poison on lock → clear poison, `index_ready=false`.

**Status:** in-process search∥index **CONSISTENT** (stronger than MCP stdio model). Still **no** multi-process coordination with CLI watch.

---

## Shared-state summary

| Object | Writers | Readers | Sync | Cross-process |
|--------|---------|---------|------|---------------|
| MCP `searcher_cache` | search take/restore, invalidate | search | Mutex + gen | No |
| MCP `index_lock` | index_repo only | — | Mutex single-flight | No |
| MCP registries | search remember, index clear | code_read | Mutex | No |
| CM `searcher_cache` | search, invalidate | search | Mutex weak | No |
| SQLite index | index/watch/LSP | Searcher | WAL + busy 5s | Yes (FS) |
| active.json | reindex_all | try_index_db_path | rename+fsync | Yes |
| Sidecars | rebuild / invalidate | search | files next to DB | Yes |
| QUERY_EMBED_CACHE | embed pass | embed pass | Mutex | N/A process-local |

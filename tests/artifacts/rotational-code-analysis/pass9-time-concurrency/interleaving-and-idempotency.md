# Pass 9 — Interleavings, idempotency, cancel / timeout, partial commit

## Critical interleavings (scheduler)

### I1 — MCP search restore vs reindex (modeled)

```
S: searcher_for          → take Searcher@G0
I: index_lock; mutate disk; invalidate → G1, entry=None
S: restore_searcher(G0)  → no-op (gen mismatch)
S: next searcher_for     → open fresh @G1
```

**Linearization:** invalidate gen bump. **Tested.** CONSISTENT.

### I2 — MCP index Err after SQLite commit (sidecar)

```
I: begin_bulk; upsert…; COMMIT          → disk gen G1
I: rebuild_dirty_sidecars → Err
I: return Err  (no invalidate)
S: uses warm Searcher@G0 / registries from G0 era
```

**Linearization:** missing at MCP. **GAP** CL-MID-SIDECAR-CACHE.

### I3 — Watch process vs MCP warm cache

```
W: update_paths(file) → COMMIT file row
M: tool_agent_search with warm Searcher opened pre-update
```

No shared invalidate. Searcher connection may observe WAL updates; response_cache gen may force recompute; IVF may degrade. **Not** the same as MCP's explicit cache drop. **GAP-WATCH-XPROC**.

### I4 — Generation activate vs open path resolution

```
R: reindex build G+1; write_active_manifest
N: new Searcher::new → try_index_db_path → G+1
O: old Searcher conn → still G file (retained)
```

**CONSISTENT** retention model; MCP must still invalidate for same-process warm path.

### I5 — Batch parallel + mutator (rejected)

```
choose_parallel: any !read_only → false
run_serial: search then index_repo (or reverse) on one session
```

**CONSISTENT** INV-BATCH-NO-MUT-PAR.

### I6 — CM poison after partial panic

```
prior: poison searcher_cache
index_repo: disk Ok; invalidate no-op
searcher_for: Err poisoned (no lock_or_recover)
```

Session bricked until process recycle. **GAP** CL-CM-POISON-INV vs MCP recover.

### I7 — LSP bg index vs search

```
bg: lock; index_all; unlock; ready=true
search: lock; Searcher::new; search; unlock
```

No overlap of Searcher with writer in-process. **CONSISTENT**.

---

## Idempotency

| Operation | Idempotent? | Notes |
|-----------|-------------|-------|
| `index_all` (no force) | content-hash skip | Re-run skips unchanged files; stats differ |
| `reindex_all` gen layout | builds G+1 each time | Not byte-identical gen id; safe activate |
| `reindex_all` pinned | clear+rebuild | **Not** crash-idempotent mid-clear |
| MCP `index_repo` | serial; second waits | Wait time counts to deadline |
| MCP search | read-only | restore may drop Searcher if gen moved |
| `code_read` via path_registry | depends on registry | Stale id after failed index (CL-INDEX-FAIL-REGISTRIES) |
| batch wave | per-call results | Partial OK siblings (FM-BATCH-PARTIAL) |

No distributed idempotency keys (single-node tools).

---

## Cancellation / timeout

| Surface | Cancel model | Ambiguous completion? |
|---------|--------------|------------------------|
| MCP `INDEX_REPO_DEADLINE` 600s | soft: pre-start refuse; post-mutate Err after invalidate | **Yes** agent-visible: Err but index applied (ESC-3) |
| MCP no cooperative cancel mid-index | kill process | bulk rollback only if still in tx; post-commit durable |
| regex pass budget | `AtomicBool` + deadline between lines | partial hits possible |
| CM `max_calls` | hard refuse next call | prior calls kept |
| watch | no cancel API; process kill | partial path updates |
| embed HTTP | reqwest; redirects(0) | timeout via client defaults (not re-audited here) |

**Lost ack:** MCP post-deadline Err is a lost-success from agent POV (mutation done, error returned).

---

## Partial commit map

| Stage | Atomic unit | Partial visible? |
|-------|-------------|------------------|
| prepare_file rayon | none (no write) | no |
| bulk upsert | one SQLite tx | no (until COMMIT) |
| post-commit sidecar | separate files | **yes** if sidecar fails |
| watch multi-path | per file | **yes** |
| gen reindex | activate rename | no (old or new pointer) |
| pinned reindex | clear then fill | **yes** empty mid |
| MCP registries | cleared only on index Ok | **yes** stale on Err |
| emitted_snippets | same | elision across failed reindex risk |

---

## Perturbation: reorder / duplicate delivery

| Perturbation | Handling |
|--------------|----------|
| Duplicate `index_repo` | single-flight serialize; second may hit deadline |
| Reordered search before index ack | serial stdio preserves order; parallel hosts need gen model (MCP has it) |
| Stale MCP session after external reindex | no auto-detect of external writers → **GAP-WATCH-XPROC** |
| Retry index after deadline Err | safe re-run; may rebuild again |
| Retry search after gen-changed Err | intended |

---

## Durability profiles (commit windows)

| Profile | Write pragma | Steady | Crash process | Power loss |
|---------|--------------|--------|---------------|------------|
| Strict | FULL | FULL | safe | intended safe |
| Balanced (default) | NORMAL | NORMAL | safe | WAL caveats |
| FastUnsafe | OFF | NORMAL restored after batch | risk if restore fails | **unsafe** |

`apply_bulk_write_result` prefers restore_synchronous failure over original write Err — no silent stuck OFF (d2a1.2 / pass9 residual closed in code).

---

## Gate statement (protocol loop 11 style)

> Every critical concurrent or retried operation has a stated linearization/commit point or is reported as ambiguous.

| Operation | Linearization / commit | Ambiguous? |
|-----------|------------------------|------------|
| MCP search restore | gen match on restore | no |
| MCP index success | disk Ok + invalidate | no |
| MCP index + soft deadline | disk Ok; Err to agent | **yes** (ESC-3) |
| MCP index + sidecar fail | SQLite commit only | **yes** (CL-MID-SIDECAR) |
| Gen reindex activate | rename active.json | no |
| Pinned reindex | clear_all commit | **yes** mid empty |
| Batch mutator | forced serial | no |
| Watch multi-file | per-file | **yes** partial set |
| Search fence owned | BEGIN..gen check | no |
| Search fence nested | none | **yes** |
| Xproc watch∥MCP cache | none | **yes** |

Gate **met** for audit: each critical op named with commit point or explicit ambiguity.

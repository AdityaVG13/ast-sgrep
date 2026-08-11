# Pass 9 — Race / crash windows (CONSISTENT | GAP | CONTRADICTION)

Freeze: `fb932aac…`. Status labels re-evaluate pass-5/7/8 residuals on **time+perturbation** axis.

## Legend

| Label | Meaning |
|-------|---------|
| **CONSISTENT** | Linearization/commit point + evidence (code and/or test) holds under named schedule |
| **GAP** | Real window or missing compensation; no dual-evidence product R-* filing this pass (audit) |
| **CONTRADICTION** | Two stated contracts conflict under an interleaving (or docs vs code) |

---

## A. In-process MCP Searcher ∥ index

| ID | Schedule | Window | Commit / linearization | Status | Evidence |
|----|----------|--------|------------------------|--------|----------|
| **RW-MCP-RESTORE** | T1 `searcher_for` take → T2 `invalidate` → T1 `restore` | Stale Searcher reinstall | restore requires matching gen | **CONSISTENT** | `restore_searcher`; test `reindex_generation_rejects_in_flight_stale_searcher` |
| **RW-MCP-INDEX-OK** | index success then search | Stale warm cache | invalidate+gen++ after disk Ok, before deadline ensure | **CONSISTENT** | `tool_index_repo`; test `index_repo_invalidates_searcher_after_disk_mutation` |
| **RW-MCP-SINGLE-FLIGHT** | two `index_repo` | double write | `index_lock` serializes | **CONSISTENT** | es7u; soft 600s deadline |
| **RW-MCP-STDIO-SERIAL** | production stdio | tool calls cannot truly interleave | single-thread line loop | **CONSISTENT** (host model) | `run_stdio` |
| **RW-MCP-DEADLINE-POST** | index Ok, wall > 600s | agent sees Err after durable mutate | invalidate already done | **CONSISTENT** (state) / **ESC-3** (agent semantics) | d2a1.13 |
| **RW-MCP-INDEX-ERR-REG** | `index_all`/`reindex` Err after any disk effect | registries + Searcher not cleared | none | **GAP** **CL-INDEX-FAIL-REGISTRIES** | early `?` before invalidate |
| **RW-MCP-MID-SIDECAR** | bulk COMMIT then sidecar Err | SQLite new; warm Searcher/registries old; IVF/tantivy stale|missing | none at MCP layer | **GAP** **CL-MID-SIDECAR-CACHE** (high) | `index_all` order; no invalidate on Err |

---

## B. Core Searcher fence ∥ concurrent writer

| ID | Schedule | Window | Commit / linearization | Status | Evidence |
|----|----------|--------|------------------------|--------|----------|
| **RW-FENCE-OWN** | search owns `BEGIN DEFERRED`; writer commits; gen changes | mixed multi-pass | hard Err "generation changed…retry" | **CONSISTENT** | `fenced` ~185–227 |
| **RW-FENCE-NESTED** | outer tx already open; `owns_snapshot=false` | multi-pass without gen hard-fail | ambiguous | **GAP** **RW-NESTED-UNFENCED** | comment ~62–63; branch skips mismatch check |
| **RW-RESP-CACHE** | reindex between cache fill and hit | wrong-gen response | gen tag + post-compute recheck | **CONSISTENT** | hdwh comments; `cached` ~334+ |
| **RW-SEM-SIDECAR** | IVF rebuild lag / mismatch | wrong ANN topology | fingerprint degrade + flat fallback | **CONSISTENT** (degraded) | `semantic_manifest` |
| **RW-QUERY-EMBED** | reindex same model | query vector reuse | key = query\|backend\|model\|dim | **CONSISTENT** | query embed independent of corpus |
| **RW-BUSY** | writer holds IMMEDIATE; reader open | SQLITE_BUSY | 5s busy_timeout | **CONSISTENT** | `sql.rs` busy_timeout; durability tests |

---

## C. Generation reindex crash / reorder

| ID | Window | On crash / kill | Reader view | Status | Evidence |
|----|--------|-----------------|-------------|--------|----------|
| **CW-GEN-BUILD** | building `generations/G+1` | orphan candidate dir | still active G | **CONSISTENT** | `reindex_into_new_generation` |
| **CW-GEN-VERIFY-FAIL** | verify fails | no pointer move | still G | **CONSISTENT** | `verify_candidate_generation` |
| **CW-GEN-ACTIVATE** | temp write / fsync / rename / dir fsync | rename atomic on same FS | old or new complete pointer | **CONSISTENT** | `write_active_manifest` |
| **CW-GEN-POST-ACTIVATE-REOPEN** | after rename, before Indexer reopen | process death | new openers see G+1; dead process local | **CONSISTENT** | activation is the public commit |
| **CW-GEN-OPEN-STALE-CONN** | open Searcher on G path; activate G+1 | — | open conn stays on G file (retained) | **CONSISTENT** (pinned open) / **GAP** if warm cache should flip without invalidate | MCP invalidate covers same-process; xproc **GAP-WATCH-XPROC** |
| **CW-PINNED-CLEAR** | `clear_all_data` then rebuild in place | empty/corrupt mid-window | empty index possible | **GAP** **CL-PINNED-REINDEX** | `reindex_all` when `index_path` set |
| **CW-FASTUNSAFE** | bulk with `synchronous=OFF` | power loss | possible DB tear | **GAP** (profile-accepted) | `Durability::FastUnsafe` |

---

## D. Watch interleaving

| ID | Schedule | Window | Status | Evidence |
|----|----------|--------|--------|----------|
| **RW-WATCH-SERIAL** | debounce → update_paths → deferred sidecar | single thread | **CONSISTENT** | `watch.rs` |
| **RW-WATCH-PARTIAL-FILE** | multi-file pending; crash mid-loop | some files new, some old | **GAP** (no tx spanning path set) | `update_paths` per-file |
| **RW-WATCH-XPROC-MCP** | watch mutates disk; MCP warm Searcher | stale Searcher until MCP reindex/invalidate | **GAP** **GAP-WATCH-XPROC** | no IPC; MCP only invalidates on its `index_repo` |
| **RW-WATCH-XPROC-SEARCH** | watch + CLI search | SQLite fence may Err or see new gen | **CONSISTENT** (core fence) / stale CLI process if long-lived Searcher without reopen | CLI typically one-shot |
| **RW-WATCH-SIDECAR-DEFER** | update marks dirty; rebuild later | search between mark and flush | degraded/mismatch paths | **CONSISTENT** (degraded) if fingerprint checks fire |

Pass-5 **GAP-WATCH-ADV** retained: watch not fully invariant-modeled as adversary schedule until this table; now named as **GAP-WATCH-XPROC** + partial-file.

---

## E. Batch / CM / poison

| ID | Schedule | Window | Status | Evidence |
|----|----------|--------|--------|----------|
| **RW-BATCH-MUT** | parallel request + `index_repo`+search | shared race | **CONSISTENT** forced serial | `choose_parallel`; test |
| **RW-BATCH-RO-PAR** | N≥4 read-only Auto | separate sessions/DB opens | **CONSISTENT** (isolated) | `run_parallel` |
| **RW-CM-INV-HAPPY** | index_repo Ok then search | clear Option | **CONSISTENT** code; **GAP** test parity | `session.rs` invalidate |
| **RW-CM-POISON** | poison then index Ok | invalidate no-op; disk new | **GAP** **CL-CM-POISON-INV** | `if let Ok(mut guard)` |
| **RW-CM-NO-GEN** | hypothetical take-across-invalidate | no gen tag | **GAP** vs MCP model | no restore_searcher |
| **RW-CM-NO-FLIGHT** | two concurrent CM sessions (two NAPIs / processes) | dual index | **GAP** (process-local only) | no cross-session lock |

---

## F. LSP

| ID | Schedule | Status | Evidence |
|----|----------|--------|----------|
| **RW-LSP-LOCK** | bg index ∥ search | **CONSISTENT** shared `index_lock` | `backend.rs` |
| **RW-LSP-READY** | search while `index_ready=false` | product policy (caller checks) | AtomicBool |
| **RW-LSP-DIRTY-REAPPLY** | full index then re-apply dirty buffers | intended | `run_full_index` |

---

## G. Invariant re-link (time axis)

| INV | Pass 5 | Pass 9 time re-check |
|-----|--------|----------------------|
| INV-MCP-SEARCHER-INV | CONSISTENT | success **CONSISTENT**; mid-sidecar/err **GAP** (does not flip ledger aggregate without dual-evidence loop) |
| INV-CM-SEARCHER-INV | GAP | still **GAP** (+ poison) |
| INV-BATCH-NO-MUT-PAR | CONSISTENT | **CONSISTENT** |
| INV-DURABILITY-FC | CONSISTENT | **CONSISTENT**; FastUnsafe crash **GAP** by design |
| INV-INDEX-PATH-PREC | CONSISTENT | gen pointer **CONSISTENT**; pinned clear window **GAP** |

No new **CONTRADICTION** between two product code paths on this axis (C1/C2 remain from prior docs/root axes).

---

## Candidate tests (not implemented this pass — audit only)

1. MCP: force `rebuild_dirty_sidecars` Err after bulk commit → assert invalidate **or** document intentional stale (**CL-MID-SIDECAR-CACHE** harness).
2. MCP: `index_all` Err path clears `path_registry` / `emitted_snippets` (or assert intentional).
3. CM: mirror `index_repo_invalidates_searcher_*` + poison recover parity with MCP.
4. Multi-process: watch update then MCP search without `index_repo` → expect stale or define reopen policy.
5. `fenced` nested-tx: gen change under `owns_snapshot=false` behavior lock.
6. Pinned `index_path` reindex crash between clear and rebuild (recovery story).

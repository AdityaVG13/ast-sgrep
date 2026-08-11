# R-XPROC-MULTIWRITER

| Field | Value |
|-------|-------|
| Residual ID | **R-XPROC-MULTIWRITER** |
| Aggregates | GAP-WATCH-XPROC, GAP-XOR-RUNTIME, GAP-RO-HOST (co-location), multi-writer ops false-negative |
| Severity | **high** (ops / silent stale answers) |
| Status | **DESIGN ASK** — dual-evidence CONFIRMED gap; fix is architectural |
| Pass | 11 independent verification |
| Tracker | markdown only |

## Problem

Production deploy shapes commonly co-locate:

1. `asgrep watch` (CLI) for live index updates, and
2. Long-running MCP (or Code Mode host) with a **warm Searcher** cache, and/or
3. CI/`asgrep index` against the same `ASGREP_INDEX_PATH`.

Watch mutates via `Indexer::index_all` / `update_paths` / `flush_deferred_rebuilds` and reports only on stderr. MCP `index_lock` is a process-local `Mutex<()>`. Searcher generation is process-local. There is **no** cross-process lease, flock, generation file notify, or IPC invalidate channel.

SQLite WAL permits concurrent readers, so the DB is not necessarily corrupt — the **application cache** is stale. Agent answers from pre-watch snapshot until an in-process `index_repo` succeeds and invalidates.

Code Mode XOR MCP is policy/docs only (GAP-XOR-RUNTIME); host can run both against one index.

## Evidence (pass 11)

1. **Watch writer:** `crates/ast-sgrep-cli/src/watch.rs` `run_watch` ~L9–80 — mutates index; stderr only; no peer notify.
2. **MCP in-process only:** `index_lock: Mutex<()>` ~L182; `tool_index_repo` flight ~L861–866; `invalidate_searcher_cache` generation local ~L604–610.
3. **Tests:** Ok-path in-process invalidate **PASS**; no two-process harness.
4. Writeup: `dual-evidence-high-findings.md` §H3; pass 10 `ops-failure-signals-map.md` multi-writer false-negative.

## Product decision options (ASK)

| Option | Notes |
|--------|-------|
| A. Document single-writer contract | Ops: never watch + MCP on same DB; doctor/status warn if lock file present |
| B. Cross-process exclusive lease (flock) on index | Second writer fails closed or waits |
| C. Generation/mtime epoch file peers poll | MCP reopens Searcher when epoch changes |
| D. Drop long-lived Searcher cache in multi-writer deploys | Simpler correctness, more open cost |

## Acceptance (when product chooses non-A)

- [ ] Design note with chosen option and failure modes
- [ ] Implementation matches design
- [ ] At least one automated test or scripted dual-process smoke under `tests/` or artifacts
- [ ] Ops docs updated (`docs/index-consistency.md` or env-trust)

## Acceptance (if A only)

- [ ] Explicit single-writer docs + deploy checklist
- [ ] Optional doctor issue when `ASGREP_WATCH` / lock file heuristics fire (if cheap)
- [ ] Residual closed as **BY-DESIGN host duty** with link

## Non-goals

- Fixing mid-sidecar invalidate (packet 02) as substitute for xproc
- Inventing remote multi-tenant isolation

## Verify (sketch)

```bash
# Design A: docs exist
rg -n "single-writer|watch.*MCP|ASGREP_INDEX_PATH" docs/
# Design C/B: after impl
# script: process1 watch / process2 mcp search generation observation
```

## Handoff

Pass 12: not auto-fixed. Seal of audit campaign allowed with residual **PENDING design**. Do not claim multi-writer CONSISTENT.

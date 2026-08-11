# Pass 9 RESULT — Time / concurrency / perturbation

| Field | Value |
|-------|-------|
| Loop | 9 / time-concurrency-perturbation (campaign pass 9; protocol axes from loops 9+11) |
| Status | **COMPLETE** |
| Mode | audit (no product edits under crates/ or packages/) |
| Freeze | `fb932aac852f5496c0a7035cc5a0b508e05111cb` (retained; HEAD may hold books) |
| Axes | representation:**interleaving** · observer:**scheduler** · scale:**thread/task/process** · time:**race+crash+recovery** · perturbation:**cancel/reorder/partial-commit** · evidence:**source+tests** |
| Axes vs pass 8 | **≥4** |
| Braid | **Continue** → pass 10 boundary/adversary + ops |
| Prior state leveraged | true (pass 8 residuals + pass 7 CL-* + pass 5 INV) |

## Deliverables

| Artifact | Path |
|----------|------|
| Concurrent state machines | `iterations/09-time-concurrency/concurrent-state-machines.md` |
| Race/crash window table | `iterations/09-time-concurrency/race-crash-window-table.md` |
| Interleaving / idempotency | `iterations/09-time-concurrency/interleaving-and-idempotency.md` |
| Machine result | `iterations/09-time-concurrency/loop-09-result.json` |
| Slim mirror | `tests/artifacts/rotational-code-analysis/pass9-time-concurrency/` |

## 1. State machines mapped

| SM | Surface | Headline |
|----|---------|----------|
| SM-1 | MCP Searcher + gen + index_lock | take/restore + gen linearization; success path tested |
| SM-2 | Indexer bulk + sidecars | SQLite commit then sidecar; mid-sidecar gap |
| SM-3 | Generation reindex (jpbq) | build→verify→atomic activate; pinned weaker |
| SM-4 | CLI watch | serial debounce; xproc cache gap |
| SM-5 | CM session + batch | mutator serial CONSISTENT; no gen; poison GAP |
| SM-6 | Searcher fence | owned snapshot CONSISTENT; nested GAP |
| SM-7 | LSP index_lock | in-process search∥index CONSISTENT |

## 2. Top concurrency findings (audit observations)

| Rank | ID | Summary | Status | Severity |
|------|-----|---------|--------|----------|
| 1 | **CL-MID-SIDECAR-CACHE** / **RW-MCP-MID-SIDECAR** | After bulk COMMIT, sidecar rebuild Err → MCP/CM skip Searcher+registry invalidate; disk gen advanced, warm state pre-mutation | **GAP** | high |
| 2 | **GAP-WATCH-XPROC** | CLI `watch` (or any external indexer) mutates DB without MCP/CM invalidate; multi-process no shared flight | **GAP** | high (ops) |
| 3 | **CL-PINNED-REINDEX** / **CW-PINNED-CLEAR** | Explicit `index_path` reindex clears live DB in place — crash empty window vs gen layout | **GAP** | medium |
| 4 | **CL-CM-POISON-INV** + **RW-CM-NO-GEN** | CM invalidate ignores poison; no generation restore model vs MCP | **GAP** | medium |
| 5 | **CL-INDEX-FAIL-REGISTRIES** | path_registry / emitted_snippets uncleared on any index Err (incl. mid-sidecar) | **GAP** | low–med |
| 6 | **RW-NESTED-UNFENCED** | Search `fenced` when not owning snapshot skips gen hard-fail | **GAP** | low–med |
| 7 | **ESC-3** / deadline post-mutate | index durable + Err to agent (lost success ack) | known semantic | low–med |
| — | **RW-MCP-RESTORE**, **INV-BATCH-NO-MUT-PAR**, **CW-GEN-***, **RW-LSP-LOCK**, bulk rollback | Positive controls | **CONSISTENT** | — |

No new product **R-*** filings (audit books only). No invented benchmarks.

## 3. Pass 8 residual disposition

| Residual | Disposition |
|----------|-------------|
| Searcher generation races | **CONSISTENT** for modeled in-process take/restore (tests); residual only on Err/mid-sidecar |
| path_registry lifetime | **GAP** on index Err (unchanged CL-INDEX-FAIL-REGISTRIES) |
| SQLite durability + sidecar | bulk rollback **CONSISTENT**; mid-sidecar **GAP**; FastUnsafe power **GAP** by design |
| Watch interleaving | named **GAP-WATCH-XPROC** + partial multi-file |
| Batch mutator serial | **CONSISTENT** reaffirmed |
| CM poison | **GAP** reaffirmed |
| Index single-flight | MCP **CONSISTENT**; CM/xproc **GAP** |

## 4. Residual for pass 10 (boundary / adversary + ops)

Pass 10 card: privilege, SSRF/path, config, deploy/rollback signals (boundary+ops).

1. **BY-CM-ROOT / C2** under adversarial `root` + concurrent index of attacker-chosen tree (boundary × time).
2. **path_registry stale ids** as capability after failed index (boundary: `code_read` resolution).
3. **GAP-WATCH-XPROC** ops story: multi-writer deploy (watch + MCP + CI index).
4. **Pinned index_path** privilege + crash recovery (INV-INDEX-PATH-PRIV).
5. **Embed allowlist / no-redirect** under retry/timeout (boundary residual GAP-EMBED-REDIR-IT).
6. **FastUnsafe** ops footgun (config → durability).
7. Dual-evidence promotion of high GAPs only in pass 11 style — do not file beads until then unless asked.
8. Retain: C1, GAP-CM-ROOT, GAP-XOR-RUNTIME, GAP-RO-HOST, B-ZS-ENGINES, B-DIRTY-FREEZE, B-SECURITY-NAPI-DOC.

## Gate check

> Every critical concurrent or retried operation has a stated linearization/commit point or is reported as ambiguous.

**Met** — see interleaving doc gate table; ambiguities named (mid-sidecar, nested fence, xproc watch, pinned clear, deadline ack).

## Evidence commands

```
# zerostack engines unavailable (B-ZS-ENGINES)
# ZS_FSZERO_BIN / tokenzero path missing under harness HOME

rg -n 'fn invalidate_searcher_cache|fn restore_searcher|fn searcher_for|fn tool_index_repo|INDEX_REPO_DEADLINE' \
  crates/ast-sgrep-mcp/src/lib.rs
rg -n 'fn choose_parallel|fn run_parallel|fn index_repo|invalidate_searcher_cache' \
  crates/ast-sgrep-codemode/src/batch.rs crates/ast-sgrep-codemode/src/session.rs
rg -n 'fn index_all|fn reindex_into_new_generation|fn update_paths|fn rebuild_dirty' \
  crates/ast-sgrep-core/src/index.rs
rg -n 'fn fenced|generation_before|write_active_manifest|apply_bulk_write_result|begin_bulk_tx' \
  crates/ast-sgrep-core/src/search/mod.rs crates/ast-sgrep-core/src/store/mod.rs \
  crates/ast-sgrep-core/src/store/sqlite.rs
rg -n 'fn run_watch|index_lock|start_background_index' \
  crates/ast-sgrep-cli/src/watch.rs crates/ast-sgrep-lsp/src/backend.rs
# tests (not re-executed this pass; anchors):
#   mcp cache_tests::reindex_generation_rejects_in_flight_stale_searcher
#   mcp cache_tests::index_repo_invalidates_searcher_after_disk_mutation
#   codemode tests/batch.rs::batch_never_parallelizes_index_repo_with_readers
#   core apply_bulk_write_result_* ; store_pragmas ; durability_epics
```

## Counts

- State machines: **7**
- Race/crash window rows: **30+**
- CONSISTENT headline controls: **8+**
- GAP residuals (time axis): **7** named
- CONTRADICTION (new on this axis): **0**
- New product R-*: **0**

## Braid residue

```
SPIN_THE_BLOCK_RESULT:
status: in_progress
mode: audit
target_root: /Users/aditya/Developer/ast-sgrep
prior_state_leveraged: true
iteration: 9
coverage_pending: foundation loops 10+
high_critical_without_loop27: n/a (audit observations; no new R-* product findings)
braid_resolve: Continue
axes_changed: 4+
void_fixture_outcome: n/a mid-wave
north_star_probe_outcome: n/a mid-wave
independent_loop27: pending
queue_action: none
books: .rotational-code-analysis/
```

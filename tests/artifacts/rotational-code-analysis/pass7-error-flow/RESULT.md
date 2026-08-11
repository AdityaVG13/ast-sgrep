# Pass 7 RESULT — Error / exception / cleanup flow

| Field | Value |
|-------|-------|
| Loop | 7 / error-exception-cleanup-flow |
| Status | **COMPLETE** |
| Mode | audit (no product edits under crates/ or packages/) |
| Freeze | `fb932aac852f5496c0a7035cc5a0b508e05111cb` (retained; HEAD may hold books) |
| Axes | representation:**exception-graph** · observer:**failure-handler** · scale:**entrypoint→cleanup** · time:**degradation** |
| Axes vs pass 6 | **4** (all changed) |
| Braid | **Continue** → pass 8 data provenance & sinks |
| Prior state leveraged | true (pass 6 residuals + pass 5 INV + pass 4 EPs) |

## Deliverables

| Artifact | Path |
|----------|------|
| Failure-path traces | `iterations/07-error-flow/failure-path-traces.md` |
| Cleanup/compensation matrix | `iterations/07-error-flow/cleanup-compensation-matrix.md` |
| Error-semantic contradictions | `iterations/07-error-flow/error-semantic-contradictions.md` |
| Machine result | `iterations/07-error-flow/loop-07-result.json` |
| Slim mirror | `tests/artifacts/rotational-code-analysis/pass7-error-flow/` |

## 1. Failure modes found

Catalog of **24** FM-* modes covering pass-6 residual classes:

| Residual (pass 6) | FM-* coverage |
|-------------------|---------------|
| MCP jail vs CM free root | FM-JAIL, FM-CM-ROOT-OK, ESC-2 |
| Empty index CLI vs MCP | FM-EMPTY-CLI, FM-EMPTY-MCP, ESC-1 |
| MCP deadline / single-flight | FM-DEADLINE-WAIT, FM-DEADLINE-POST, ESC-3 |
| CM max_calls / lock poison | FM-MAX-CALLS, FM-POISON-MCP, FM-POISON-CM, ESC-7 |
| Cascade empty lexical vs structural | FM-CASCADE-LEX-EMPTY, FM-CASCADE-STRUCT-EMPTY, ESC-4 |
| Embed allowlist / fail-closed mid-path | FM-EMBED-DENY, FM-EMBED-HYBRID-ABORT, FM-EMBED-CHAIN-FALLBACK, ESC-5/6 |
| Pi sticky miss + fallback | FM-PI-DEGRADE |
| Batch partial when mutator serial | FM-BATCH-PARTIAL |
| Parse: unknown tool, oversize, bad node | FM-PARSE-TOOL/QUERY/NODE |
| Mid-index cleanup | FM-MID-INDEX-BULK/SIDECAR/REINDEX/PER-FILE-SOFT |

**Headline observations (audit, not new product R-* filings):**

1. **MCP search restore is exception-safe** (`restore_searcher` before `?`); deadline path invalidates before soft Err (d2a1.13).
2. **Empty-index contracts diverge** hard (CLI Err) vs soft (MCP Ok+why) -- agent-visible ESC-1.
3. **Post-commit sidecar failure skips cache invalidate** on MCP/CM -- primary cleanup gap (CL-MID-SIDECAR-CACHE).
4. **CM poison invalidate is a no-op**; MCP recovers -- INV-CM-SEARCHER-INV GAP on failure axis.
5. **Embed URL allowlist + redirects(0)** remain fail-closed; hybrid stage D `?` drops prior hits; chain fallback message vs Semantic return is honesty tension.
6. **Bulk write rollback** prefers restore error (no swallow); generation reindex fails closed with previous retained; pinned-path reindex weaker.
7. **Degraded successes:** rerank skip, ledger skip, lexicon skip, per-file files_failed with Ok stats.

## 2. Cleanup gaps

| ID | Summary | Severity |
|----|---------|----------|
| CL-MID-SIDECAR-CACHE | Sidecar Err after SQLite commit → no searcher/registry invalidate | high |
| CL-CM-POISON-INV | invalidate ignores poison | medium |
| CL-INDEX-FAIL-REGISTRIES | path_registry / emitted_snippets uncleared on index Err | low–med |
| CL-PINNED-REINDEX | in-place clear window for explicit index_path | medium |
| CL-EMBED-MSG | refuse-fallback log then Semantic | low |
| CL-HYBRID-EMBED-DROP | embed Err discards lexical/structural hits | medium (UX) |

Positive controls listed in cleanup matrix (restore, bulk rollback, gen verify, batch mutator serial, jail).

## 3. Residual for pass 8 (data sinks)

1. **Untrusted sources → sinks:** query text, `root`/`path` args, node ids, compact path_registry keys, embed URL/env, `ASGREP_INDEX_PATH`, plan `$ref` paths.
2. **Validation order:** wire deny_unknown_fields → sandbox_root → parse_node_id relative components → open/read; check whether later transforms reintroduce escapes.
3. **Capability residue of path_registry** after failed index (CL-INDEX-FAIL-REGISTRIES) -- stale id→path map as soft capability.
4. **Embed request body / API key** and no-redirect final-hop (INV-EMBED-ALLOW evidence sink).
5. **Miss envelope / compact JSON** as agent-control surface (why/next fields).
6. **Index DB write sinks** under durability profiles (from_env fail-closed unknown).
7. Do **not** invent benchmark numbers; provenance rules unchanged.

Prior open residuals retained: GAP-CM-ROOT, GAP-CM-INV-TEST, GAP-RO-HOST, GAP-XOR-RUNTIME, GAP-INDEX-PATH-DOC, B-ZS-ENGINES, B-DIRTY-FREEZE, B-SECURITY-NAPI-DOC, C1/C2.

## Gate check

> Each critical side effect has a documented failure and cleanup outcome; unhandled exits become findings or UNKNOWNs.

**Met** -- side-effect matrix with Y/N/P/I outcomes; gaps named CL-*; no silent UNKNOWN for pass-6 residual classes (all mapped to FM-*).

## Evidence commands

```
# zerostack unavailable (B-ZS-ENGINES)
zs --json -C /Users/aditya/Developer/ast-sgrep fs '…'  # fail: fszero-codemode missing

rg -n 'fn sandbox_root|tool_index_repo|INDEX_REPO_DEADLINE|lock_or_recover|invalidate_searcher' \
  crates/ast-sgrep-mcp/src/lib.rs
rg -n 'bump_call|max_calls|invalidate_searcher_cache|searcher cache lock poisoned' \
  crates/ast-sgrep-codemode/src/session.rs
rg -n 'ensure_nonempty_index|apply_bulk_write_result|rebuild_dirty_sidecars|search_hybrid' \
  crates/ast-sgrep-cli/src/index_cmd.rs crates/ast-sgrep-core/src/index.rs \
  crates/ast-sgrep-core/src/store/sqlite.rs crates/ast-sgrep-core/src/search/mod.rs
rg -n 'embed_url_is_allowed|embed_with_chain|ASGREP_EMBED_FALLBACK' crates/ast-sgrep-embed/src/embedder.rs
rg -n 'to_compact_miss|EmptyIndex|choose_parallel|is_read_only' \
  crates/ast-sgrep-plugins/src/lib.rs crates/ast-sgrep-codemode/src/batch.rs
# prior: pass6 traces + pass5 invariant-ledger
```

No product binaries re-run for timings (source-level only; no invented numbers).

## Counts

- Failure modes catalogued: **24**
- Cleanup matrix rows: **30+**
- Named cleanup gaps: **6**
- Error-semantic contradictions: **9** ESC-*
- Invariants re-linked: **18** (error-axis notes)
- New product R-* findings filed: **0** (audit books; gaps reinforce existing INV/GAP/C*)

## Braid residue

```
SPIN_THE_BLOCK_RESULT:
status: in_progress
mode: audit
target_root: /Users/aditya/Developer/ast-sgrep
prior_state_leveraged: true
iteration: 7
coverage_pending: foundation loops 8+
high_critical_without_loop27: n/a (audit observations; no new R-* product findings)
braid_resolve: Continue
axes_changed: 4
void_fixture_outcome: n/a mid-wave
north_star_probe_outcome: n/a mid-wave
independent_loop27: pending
queue_action: none
books: .rotational-code-analysis/
```

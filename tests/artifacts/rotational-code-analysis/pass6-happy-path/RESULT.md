# Pass 6 RESULT — Happy-path control flow

| Field | Value |
|-------|-------|
| Loop | 6 / happy-path-control-flow |
| Status | **COMPLETE** |
| Mode | audit (no product edits under crates/ or packages/) |
| Freeze | `fb932aac852f5496c0a7035cc5a0b508e05111cb` (retained; HEAD may hold books) |
| Axes | representation:**call-trace** · observer:**runtime** · scale:**entrypoint→sink** · time:**normal** |
| Axes vs pass 5 | **4** (all changed) |
| Braid | **Continue** → pass 7 error/cleanup flow |
| Prior state leveraged | true (pass 4 EPs + pass 5 INV ledger + C1/C2) |

## Deliverables

| Artifact | Path |
|----------|------|
| Success traces | `iterations/06-happy-path/traces.md` |
| Invariant enforcement map | `iterations/06-happy-path/invariant-enforcement-map.md` |
| Machine result | `iterations/06-happy-path/loop-06-result.json` |
| Slim mirror | `tests/artifacts/rotational-code-analysis/pass6-happy-path/` |

## Path list with anchors

| ID | Entry → sink | Key anchors |
|----|--------------|-------------|
| **HP-CLI-SEARCH** | `asgrep` bare/search → `Searcher::search` | `cli/lib.rs` `run_default_search`; `index_cmd.rs` `open_searcher`; `search/mod.rs` `search` |
| **HP-MCP-SEARCH** | `tools/call` → channel `Searcher::*` | `mcp/main.rs`; `dispatch_tool` / `sandbox_root` / `tool_agent_search` |
| **HP-CM-CALL** | `Session.call` → catalog → hybrid | `codemode/session.rs` `call`/`search`/`root_arg`; `tools.rs` `call_tool`; napi `Session::call` |
| **HP-PI-ASGREP** | Pi `asgrep` → sticky NAPI → CM | `pi/.../index.ts`; `session-pool.ts`; `connector.ts` → HP-CM-CALL |
| **HP-INDEX** | index / `index_repo` → `Indexer::index_all` | CLI `with_index`; MCP `tool_index_repo`; CM `index_repo`; `index.rs` `index_all` |
| **HP-CASCADE** | Hybrid arm → RRF → finish | `search_hybrid` ~480; `apply_weighted_rrf`; `finish_response_checked` |

## Headline findings (audit observations)

1. **Six end-to-end success paths** traced; all search paths share `ast-sgrep-core` open+search except MCP skips hybrid fusion.
2. **C1 on live happy shape:** empty structural → lexical working set + optional embed (`search_hybrid` ht1h.3); docs stop claim remains CONTRADICTION.
3. **C2 on live happy shape:** MCP `sandbox_root` fail-closed vs CM `root_arg` free -- same Indexer/Searcher, different boundary.
4. **Divergent control planes:** MCP gen-aware Searcher + deadline index vs CM Option cache + no deadline vs CLI cold process.
5. **INV-RO-CATALOG GAP confirmed on Pi/CM success:** `index_repo` reachable without host approval gate.
6. **Who runs cascade:** CLI default, CM `search`, Pi natural search -- **not** MCP tools.

## Invariant enforcement (happy-path summary)

| Class | IDs | Happy-path note |
|-------|-----|-----------------|
| Enforced on relevant paths | INV-MCP-SANDBOX, INV-INDEX-PATH-PREC, INV-MCP-SEARCHER-INV, INV-BATCH-NO-MUT-PAR, INV-EMBED-ALLOW, INV-DURABILITY-FC, INV-CASCADE-NO-WIDEN, INV-LIMITS, INV-RANK-FUSION | gates/clamps/fusion present on success |
| Contradiction exercised by success | INV-CASCADE-STRUCT-EMPTY (C1), INV-SURFACE-ROOT-PARITY (C2) | code succeeds under one contract of each pair |
| Gap visible on success | INV-CM-ROOT-FREE, INV-INDEX-PATH-PRIV, INV-CM-SEARCHER-INV, INV-RO-CATALOG, INV-XOR-CM-MCP | free root / privilege / advisory flags |
| N/A on pure search | INV-AST-GREP, INV-EDIT-ROOT | other paths |

Full matrix: [`invariant-enforcement-map.md`](./invariant-enforcement-map.md).

## Gate check

> Trace 4–6 critical success paths end-to-end from source; link invariants; note divergences; mirror artifacts.

**Met** -- 6 paths, INV map, divergences table, slim mirror under `tests/artifacts/.../pass6-happy-path/`.

## Evidence commands

```
# zerostack unavailable
zs --json -C /Users/aditya/Developer/ast-sgrep fs '…'  # fail: fszero-codemode missing

rg -n 'fn search_hybrid|working_files = if structural|fn sandbox_root|fn root_arg|fn tool_index_repo|pub fn call\(&mut self' \
  crates/ast-sgrep-core/src/search/mod.rs \
  crates/ast-sgrep-mcp/src/lib.rs \
  crates/ast-sgrep-codemode/src/session.rs

rg -n 'run_default_search|open_searcher|dispatch_tool|createAsgrepConnector|NativeSessionPool|index_all' \
  crates/ast-sgrep-cli packages/pi/extension/src crates/ast-sgrep-core/src/index.rs

# prior: pass4 entry-point-catalog + pass5 invariant-ledger
```

No product binaries re-run for timings (source-level only; no invented numbers).

## Counts

- Success paths traced: **6**
- Surfaces covered: CLI, MCP, Code Mode/NAPI, Pi, Index multi-entry, core cascade
- Invariants linked: **18** (from pass 5)
- New product R-* findings filed: **0** (audit books only; contradictions already in pass 5)
- New residual classes for pass 7: **10** named error branches (see traces residual section)

## Residuals → pass 7 (error / cleanup flow)

1. MCP jail escape errors vs CM foreign-root success (C2 fail vs success asymmetry)
2. Empty-index CLI bail vs MCP miss envelope
3. MCP `index_repo` deadline / single-flight wait timeout
4. Codemode call budget / lock poison
5. Cascade empty-lexical stop vs empty-structural continue (C1 error-shape clarity)
6. Embed allowlist / feature-flag fail-closed mid-path
7. Pi sticky miss + CLI fallback + freshness rebuild failures
8. Batch partial failure envelopes when mutator serial
9. Boundary parse: unknown tool, oversize query, bad node ids (`code_read`)
10. Cleanup: path_registry / emitted_snippets / Searcher gen after failed index midway

## Braid residue

```
SPIN_THE_BLOCK_RESULT:
status: in_progress
mode: audit
target_root: /Users/aditya/Developer/ast-sgrep
prior_state_leveraged: true
iteration: 6
coverage_pending: foundation loops 7+
high_critical_without_loop27: n/a (audit observations; no new R-* product findings)
braid_resolve: Continue
axes_changed: 4
void_fixture_outcome: n/a mid-wave
north_star_probe_outcome: n/a mid-wave
independent_loop27: pending
queue_action: none
books: .rotational-code-analysis/
```

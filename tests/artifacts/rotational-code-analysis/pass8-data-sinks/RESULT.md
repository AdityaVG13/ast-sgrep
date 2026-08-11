# Pass 8 RESULT — Data provenance validation & sinks

| Field | Value |
|-------|-------|
| Loop | 8 / data-provenance-validation-and-sinks |
| Status | **COMPLETE** |
| Mode | audit (no product edits under crates/ or packages/) |
| Freeze | `fb932aac852f5496c0a7035cc5a0b508e05111cb` (retained; HEAD may hold books) |
| Axes | representation:**dataflow** · observer:**data-owner+adversary** · scale:**source→sink** · evidence:**source+schema** |
| Axes vs pass 7 | **4** (all changed) |
| Braid | **Continue** → pass 9 persistence / transactions / cache consistency (time+concurrency residue) |
| Prior state leveraged | true (pass 7 residuals + pass 5 INV + pass 4/6 sinks) |

## Deliverables

| Artifact | Path |
|----------|------|
| Source-to-sink traces | `iterations/08-data-sinks/source-to-sink-traces.md` |
| Validation/normalization map | `iterations/08-data-sinks/validation-normalization-map.md` |
| Sensitive-data ownership ledger | `iterations/08-data-sinks/sensitive-data-ownership-ledger.md` |
| Classification vs invariants | `iterations/08-data-sinks/classification-vs-invariants.md` |
| Machine result | `iterations/08-data-sinks/loop-08-result.json` |
| Slim mirror | `tests/artifacts/rotational-code-analysis/pass8-data-sinks/` |

## 1. Critical sinks (traced)

| Sink ID | Data in | Enforcement | Status |
|---------|---------|-------------|--------|
| S-SEARCH-ENGINE | query | INV-LIMITS, parse modes | CONSISTENT |
| S-EMBED-HTTP + S-AUTH-HEADER | query/chunks + API key | INV-EMBED-ALLOW, Debug redact | CONSISTENT |
| S-SQLITE-OPEN/WRITE | index path + file corpus | INV-INDEX-PATH-PREC; PRIV unlabeled | GAP on privilege |
| S-INDEX-WALK | root | MCP sandbox / CM free | C2 |
| S-FILE-READ (code_read) | node id | parse_node_id + TOCTOU + sandbox | CONSISTENT |
| S-DISK-WRITE (Pi edit) | path + contents | INV-EDIT-ROOT | CONSISTENT |
| S-CMD-ASTGREP | pattern + root | INV-AST-GREP dual opt-in | CONSISTENT |
| S-RESPONSE / miss | query, why/next | product-owned strings | soft agent-control |

## 2. Bypasses / non-enforcements

| ID | Summary | Severity |
|----|---------|----------|
| BY-CM-ROOT | Code Mode `root_arg` no jail → index/search/walk | high (host-dependent) |
| BY-INDEX-ABS | Absolute index path outside project root | medium (privileged env) |
| BY-REGISTRY-STALE | path_registry not cleared on index Err | low–medium |
| DF-PLAN-ROOT | `$ref` can feed CM free root | medium (amplifies C2) |
| BY-QUERY-REGEX-CPU | regex length only | low residual |
| BY-QUERY-CONTENT | no DLP on embed payload | by design |

## 3. Invariant classification (18)

Unchanged aggregate: **11 CONSISTENT · 2 CONTRADICTION · 5 GAP**. Dataflow axis **reinforces** C1/C2 and index/embed/read chains; no status flips.

## 4. Residual for pass 9 (time / concurrency / persistence)

Pass 9 card: persistence, transactions, cache consistency (state-store; data-integrity; operation→stores; commit+recovery).

1. **Searcher cache + generation** under concurrent MCP search vs `index_repo` (d2a1.13 restore races) — durability of in-memory Searcher vs on-disk generation.
2. **path_registry / emitted_snippets** lifetime across failed index and multi-flight (BY-REGISTRY-STALE + CL-INDEX-FAIL-REGISTRIES).
3. **SQLite durability profiles** write windows (Strict/Balanced/FastUnsafe) and bulk rollback vs sidecar commit (CL-MID-SIDECAR-CACHE).
4. **Index generation pointer** activation vs pinned `index_path` clear window (CL-PINNED-REINDEX).
5. **Query/response embed caches** poison recovery and stale vectors after reindex.
6. **Batch serial mutator** vs parallel readers (INV-BATCH-NO-MUT-PAR) under load — consistency of shared Searcher.
7. **CM searcher_cache Mutex** poison path (CL-CM-POISON-INV) as concurrent failure mode.
8. Do **not** invent benchmark numbers; provenance rules unchanged.

Prior open residuals retained: C1/C2, GAP-CM-ROOT, GAP-CM-INV-TEST, GAP-RO-HOST, GAP-XOR-RUNTIME, GAP-INDEX-PATH-DOC, GAP-EMBED-REDIR-IT, B-ZS-ENGINES, B-DIRTY-FREEZE, B-SECURITY-NAPI-DOC, pass-7 CL-*.

## Gate check

> Every critical sink has a traced source and enforcement path; pattern-only suspicions are not findings.

**Met** — eight DF-* data classes with source→validation→sink; BY-* named with evidence anchors; pattern-only (e.g. generic “injection”) not filed without path.

## Evidence commands

```
# zerostack unavailable (B-ZS-ENGINES)
zs --json -C /Users/aditya/Developer/ast-sgrep fs '…'  # fail: fszero-codemode missing

rg -n 'fn sandbox_root|fn parse_node_id|fn tool_code_read|fn read_node' crates/ast-sgrep-mcp/src/lib.rs
rg -n 'fn root_arg|validate_query_len' crates/ast-sgrep-codemode/src/session.rs
rg -n 'fn try_index_db_path|Durability::from_env' crates/ast-sgrep-core/src/store/mod.rs
rg -n 'embed_url_is_allowed|redirects\(0\)|api_key' crates/ast-sgrep-embed/src/embedder.rs
rg -n 'export function planEdit|containedInRoot|assertSafeEditTarget' packages/pi/extension/src/edit.ts
rg -n 'fn resolve_ref|run_plan' crates/ast-sgrep-codemode/src/plan.rs
rg -n 'validate_query_len|MAX_QUERY_CHARS|MAX_REGEX' crates/ast-sgrep-core/src/limits.rs crates/ast-sgrep-core/src/search/passes/regex.rs
# tests: crates/ast-sgrep-mcp/tests/protocol.rs tool_roots_* code_read_* compact_*
# prior: pass5 invariant-ledger, pass6 traces, pass7 failure-path-traces
```

No product binaries re-run for timings (source-level only; no invented metrics).

## Counts

- Dataflow traces (DF-*): **8**
- Critical sinks named: **8+**
- Named bypasses: **6**
- Invariants re-linked: **18**
- New product R-* findings: **0** (audit books)

## Braid residue

```
SPIN_THE_BLOCK_RESULT:
status: in_progress
mode: audit
target_root: /Users/aditya/Developer/ast-sgrep
prior_state_leveraged: true
iteration: 8
coverage_pending: foundation loops 9+
high_critical_without_loop27: n/a (audit observations; no new R-* product findings)
braid_resolve: Continue
axes_changed: 4
void_fixture_outcome: n/a mid-wave
north_star_probe_outcome: n/a mid-wave
independent_loop27: pending
queue_action: none
books: .rotational-code-analysis/
```

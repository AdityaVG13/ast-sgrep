# Pass 7 — Failure-path traces (exception graph)

| Field | Value |
|-------|-------|
| Loop | 7 / error-exception-cleanup-flow |
| Freeze | `fb932aac852f5496c0a7035cc5a0b508e05111cb` (product freeze retained; HEAD may hold books) |
| Axes | representation:**exception-graph** · observer:**failure-handler** · scale:**entrypoint→cleanup** · time:**degradation** |
| Axes vs pass 6 | **4** (call-trace / runtime / entrypoint→sink / normal → this set) |
| Mode | audit (no product edits) |
| Evidence | source-level static traces; zerostack `fszero-codemode` missing (B-ZS-ENGINES) |
| Inputs | pass 6 residuals + pass 5 INV ledger + pass 4 EPs |

**Failure** here means: early return, `Err`/`bail!`/`ensure!`, soft miss envelope, deadline, budget, poison recovery, or partial stats -- not crash-only.

Happy-path counterparts: pass 6 `HP-*`. This pass IDs failure modes as `FM-*` and cleanup rows as `CL-*`.

---

## Catalog index (from pass 6 residuals)

| ID | Class | Surfaces | Primary anchors |
|----|-------|----------|-----------------|
| **FM-JAIL** | authz / path | MCP | `sandbox_root` |
| **FM-CM-ROOT-OK** | authz asymmetry | CM/Pi | `root_arg` (no jail fail) |
| **FM-EMPTY-CLI** | empty index hard | CLI | `ensure_nonempty_index` |
| **FM-EMPTY-MCP** | empty index soft miss | MCP | `diagnose_miss` / `to_compact_miss_json` |
| **FM-DEADLINE-WAIT** | timeout admission | MCP | `tool_index_repo` pre-start |
| **FM-DEADLINE-POST** | soft timeout after mutation | MCP | `tool_index_repo` post-work |
| **FM-MAX-CALLS** | call budget | CM | `bump_call` |
| **FM-POISON-MCP** | lock poison recover | MCP | `lock_or_recover` |
| **FM-POISON-CM** | lock poison fail / soft invalidate | CM | `searcher_for` / `invalidate_searcher_cache` |
| **FM-EMBED-DENY** | URL allowlist / no-redirect | core+embed | `embed_url_is_allowed` |
| **FM-EMBED-HYBRID-ABORT** | mid-cascade embed `?` | core | `search_hybrid` stage D |
| **FM-EMBED-CHAIN-FALLBACK** | preferred backend miss | embed | `embed_with_chain` |
| **FM-MID-INDEX-BULK** | bulk write fail | core | `apply_bulk_write_result` |
| **FM-MID-INDEX-SIDECAR** | post-commit sidecar fail | core+MCP/CM | `index_all` after commit |
| **FM-REINDEX-GEN** | generation activate refuse | core | `reindex_into_new_generation` |
| **FM-PER-FILE-SOFT** | per-file prepare fail | core | `PrepareOutcome::Failed` |
| **FM-CASCADE-LEX-EMPTY** | empty lexical stop | core | `search_hybrid` early `Ok([])` |
| **FM-CASCADE-STRUCT-EMPTY** | empty structural continue | core | ht1h.3 / C1 |
| **FM-PARSE-TOOL** | unknown tool | MCP/CM | `dispatch_tool` / `CallError::UnknownTool` |
| **FM-PARSE-QUERY** | oversize / empty query | MCP/core | `MAX_QUERY_CHARS` |
| **FM-PARSE-NODE** | bad `code_read` id | MCP | `parse_node_id` |
| **FM-BATCH-PARTIAL** | per-call error envelope | CM batch | `invoke` / `all_ok` |
| **FM-PI-DEGRADE** | NAPI miss → CLI sticky | Pi | `NativeSessionPool.#start` |
| **FM-RERANK-SKIP** | swallowed rerank Err | core | `finish_response` path |
| **FM-LEDGER-SKIP** | swallowed ledger Err | core | `try_append_ledger` |

---

## FM-JAIL — MCP workspace jail fail

### Trigger
`tools/call` with `root` (or path arg) outside configured workspace, or non-existent root.

### Exception graph

```
handle_tools_call
  → dispatch_tool → parse_* → resolve_root / sandbox_root
       candidate.exists()? canonicalize : bail("does not exist...")
       ensure!(canonical.starts_with(&self.root), "root {} escapes configured workspace {}")
  → Err → content text + isError: true
```

### User-visible
JSON-RPC tool result: `isError: true`, message includes `escapes configured workspace`.

### Cleanup
No Searcher open; no path_registry write; no index mutation.

### Invariants
**INV-MCP-SANDBOX** enforced. **INV-SURFACE-ROOT-PARITY** contradiction vs CM (see FM-CM-ROOT-OK).

### Anchors
`crates/ast-sgrep-mcp/src/lib.rs` `sandbox_root` ~547–570; `handle_tools_call` ~375–392.

---

## FM-CM-ROOT-OK — Code Mode free root (failure asymmetry)

### Trigger
CM/Pi pass absolute `root` outside host workspace. **Does not fail** at surface policy.

### Graph

```
CodeModeSession::root_arg → PathBuf::from(str) | config.root
  → Searcher::new / Indexer::new (OS permissions only)
```

### Observer note
Pass 7 documents the **absence** of a jail failure handler as the C2 dual of FM-JAIL. Not a product bug re-file; already CONTRADICTION C2 / GAP-CM-ROOT.

### Invariants
**INV-CM-ROOT-FREE** (gap/intentional free), **INV-SURFACE-ROOT-PARITY** C2.

---

## FM-EMPTY-CLI vs FM-EMPTY-MCP — empty index contracts

### CLI hard fail (FM-EMPTY-CLI)

```
open_searcher → Searcher::new → ensure_nonempty_index(file_count)
  file_count==0 → bail!("index is empty for {}; run: asgrep index ...")
```

Process exit via `anyhow` chain (non-zero). No miss envelope.

### MCP soft miss (FM-EMPTY-MCP)

```
tool_agent_search
  → searcher_for (opens even if empty)
  → search_* → hits empty
  → restore_searcher  (always, before ?)
  → to_compact_miss_json(diagnose_miss)
       indexed_files from Indexer::status().file_count
       reason EmptyIndex when indexed_files == Some(0)
  → Ok(json) → isError: false   << success envelope for operational empty
```

### Semantic contradiction
Same underlying empty store: CLI **Err**, MCP **Ok + why=empty_index**. Agents must branch on `why`, not only `isError`.

### Invariants
None named EMPTY-*; related **INV-LIMITS** N/A. Surfaces diverge (pass 6 residual 2).

### Anchors
CLI: `index_cmd.rs` `ensure_nonempty_index` ~44–52.  
MCP: `tool_agent_search` ~661–686; `diagnose_miss` ~755–789; plugins `MissContext::reason` EmptyIndex.

---

## FM-DEADLINE-WAIT / FM-DEADLINE-POST — MCP `index_repo` deadline

`INDEX_REPO_DEADLINE = 600s`. Single-flight via `index_lock`.

### Wait admission (FM-DEADLINE-WAIT)

```
tool_index_repo:
  started = Instant::now()
  lock_or_recover(index_lock)   // wait counts toward deadline
  ensure!(elapsed < DEADLINE) else "exceeded ... before start"
  // no Indexer work yet
```

Cleanup: lock released on drop; no cache invalidate; no disk write.

### Post-mutation soft timeout (FM-DEADLINE-POST)

```
  index_all / reindex_all succeeds
  invalidate_searcher_cache()          // ALWAYS before post check (d2a1.13)
  path_registry.clear(); emitted_snippets.clear()
  ensure!(elapsed <= DEADLINE) else "exceeded ... deadline"
  // client sees Err, disk already mutated, caches cleared
```

### Observer: success-after-failure shape
From agent POV: **error** after durable index mutation. Not a silent rollback. Documented intentional soft deadline; state is consistent (new index + cold searcher). Residual for ops: callers may retry index unnecessarily.

### Invariants
**INV-MCP-SEARCHER-INV** held (invalidate before error return). **INV-DURABILITY-FC** orthogonal (writes committed before ensure).

### Anchors
`mcp/lib.rs` ~56, ~861–898.

---

## FM-MAX-CALLS — Codemode call budget

```
CodeModeSession::call
  → bump_call: if calls >= max_calls (default 64) → Err("codemode call budget exceeded")
  → else calls += 1; call_tool
```

- No tool body runs on budget fail (budget checked before dispatch).
- Batch serial sets `max_calls = MAX_BATCH_CALLS * 4`; parallel per-call sessions use 8; serve may raise to 10_000.
- Cleanup: none needed (no partial tool side effect for this call). Prior calls already counted.

### Invariants
Resource bound sibling of **INV-LIMITS** (surface budget, not query chars).

### Anchors
`codemode/session.rs` ~88–97; `batch.rs` ~220–233, ~260.

---

## FM-POISON-MCP vs FM-POISON-CM — lock poison handlers

### MCP (recover)

```
lock_or_recover(mutex, clear):
  Ok(guard) | Err(poisoned) → clear_poison; into_inner; clear(&mut); return guard
searcher_for on poison: generation++, entry=None, rebuild Searcher
```

Fail-closed toward **rebuild**, not reuse of tainted entry.

### CM (asymmetric)

```
searcher_for: lock().map_err(|_| "searcher cache lock poisoned")?   // hard Err
invalidate_searcher_cache:
  if let Ok(mut guard) = lock() { *guard = None; }   // poison → NO-OP invalidate
```

**CL-CM-POISON-INV:** if a prior holder poisoned the mutex, `index_repo` can succeed on disk while `invalidate_searcher_cache` silently skips; subsequent `searcher_for` fails closed with poison error rather than rebuild. Weaker than MCP generation model (pass 6 residual / INV-CM-SEARCHER-INV GAP).

### Anchors
MCP ~581–642; CM ~99–103, ~126–129.

---

## FM-EMBED-DENY / FM-EMBED-HYBRID-ABORT / FM-EMBED-CHAIN-FALLBACK

### URL deny (FM-EMBED-DENY) — INV-EMBED-ALLOW

```
embed_url_is_allowed(url):
  scheme must be http|https
  host allowlist (+ ASGREP_EMBED_URL_ALLOWLIST)
  http non-loopback requires ASGREP_EMBED_ALLOW_INSECURE_HTTP=1
embed_http_agent: redirects(0)  // allowlist is final hop
```

Config construction: `CloudEmbeddingConfig::from_env` / Ollama use `.ok()?` → **backend absent** (no HTTP).  
Runtime `embed_via_api`: returns `Err(String)` if deny.

### Hybrid stage D abort (FM-EMBED-HYBRID-ABORT)

```
search_hybrid:
  lexical → structural working set
  if use_embed && stop.is_none():
    hits.extend(embed_pass_for_files(...)?)   // ? propagates
```

If `embed_query` / model mismatch / semantic-v1 rewrite required → **entire hybrid returns Err**, discarding already-collected lexical+structural hits. Fail-closed for semantic integrity; **degrades user-visible success** (partial good results lost).

### Chain fallback vs message (FM-EMBED-CHAIN-FALLBACK)

`embed_with_chain` eprints "refusing silent hashed Semantic — set ASGREP_EMBED_FALLBACK=1" when Cloud/Ollama preference unavailable **without** the flag, then **still returns local Semantic** vector. Message implies refuse; code soft-falls. Tension for fail-closed docs (not URL allowlist -- INV-EMBED-ALLOW still holds for HTTP).

Stored-backend path `embed_query` **does** hard-Err on missing backend / dim mismatch (search semantic integrity).

### Anchors
`ast-sgrep-embed/embedder.rs` ~27–90, ~151–152, ~485–511, ~565–580; `search/mod.rs` ~529–536; `passes/embed.rs` `embed_query_vector` ~255–305.

---

## FM-MID-INDEX-* — index failure + cleanup

### Bulk write fail (FM-MID-INDEX-BULK) — clean rollback

```
index_all:
  begin_bulk_tx
  commit_prepared_files → Result
  apply_bulk_write_result:
    Ok → commit_bulk_tx
    Err(e) → rollback_bulk_tx; prefer restore_synchronous Err over e (d2a1.2)
```

Active generation untouched on write Err (when rollback succeeds). MCP/CM invalidate **not** reached (`?` before invalidate) -- correct because live index unchanged.

### Per-file soft fail (FM-PER-FILE-SOFT)

`PrepareOutcome::Failed` → eprint + `stats.files_failed++` + continue. `index_all` can still **Ok**. Surfaces invalidate as success path. Partial corpus indexed.

### Post-commit sidecar fail (FM-MID-INDEX-SIDECAR) — **cleanup gap**

```
apply_bulk_write_result(Ok)   // SQLite already committed new rows
rebuild_dirty_sidecars?       // tantivy / semantic IVF may Err here
post_index_hooks?
```

On sidecar Err:
- SQLite reflects new content; sidecars may be stale/missing.
- MCP `tool_index_repo` / CM `index_repo`: **invalidate not executed** (error before that line).
- **Stale Searcher cache** may keep serving pre-mutation hits until process restart or successful later index.

### Generation reindex refuse (FM-REINDEX-GEN) — good compensation

`reindex_into_new_generation`: build candidate → `verify_candidate_generation` (integrity, file_count>0, smoke search) → only then write active manifest. Fail leaves previous active. Explicit pin `index_path` / `ASGREP_INDEX_PATH` uses in-place clear+index (weaker; INV-INDEX-PATH-PRIV GAP).

### Anchors
`index.rs` ~231–284, ~307–310, ~481–489, ~519–596; `store/sqlite.rs` `apply_bulk_write_result` ~540–548; MCP ~881–891; CM ~248–261.

---

## FM-CASCADE-LEX-EMPTY / FM-CASCADE-STRUCT-EMPTY

| Stage empty | Behavior | Hits |
|-------------|----------|------|
| Lexical files empty | `return Ok(Vec::new())` stop | no structural/embed |
| Structural empty | `working_files = lexical_files`; may embed (ht1h.3) | lexical (+ optional semantic) |

C1: docs claiming full stop on empty structural remain CONTRADICTION; error-shape is **success with fewer stages**, not Err. Empty-lexical is the true hard empty of cascade (not empty-structural).

---

## FM-PARSE-* — boundary parse

| Mode | Handler | Exit |
|------|---------|------|
| Unknown tool MCP | `dispatch_tool` `Err("unknown tool: {other}")` | isError true |
| Unknown tool CM | `CallError::UnknownTool` | Err to host |
| Oversize query MCP | `parse_agent_search` ensure chars ≤ MAX_QUERY_CHARS (4096) | isError true |
| Oversize query core | `validate_query_len` StoreError | Err |
| Bad node id | `parse_node_id` context errors; relative path components only | isError true |
| Unknown JSON-RPC method | method handler Err | JSON-RPC error |
| Oversized JSON-RPC line | parse error id:null code -32700 | protocol |

`deny_unknown_fields` on wire structs maps serde errors via `map_wire_error`.

### Anchors
MCP dispatch ~397–424; parse_node_id ~901–926; main loop ~224–250; limits `MAX_QUERY_CHARS`; CM `tools.rs` CallError.

---

## FM-BATCH-PARTIAL — serial partial failure

```
run_batch → run_serial | run_parallel
invoke: Ok → {ok:true,value} | Err → {ok:false,error:string}
BatchResponse { all_ok, results[], ... }
```

- Validation fail (empty calls, too many, blank id) → whole-batch `CallError` (no partial).
- Per-call fail does **not** abort remaining serial calls.
- Mutators force serial (`choose_parallel` any `!read_only` → false) — **INV-BATCH-NO-MUT-PAR**.
- Unknown tool names: `is_read_only` false via catalog miss → treated as mutator for parallel policy (conservative).

Cleanup: each call independent; shared serial session retains Searcher across calls (including after failed call, if open succeeded earlier).

---

## FM-PI-DEGRADE — Pi sticky backend degradation

```
NativeSessionPool.#start(root):
  loadCodemodeNative()?
    try new Session → inProcessWorker
    catch → fall through
  if !binary → return null
  try CLI sticky worker
  catch → return null
generation mismatch mid-start → worker.end().catch(()=>undefined); return null
pool.end: worker.end().catch(()=>undefined)  // swallow end errors
```

Degradation path is intentional. Double-fail returns null (host must surface). Freshness/index failures live in `FreshnessCoordinator.ensureFresh` (INDEX_STATUS_UNKNOWN, CANCELLED, index failed) -- separate from pool start.

---

## FM-RERANK-SKIP / FM-LEDGER-SKIP — swallowed non-critical

| Site | Behavior | Risk |
|------|----------|------|
| rerank Err | eprint `rerank skipped`; return unre-ranked hits | success with degraded ranking |
| ledger append Err | eprint / skip; search still Ok | audit trail gap |
| lexicon rebuild Err | eprint skip in `post_index_hooks` | search still serves |
| nested ROLLBACK `let _ =` in sqlite | intentional; outer `apply_bulk_write_result` surfaces restore | documented pass9 residual closed for product path |

Not authz swallows; mark as **degraded success** under time:degradation axis.

---

## MCP searcher restore discipline (positive control)

```
tool_agent_search:
  (searcher, gen) = searcher_for?
  response = search_*(...)   // Result not unwrapped yet
  restore_searcher(...)      // always, even if search Err
  response?
```

Cleanup on search Err: Searcher returned to cache if generation still matches. **Good** exception-safe pattern (CL-MCP-RESTORE).

---

## Residual pointers → pass 8 (data sinks)

- Query / path / embed URL / node-id strings as untrusted sources into store, HTTP, FS open.
- Compact path_registry as capability map for `code_read` (validation order).
- `ASGREP_INDEX_PATH` absolute sink privilege (INV-INDEX-PATH-PRIV).
- Embed request body / API key env (not re-audited here beyond allowlist).
- Batch / plan `$ref` resolution as internal dataflow (plan.rs).

# Pass 7 — Error-semantic contradictions & inconsistent contracts

Cross-surface failure meaning for the same underlying condition. Links pass-5 INV status.

## ESC-1 — Empty index: hard Err vs soft Ok miss

| Surface | Signal | Code path |
|---------|--------|-----------|
| CLI | process/anyhow **Err** | `ensure_nonempty_index` |
| MCP | tool **Ok**, `isError:false`, `why: empty_index` | `to_compact_miss_json` |
| CM/core | open Searcher allowed; search returns empty hits | no ensure_nonempty |

**Impact:** automations that only check MCP `isError` miss empty-index; must parse compact `why`.  
**INV:** none dedicated; related surface parity theme of C2.  
**Status:** documented divergence (not new product R-* this pass).

## ESC-2 — Jail fail vs free root success (C2 live on error axis)

| Surface | Outside-workspace root |
|---------|------------------------|
| MCP | **Err** `escapes configured workspace` |
| CM/Pi/NAPI | **can succeed** under OS perms |

**INV-SURFACE-ROOT-PARITY** CONTRADICTION · **INV-MCP-SANDBOX** CONSISTENT · **INV-CM-ROOT-FREE** GAP.  
Error axis makes asymmetry sharp: MCP has a failure handler CM lacks by design.

## ESC-3 — Soft deadline after successful mutation

MCP `index_repo` may return **isError true** after disk mutation + cache invalidate.  
Contract: soft wall-clock, not transactional timeout.  
**Not** silent success-after-failure on disk; **is** error-after-success for the RPC.  
Agents retrying blindly re-run expensive index.

**INV-MCP-SEARCHER-INV** still holds (no stale serve).

## ESC-4 — Cascade empty-structural: success path, not error

Empty structural does **not** enter an error handler; continues with lexical (+ optional embed).  
Docs claiming stop remain **C1 / INV-CASCADE-STRUCT-EMPTY** CONTRADICTION.  
True cascade "empty" failure shape is **empty lexical → Ok([])**, not Err.

## ESC-5 — Embed "refuse fallback" vs still-Semantic

`embed_with_chain` warns that silent hashed Semantic is refused unless `ASGREP_EMBED_FALLBACK=1`, then returns Semantic anyway.  
**Semantic contradiction** between log contract and return value.  
Distinct from **INV-EMBED-ALLOW** (HTTP allowlist -- still CONSISTENT).  
Stored-backend `embed_query` remains hard-fail on mismatch (good).

## ESC-6 — Hybrid embed fail: integrity over partial hits

Stage D `?` converts embed/config failure into full search Err after cheaper stages succeeded.  
Contract prioritizes **no mixed-generation / wrong-backend semantic** over **best-effort lexical response**.  
Opposite of rerank (FM-RERANK-SKIP: degrade and continue).  
Inconsistent **degradation policy** across optional neural features (rerank soft / embed stage hard when use_embed).

## ESC-7 — CM poison: fail search vs no-op invalidate

| API | Poison behavior |
|-----|-----------------|
| `searcher_for` | Err |
| `invalidate_searcher_cache` | ignore (leave poisoned guard content) |

MCP always recovers. CM error contract is **incomplete** vs MCP for same conceptual cache.  
**INV-CM-SEARCHER-INV** GAP reinforced on failure axis.

## ESC-8 — Per-file index fail vs index_all Ok

`files_failed > 0` still yields successful stats JSON from MCP/CM index tools.  
Partial corpus is **success-with-stats**, not error. CLI same core path.  
Honest if clients read `files_failed`; easy to miss if only `ok: true`.

## ESC-9 — Budget spend on failed tool body (minor)

`bump_call` increments before `call_tool`; tool Err still consumes budget.  
Admission control is **attempt-counted**, not **success-counted**. Document for operators.

## Swallowed vs fail-closed summary

| Pattern | Examples | Class |
|---------|----------|-------|
| Fail-closed authz/SSRF | sandbox, embed URL, redirects 0, feature flags | good |
| Fail-closed integrity | bulk rollback prefer restore Err, semantic-v1 rewrite, model mismatch | good |
| Soft degrade | rerank skip, ledger skip, lexicon skip, empty miss envelope | intentional |
| Message/code mismatch | embed_with_chain fallback | honesty residual |
| Cleanup skip | mid-index sidecar + no invalidate | **gap** |
| Asymmetry | CLI empty hard / MCP empty soft; MCP jail / CM free | documented |

## Link table → pass 5 INV

| INV | Error-axis note |
|-----|-----------------|
| INV-MCP-SANDBOX | FM-JAIL enforced |
| INV-CM-ROOT-FREE | FM-CM-ROOT-OK no fail handler |
| INV-SURFACE-ROOT-PARITY | ESC-2 |
| INV-MCP-SEARCHER-INV | restore + deadline invalidate OK; mid-sidecar gap |
| INV-CM-SEARCHER-INV | poison invalidate gap |
| INV-BATCH-NO-MUT-PAR | choose_parallel mutator force serial |
| INV-EMBED-ALLOW | URL + no-redirect; chain fallback separate |
| INV-DURABILITY-FC | bulk rollback; gen reindex; pinned path weaker |
| INV-CASCADE-STRUCT-EMPTY | ESC-4 / C1 |
| INV-CASCADE-NO-WIDEN | working_files retain; not error-path issue |
| INV-LIMITS | query/parse bounds; max_calls sibling |
| INV-RO-CATALOG | index_repo still callable on error paths (no approval) |
| INV-XOR-CM-MCP | docs-only; Pi degrade not XOR enforcement |
| INV-INDEX-PATH-PREC/PRIV | open path errors; pinned reindex window |
| INV-RANK-FUSION | only on success finish; n/a pure Err |
| INV-AST-GREP / INV-EDIT-ROOT | not primary this pass |

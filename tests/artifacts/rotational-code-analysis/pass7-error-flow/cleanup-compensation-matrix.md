# Pass 7 — Cleanup / compensation matrix

Maps critical **side effects** to failure exits and restore outcomes. Gate: every side effect row has a documented failure + cleanup outcome (or UNKNOWN/GAP).

Legend: **Y** restored/consistent · **N** gap · **P** partial · **I** intentional soft · **n/a** no SE

## Matrix

| Side effect | Owner | Fail mode | Cleanup / compensation | Outcome | INV |
|-------------|-------|-----------|------------------------|---------|-----|
| MCP Searcher cache entry | MCP | search Err after take | `restore_searcher` always before `?` | **Y** | INV-MCP-SEARCHER-INV |
| MCP Searcher cache | MCP | index_repo success or post-deadline Err | invalidate + gen++ before deadline ensure | **Y** | INV-MCP-SEARCHER-INV |
| MCP Searcher cache | MCP | index_all/reindex **Err** mid-path | invalidate **skipped** | **N** if bulk already committed (sidecar fail); **Y** if rolled back | see FM-MID-INDEX-SIDECAR |
| MCP path_registry | MCP | index_repo post-success path | `.clear()` with searcher invalidate | **Y** | elision gen |
| MCP path_registry | MCP | index fail before clear | retains pre-index ids | **P** ids may point at old paths; code_read may 404 | residual |
| MCP emitted_snippets | MCP | index_repo success path | `.clear()` | **Y** | comment: no elision across gens |
| MCP emitted_snippets | MCP | index fail | not cleared | **P** elide across failed index attempt | residual |
| MCP index_lock | MCP | any exit from tool_index_repo | MutexGuard drop | **Y** | single-flight |
| MCP lock poison | MCP | any lock_or_recover | clear_poison + clear fn | **Y** rebuild | INV-MCP-SEARCHER-INV |
| CM Searcher Option cache | CM | index_repo Ok | `invalidate` sets None if lock Ok | **Y** if unpoisoned | INV-CM-SEARCHER-INV soft |
| CM Searcher Option cache | CM | index_repo Ok + poisoned mutex | invalidate **no-op** | **N** CL-CM-POISON-INV | GAP |
| CM Searcher | CM | searcher_for poison | hard Err (no rebuild) | **I** fail-closed, weaker UX | GAP |
| SQLite bulk write | core | commit_prepared Err | `apply_bulk_write_result` rollback + restore_synchronous | **Y** (prefer rb Err) | INV-DURABILITY-FC |
| SQLite bulk write | core | commit Ok, sidecar rebuild Err | no bulk rollback; index rows live | **P** DB new / sidecar stale | residual |
| Generation reindex | core | verify/smoke fail | active manifest not switched; previous retained | **Y** | INV-DURABILITY-FC |
| In-place reindex (pinned path) | core | fail after `clear_all_data` | **destructive window** | **P/N** vs gen path | INV-INDEX-PATH-PRIV GAP |
| Per-file prepare fail | core | PrepareOutcome::Failed | count files_failed; continue | **I** soft partial Ok | -- |
| Lexicon rebuild | core | post_index_hooks Err on lexicon | eprint skip; index Ok | **I** degraded | -- |
| Hybrid hit list | core | embed stage D Err | return Err; lexical hits dropped | **I** fail-closed integrity / **P** UX | INV-EMBED related |
| Hybrid hit list | core | empty lexical | Ok([]) early | **Y** empty | -- |
| Embed HTTP | embed | URL deny / no-redirect | no request / Err | **Y** | INV-EMBED-ALLOW |
| Embed preferred backend | embed | cloud/ollama unavailable | eprint + still Semantic vector | **I/P** message vs code | residual |
| Feature flags neural/rerank | core | build without feature | Searcher::new Err | **Y** fail-closed | -- |
| Rerank optional | core | runtime rerank Err | eprint skip; unre-ranked hits | **I** degraded success | -- |
| Search ledger | core | append fail | try_append skip | **I** trail gap | -- |
| CM call counter | CM | max_calls | no increment on fail path? **increments only after check passes** | **Y** | -- |
| CM call counter | CM | tool body Err after bump | call still counted | **I** spent budget on fail | residual minor |
| Batch per-call | CM | invoke Err | `{ok:false}`; continue others | **Y** envelope | INV-BATCH-NO-MUT-PAR |
| Batch mutator parallel | CM | any non-read_only | force serial | **Y** | INV-BATCH-NO-MUT-PAR |
| Pi worker start | Pi | NAPI throw | catch → CLI sticky | **Y** degrade | -- |
| Pi worker start | Pi | both fail | null; end catch swallow | **P** host must detect null | -- |
| Pi generation race | Pi | gen mismatch after start | worker.end(); discard | **Y** | -- |
| MCP sandbox | MCP | jail fail | no open | **Y** | INV-MCP-SANDBOX |
| CLI empty index | CLI | file_count==0 | bail; no search | **Y** hard | -- |
| MCP empty index | MCP | file_count==0 | Ok miss why=empty_index | **I** soft | contradiction vs CLI |
| Snapshot read during search | core | gen change mid-search | Err retry; COMMIT/ROLLBACK snapshot | **Y** | consistency |
| Nested file_tx poison | core | nested begin | not Ok after poison (tests pin) | **Y** | pass3 residual closed |

## Cleanup gaps (named)

| ID | Gap | Severity | Residual to |
|----|-----|----------|-------------|
| **CL-MID-SIDECAR-CACHE** | Post-commit sidecar Err skips MCP/CM searcher invalidate → stale cache possible | **high** (consistency) | pass 9 concurrency + harden |
| **CL-CM-POISON-INV** | `invalidate_searcher_cache` ignores poison; no rebuild path | **medium** | INV-CM-SEARCHER-INV GAP |
| **CL-INDEX-FAIL-REGISTRIES** | path_registry / emitted_snippets uncleared on index Err | **low–medium** | pass 8 capability map |
| **CL-PINNED-REINDEX** | explicit index_path in-place clear window | **medium** (durability) | INV-INDEX-PATH-PRIV |
| **CL-EMBED-MSG** | "refuse fallback" eprint then Semantic | **low** (honesty) | docs / pass 8 embed sink |
| **CL-HYBRID-EMBED-DROP** | embed Err drops prior stage hits | **medium** (UX/degrade) | product judgment (ask) |

## Positive controls (do not "fix")

- MCP `restore_searcher` on search Err path.
- MCP invalidate **before** post-deadline soft Err (d2a1.13).
- `apply_bulk_write_result` no `let _ =` on product rollback path (pass9).
- Generation reindex activate-after-verify.
- Batch mutators never parallel with readers.
- Embed HTTP redirects(0) + allowlist.

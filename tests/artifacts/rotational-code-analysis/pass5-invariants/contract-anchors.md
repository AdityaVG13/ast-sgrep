# Pass 5 — Contract → code/test anchors

Maps each invariant to concrete anchors for later loops (happy path, error, sinks).

| INV | Doc | Code | Test / oracle |
|-----|-----|------|---------------|
| INV-MCP-SANDBOX | `docs/env-trust.md` MCP workspace | `mcp/lib.rs::sandbox_root` | `mcp/tests/protocol.rs::tool_roots_are_sandboxed_*` |
| INV-CM-ROOT-FREE | `docs/codemode.md` no OS jail | `codemode/session.rs::root_arg` | **missing negative** |
| INV-SURFACE-ROOT-PARITY | (none unified) | MCP vs CM pair | pass-4 policy map P1 |
| INV-INDEX-PATH-PREC | `docs/getting-started.md` index path | `store/mod.rs::try_index_db_path` | `testkit/isolation.rs` poison env |
| INV-INDEX-PATH-PRIV | (weak) | same resolver | no security test |
| INV-MCP-SEARCHER-INV | MCP module docs | `tool_index_repo` + gen invalidate | `index_repo_invalidates_searcher_*` |
| INV-CM-SEARCHER-INV | batch/session warm notes | `session::index_repo` | **missing parity** |
| INV-BATCH-NO-MUT-PAR | batch module docs | `batch::choose_parallel` | `batch_never_parallelizes_index_repo_*` |
| INV-RO-CATALOG | catalog `read_only` comment | `catalog.rs`, adapters | catalog expose tests only |
| INV-XOR-CM-MCP | `docs/codemode.md`, `docs/mcp.md`, skills | (none runtime) | (none) |
| INV-EMBED-ALLOW | `docs/env-trust.md` | `embed_url_is_allowed` + `redirects(0)` | unit tests metadata/evil/file |
| INV-DURABILITY-FC | store Durability docs | `Durability::{parse,from_env}` | `store_pragmas.rs` |
| INV-CASCADE-NO-WIDEN | `docs/cascade-query-planner.md` | `search_hybrid` working_files | `cascade_planner.rs` |
| INV-CASCADE-STRUCT-EMPTY | **doc conflict** | `working_files` fallback | `cascade_stops_when_*` |
| INV-AST-GREP | `docs/env-trust.md` | `pattern.rs` dual gate | env removal tests |
| INV-EDIT-ROOT | pi-package / edit | `edit.ts::planEdit` | pi extension tests |
| INV-LIMITS | limits comments | `limits.rs` + Searcher::new | unit clamps |
| INV-RANK-FUSION | `docs/fusion-ranking.md` | `finish_response` / RRF | `ranking_oracle.rs` |

## Critical scenarios covered (gate)

| Scenario | INV(s) |
|----------|--------|
| Agent indexes foreign tree via MCP | INV-MCP-SANDBOX |
| Agent indexes foreign tree via Code Mode | INV-CM-ROOT-FREE, INV-SURFACE-ROOT-PARITY |
| Index path hijack via env | INV-INDEX-PATH-PREC, INV-INDEX-PATH-PRIV |
| Stale search after reindex | INV-MCP-SEARCHER-INV, INV-CM-SEARCHER-INV |
| Parallel batch races mutation | INV-BATCH-NO-MUT-PAR |
| Silent SSRF via embed URL | INV-EMBED-ALLOW, INV-EMBED-NO-REDIR |
| Durability silent downgrade | INV-DURABILITY-FC |
| Semantic expands beyond lexical | INV-CASCADE-NO-WIDEN |
| Hybrid stop on weak structure | INV-CASCADE-STRUCT-EMPTY |
| Unapproved index from PTC sandbox | INV-RO-CATALOG |
| Dual surface load | INV-XOR-CM-MCP |
| External binary spawn | INV-AST-GREP |
| Source write escape | INV-EDIT-ROOT |
| Unbounded results/query | INV-LIMITS |

Gate (**Each critical scenario has ≥1 falsifiable invariant + evidence source**): **MET**.

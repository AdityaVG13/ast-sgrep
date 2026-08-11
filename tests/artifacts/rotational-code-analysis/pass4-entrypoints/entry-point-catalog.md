# Pass 4 — Entry-point catalog

Machine-readable twin: [`entry-point-catalog.json`](./entry-point-catalog.json).

## Process / package entries

| ID | Name | Surface | Trust | Side effects |
|----|------|---------|-------|--------------|
| EP-CLI-ASGREP | `asgrep` | CLI bin | OS user / full FS | index R/W, search, watch, serve, embed |
| EP-CLI-AST-SGREP | `ast-sgrep` | CLI alias | same | same |
| EP-CLI-SUB-INDEX | index/reindex | CLI sub | OS user | **write** index |
| EP-CLI-SUB-SEARCH | search family | CLI sub | OS user | read + optional embed |
| EP-CLI-SUB-WATCH | watch | CLI long-run | OS user | continuous index write |
| EP-CLI-SUPERVISOR | worker supervisor | process ctl | nonce+marker | spawn/kill |
| EP-CLI-CODEMODE-BATCH | codemode-batch | CLI JSON IPC | OS user | multi-tool warm |
| EP-CLI-CODEMODE-SERVE | codemode-serve | CLI NDJSON | stdio local | sticky tools |
| EP-CLI-META | capabilities/doctor/… | CLI meta | OS user | probe / bench / eval |
| EP-MCP-SERVER | `asgrep-mcp` | MCP stdio | OS user + **ASGREP_ROOT jail** | tools R/W |
| EP-MCP-TOOLS | 7 tools | tools/call | sandboxed roots | index_repo write |
| EP-LSP-SERVER | `asgrep-lsp` | LSP stdio | workspace URI jail | ensure/reindex, nav |
| EP-VSCODE | vscode extension | editor | host trust | spawn LSP |
| EP-NAPI | codemode-napi | N-API | same Node | Session.call/batch |
| EP-CODEMODE-SESSION | 12 tools | lib API | inherits; **root unsandboxed** | index_repo write |
| EP-PI-EXTENSION | pi-ast-sgrep | Pi tools | Pi user + edit jail | Code Mode + edit + index |
| EP-PI-LAUNCHER | npm launcher | bin spawn | binary integrity | exec asgrep |
| EP-AGENT-PLUGIN | agent-plugin | MCP pack | host + skill trust note | spawn mcp |
| EP-ENV | ASGREP_* | env plane | ambient | configures sinks |
| EP-FUZZ | fuzz targets | dev | CI/dev | crash harness |
| EP-PLUGINS-LIB | plugins formatters | internal lib | n/a | format only |
| EP-CORE-LIB | core API | internal | n/a | Indexer/Searcher |

## MCP tools

| Tool | State-changing | Notes |
|------|----------------|-------|
| keyword_search | no | lexical |
| ast_search | no | native pattern |
| semantic_search | no | embed channel |
| code_search | no | deprecated → keyword |
| code_read | no | FS read under root |
| index_status | no | |
| index_repo | **yes** | single-flight + deadline |

## Codemode tools

`search`, `semantic`, `chain`, `defs`, `callers`, `imports`, `index_status`, **`index_repo`** (only non-read_only), `filter_hits`, `select`, `catalog_search`, `catalog_describe`.

## Pi tools / commands / hooks

| Kind | Names |
|------|-------|
| Tools | `asgrep`, `asgrep_search`, `asgrep_index`, `asgrep_status`, **`asgrep_edit`** |
| Commands | `asgrep-doctor`, `asgrep-status`, `asgrep-index`, `asgrep-reindex` |
| Hook | `tool_result` → mark freshness dirty on write/edit/asgrep_edit |

## LSP

**Requests:** initialize, shutdown, workspace/symbol, documentSymbol, definition, references, call hierarchy trio, executeCommand.

**Notifications:** initialized, exit, didOpen/Change/Save/Close.

**executeCommand:** asgrep.search, .search.semantic, **.reindex**, .callers, .defs.

## NAPI exports

`Session` (new, call, call_count, root), `batch`, `binding_version`, `is_native`.

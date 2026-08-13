# Query guide

| Goal | MCP action | Example |
| --- | --- | --- |
| Find a literal string | exact-text search | `ASGREP_TIMEOUT_MS` |
| Find code by purpose | `keyword_search` | `refresh the index after edits` |
| Find a syntax shape | `ast_search` | `await $CLIENT.fetch($URL)` |
| Locate a symbol | `keyword_search` | `FreshnessCoordinator` |
| Broaden intent retrieval | `semantic_search` | `native package selection` |
| Expand a hit | `code_read` with its compact ID | `p3:120-145` |

## Failure recovery

- Missing executable: verify `asgrep-mcp` is installed and executable. Do not download or execute an arbitrary replacement binary.
- Missing index: call `index_repo`, then retry the same query.
- Incompatible or corrupt index: call `index_repo` with `force: true`, then retry.
- Root outside the configured workspace: choose a path under `ASGREP_ROOT`. Do not relax confinement without explicit user authorization.
- Deadline or output-limit failures: narrow the query or reduce the limit; do not silently discard the structured error.

For an unfamiliar codebase, inspect `index_status`, reconcile with `index_repo` if needed, search with a small limit, and expand only promising compact IDs with `code_read`.

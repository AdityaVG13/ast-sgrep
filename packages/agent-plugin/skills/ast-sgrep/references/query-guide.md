# Query guide

| Goal | Pi action | Example |
| --- | --- | --- |
| Find a literal string | exact-text search | `ASGREP_TIMEOUT_MS` |
| Find code by purpose | `asgrep` calling `asgrep.search` | `refresh the index after edits` |
| Find a syntax shape | `asgrep.search` / `asgrep_search` with `mode: "pattern"` | `await $CLIENT.fetch($URL)` |
| Locate a symbol definition | `asgrep.defs` or `asgrep_search` `mode: "defs"` | `FreshnessCoordinator` |
| Locate callers | `asgrep.callers` or `asgrep_search` `mode: "callers"` | `ensureFresh` |
| Trace a flow | `asgrep.chain` | `write to next search` |
| Broaden intent retrieval | `asgrep.semantic` | `native package selection` |
| Compose many lookups | **`asgrep`** with `Promise.all` | parallel defs + callers |

## Failure recovery

- `BINARY_NOT_FOUND` or `UNSUPPORTED_PLATFORM`: run `/asgrep-doctor`; inspect the structured details and package installation. Do not download or execute an arbitrary replacement binary.
- `INDEX_MISSING`: run `/asgrep-index`, then retry the same query.
- `INDEX_INCOMPATIBLE`: run `/asgrep-reindex`, then retry.
- `ROOT_OUTSIDE_PROJECT`: choose a path inside the current project. Do not relax confinement without explicit user authorization.
- `TIMEOUT`, cancellation, or output-limit failures: narrow the query or reduce the limit; do not silently discard the error envelope.

For an unfamiliar codebase, prefer `asgrep`: doctor/status/index via slash commands, then one Code Mode program that searches, picks a symbol, and fans out `defs`/`callers`/`chain` with `Promise.all`. Return a shaped object — not every intermediate hit list.

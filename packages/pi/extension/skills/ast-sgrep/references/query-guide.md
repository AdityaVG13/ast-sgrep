# Query guide

| Goal | Pi action | Example |
| --- | --- | --- |
| Find a literal string | exact-text search | `ASGREP_TIMEOUT_MS` |
| Find indexed terms | `asgrep_search` with `mode: "keyword"` | `refresh index edits` |
| Find a whole word | `asgrep_search` with `mode: "word"` | `word: auth` |
| Find an exact phrase | `asgrep_search` with `mode: "literal"` | `literal: refresh token` |
| Find by regex | `asgrep_search` with `mode: "regex"` | `regex: auth_.*` |
| Find imports of a module | `asgrep_search` with `mode: "imports"` | `imports: express` |
| Find code by purpose | `asgrep_search` with `mode: "semantic"` | `refresh the index after edits` |
| Find a syntax shape | `asgrep_search` with `mode: "pattern"` | `await $CLIENT.fetch($URL)` |
| Locate a symbol definition | `asgrep_search` with `mode: "defs"` | `FreshnessCoordinator` |
| Locate callers | `asgrep_search` with `mode: "callers"` | `ensureFresh` |
| Trace a flow | `asgrep_search` with `mode: "chain"` | `write to next search` |
| Broaden intent retrieval | `asgrep_search` with `mode: "semantic"` | `native package selection` |

## Failure recovery

- `BINARY_NOT_FOUND` or `UNSUPPORTED_PLATFORM`: run `/asgrep-doctor`; inspect the structured details and package installation. Do not download or execute an arbitrary replacement binary.
- `INDEX_MISSING`: run `/asgrep-index`, then retry the same query.
- `INDEX_INCOMPATIBLE`: run `/asgrep-reindex`, then retry.
- `ROOT_OUTSIDE_PROJECT`: choose a path inside the current project. Do not relax confinement without explicit user authorization.
- `TIMEOUT`, cancellation, or output-limit failures: narrow the query or reduce the limit; do not silently discard the error envelope.

For an unfamiliar codebase, a deterministic first pass is: `/asgrep-doctor`, `/asgrep-status`, one bounded `keyword`, `pattern`, or `semantic` query with no excerpts, then a `defs`, `callers`, or `chain` query for the selected symbol. Read full code only for selected result IDs.

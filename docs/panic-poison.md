# Panic / poison integrity matrix (`ast-sgrep-sxjc`)

| Surface | Failure mode | Policy |
|---------|--------------|--------|
| Regex pass worker | `join` panic | Propagate `StoreError`; never `unwrap_or_default` empty hits |
| Searcher response cache | Mutex poison | `clear_poison` + clear map / invalidate generation |
| Semantic embed cache | Mutex poison | `clear_poison` + drop `Option` slot |
| META_CACHE (prevented reads) | Mutex poison | `clear_poison` + clear map |
| IVF session cache | Mutex poison | `clear_poison` + clear entries |
| MCP Searcher cache | Mutex poison | Invalidate (`None`) before reuse |
| MCP `index_repo` | Concurrent calls | Single-flight `index_lock` + wall deadline |
| LSP `index_lock` | Mutex poison | Mark `index_ready=false`, clear poison, allow rebuild |

Panic-injection coverage: `search::tests::lock_clear_on_poison_resets_state` and
regex worker fail-closed mapping in `regex` unit tests.

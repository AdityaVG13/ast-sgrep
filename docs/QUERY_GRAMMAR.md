# Query prefixes

Normative surface for `ParsedQuery::parse` in `crates/ast-sgrep-core/src/query.rs`.
There is no composable `AND` / `path:` / `lang:` / `sem:` grammar — unprefixed
input is hybrid retrieval; one leading mode prefix selects a single channel.

## Mode prefixes

| Prefix | Mode | Notes |
|--------|------|--------|
| *(none)* | Hybrid | Lexical + structural + semantic fusion |
| `callers:` | Callers | Case-insensitive symbol graph |
| `defs:` | Definitions | Case-insensitive symbol lookup |
| `imports:` | Imports | Case-insensitive module substring |
| `pattern:` | Structural | Native tree-sitter patterns |
| `literal:` | Literal | Exact substring (GLOB/LIKE escaped) |
| `regex:` | Regex | Line regex |
| `word:` | Word | Token / word boundary |

Examples:

```text
process_request
callers:RefreshToken
defs:auth_refresh
imports:./Utils
pattern:function $NAME($$$)
literal:foo_bar
regex:foo.*bar
word:token
```

## What is not supported

- Clause conjunction (`AND`, multiple prefixes in one query)
- `sem:`, `path:`, `lang:` filter clauses
- Nested / parenthesized boolean expressions

Use CLI/MCP/LSP options (e.g. language filter, semantic-only search) for filters
that are not part of the query string.

## Related

- [How it works](how-it-works.md) — hybrid ranking overview
- [Semantic search](semantic-search.md) — embed backends
- [Structural patterns](../README.md) — pattern examples in the main README

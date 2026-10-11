# Query prefixes

This document specifies how query strings parse (`ParsedQuery::parse` in `crates/ast-sgrep-core/src/query.rs`), so implementations and tests agree.
The parser has no composable `path:` / `lang:` / `sem:` grammar. Unprefixed
input is hybrid retrieval; one leading mode prefix selects a single channel.
One layer above the parser, `Searcher::search` recognizes exactly one
two-channel conjunction form; see "Two-channel conjunction" below.

Clause IDs **QG-xxx** (ghiw.2). Prefix controls live in
`tests/core/oracle_query.rs::parsed_query_modes_scopes_and_targets`; public scope
and conjunction behavior live in `tests/core/path_scope_ingress.rs` and
`tests/core/conjunction_queries.rs`. The matrix distinguishes those controls
from clauses without a mapped regression. No full conformance score or MUST%
is certified here.

## Mode prefixes

| Prefix | Mode | Notes |
|--------|------|--------|
| *(none)* | Hybrid | Lexical + structural + semantic fusion |
| `callers:` | Callers | Case-insensitive symbol graph |
| `defs:` | Definitions | Case-insensitive symbol lookup |
| `imports:` | Imports | Case-insensitive module substring |
| `pattern:` | Structural | Native tree-sitter patterns |
| `literal:` | Literal | Exact substring; query terms preserve case |
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

## Root-relative `in:` scope

A whitespace-delimited `in:path` token outside double quotes scopes the query
without changing its mode. The token is extracted while the remaining query's
interior bytes are preserved; `raw` is that trimmed remaining payload, including
its mode prefix. Single quotes are not scope-token quoting syntax.

- A file scope selects that exact indexed relative path; a directory selects its
  descendants. `in:src`, `in:./src` and `in:src/.` select the same directory;
  repeated separators and dot components use canonical relative hit identity.
- `in:.` or `in:./` selects the root. Normalization is lexical, not filesystem
  canonicalization: it does not follow symlinks or rewrite Unix backslash names.
- Wildcard scopes (`*`, `?`) use the existing file-filter matching rules and need
  not exist on disk. Equivalent dot/separator spellings are normalized too.
- Bare, duplicate, absolute, parent-traversing and nonexistent non-glob scopes
  refuse loudly. Authored non-glob paths are checked before normalization, so a
  file with a trailing slash still refuses.
- Explicit `SearchOptions.file_filter` takes precedence over a valid query scope;
  it does not suppress malformed-scope refusal. Each conjunction channel applies
  its scope before joining. Programmatic needle APIs (`search_literal`,
  `search_regex`, `search_word`) do not interpret scope syntax inside the needle.

Public regressions: `tests/core/path_scope_ingress.rs` (including
`relative_scope_spellings_preserve_canonical_membership`) and
`tests/core/conjunction_queries.rs::{conjunction_applies_each_channel_scope_before_joining,conjunction_rejects_invalid_scopes_on_either_channel}`.

## MUST matrix

Prefixes are **case-sensitive** (`Callers:Foo` is Hybrid, not Callers). `parse`
trims the full input; after scope extraction, `raw` is the trimmed remaining
payload (prefix kept). `target` for
prefixed modes is the remainder after the first matching prefix, then trimmed.
Unprefixed Hybrid sets `target: None`.

| ID | MUST | Observed contract | Test |
|---|---|---|---|
| QG-001 | Unprefixed input is Hybrid | `process_request` → `QueryMode::Hybrid`, `target: None` | `oracle_query::parsed_query_modes_scopes_and_targets` (Hybrid control) |
| QG-002 | `callers:` selects Callers | remainder is `target` | `oracle_query::parsed_query_modes_scopes_and_targets` |
| QG-003 | `defs:` selects Defs | remainder is `target` | `oracle_query::parsed_query_modes_scopes_and_targets` |
| QG-004 | `imports:` selects Imports | remainder is `target` | `oracle_query::parsed_query_modes_scopes_and_targets` |
| QG-005 | `pattern:` selects Pattern | remainder is `target`; terms = `[target]` | `oracle_query::parsed_query_modes_scopes_and_targets` |
| QG-006 | `literal:` selects Literal | terms preserve case | `oracle_query::parsed_query_modes_scopes_and_targets` |
| QG-007 | `regex:` selects Regex | terms preserve case | `oracle_query::parsed_query_modes_scopes_and_targets` |
| QG-008 | `word:` selects Word | terms are lowercased | `oracle_query::parsed_query_modes_scopes_and_targets` |
| QG-009 | `raw` retains the mode prefix | `raw` is the trimmed payload after scope extraction, with its mode prefix kept | `oracle_query::parsed_query_modes_scopes_and_targets` (Literal control; not an all-mode matrix) |
| QG-010 | `parse` never panics | arbitrary-string property remains a contract, not certified coverage | No mapped property regression |
| QG-011 | Empty target after a prefix is well-defined | `callers:` / `pattern:` → `target: Some("")`, mode still prefixed | No mapped empty-prefixed-target regression |
| QG-012 | Target remainder is trimmed | `defs:  auth` → target `auth` | `oracle_query::parsed_query_modes_scopes_and_targets` (Callers trim control) |

## Unsupported / negative matrix

These are **not errors**. The parser does not reject unknown grammar; it
selects the first leading mode prefix or Hybrid-as-text.

| ID | MUST-not / negative | Observed contract | Test |
|---|---|---|---|
| QG-020 | No `sem:` query filter | `sem:foo` is Hybrid; the string is scored as text | Parser source contract; no mapped regression |
| QG-021 | No `path:` query filter | `path:src/` is Hybrid-as-text | Parser source contract; no mapped regression |
| QG-022 | No `lang:` query filter | `lang:rust foo` is Hybrid-as-text | Parser source contract; no mapped regression |
| QG-023 | No composable AND / multi-prefix **in the parser** | `callers:Foo defs:Bar` is Callers with target `Foo defs:Bar`; `Searcher::search` layers the two-channel `AND` form above the parser | Parser source contract; Searcher controls in `conjunction_queries` |
| QG-024 | Nested / parenthesized boolean unsupported | `(defs:Foo AND callers:Bar)` is Hybrid-as-text (no leading prefix) | Parser/conjunction source contract; no mapped parenthesized regression |
| QG-025 | Prefix match is case-sensitive | `Callers:Foo` is Hybrid, not Callers | Parser source contract; no mapped case-mismatch regression |
| QG-026 | Unknown prefix is Hybrid-as-text | `xyzzy:Foo` is Hybrid | Parser source contract; no mapped unknown-prefix regression |
| QG-027 | A literal / word answer prefers the code that *uses* the needle | a JSON fixture or a comment that merely quotes it ranks below the code line (shortlist-context rule in the critic) | `literal_answers_prefer_use_over_mention_and_reach_definitions` |
| QG-028 | `word:<Identifier>` / `literal:<Identifier>` also return the declaration | the def lane joins the line lane for identifier-shaped needles, so the def ranks first | `literal_answers_prefer_use_over_mention_and_reach_definitions` |

Use CLI/MCP/LSP options (language filter, semantic-only search) for filters
that are not part of the query string. `pattern:` completeness vs ast-grep is
**ghiw.3** (`DISC-pattern-native-subset`), not this matrix.

## Two-channel conjunction (Searcher level)

`Searcher::search` recognizes exactly one composed form before the parser
runs (`crates/ast-sgrep-core/src/search/conjunction.rs`):

```text
<channel> AND <channel>
<channel> AND NOT <channel>
```

- Both sides must be prefixed channel queries: `defs:`, `callers:`,
  `imports:`, `pattern:`, `literal:`, `regex:`, `word:`, or
  `semantic:"..."` (embedding-only retrieval, quotes optional).
- `AND` is uppercase and space-delimited; `NOT` / `not` both negate.
- Exactly two channels. More `AND`s, unprefixed sides, or parenthesized
  forms fall through to ordinary search, so plain English "AND" keeps its
  meaning (QG-023 / QG-024 still hold at the parser).
- The left channel is the result identity. Pattern/caller pairs join by span:
  `pattern:... AND callers:x` returns only pattern spans containing a call to
  `x`, and the reversed order returns only caller hits inside a matching
  pattern span. Other pairs join by file. `AND NOT` subtracts at the same
  scope. Overlapping right evidence merges into the kept hit's contributors.
- Empty channels stay honest: empty left is empty; empty right makes `AND`
  empty and `AND NOT` a no-op.

```text
callers:process_request AND pattern:fn $NAME($$$)
pattern:fn $NAME($$$) AND callers:process_request
imports: rusqlite AND semantic:"parameterized query"
defs:handle AND NOT callers:test_
```

Tests: `tests/core/conjunction_queries.rs`.

## What is not supported

- More than two channels in one conjunction
- Unprefixed (hybrid text) sides in a conjunction
- `sem:`, `path:`, `lang:` filter clauses
- Nested / parenthesized boolean expressions

## Related

- [How it works](how-it-works.md), hybrid ranking overview
- [Semantic search](semantic-search.md), embed backends
- [Structural patterns](../README.md), pattern examples in the main README

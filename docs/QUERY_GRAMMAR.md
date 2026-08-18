# Query prefixes

Normative surface for `ParsedQuery::parse` in `crates/ast-sgrep-core/src/query.rs`.
The parser has no composable `path:` / `lang:` / `sem:` grammar — unprefixed
input is hybrid retrieval; one leading mode prefix selects a single channel.
One layer above the parser, `Searcher::search` recognizes exactly one
two-channel conjunction form; see "Two-channel conjunction" below.

Clause IDs **QG-xxx** (ghiw.2). Tests: `tests/unit/core/query.rs` (lib
`query::tests`) and `tests/core/properties.rs` (`parse_never_panics`). Score is
**TBD** until a full conformance run (ghiw.5). Do not quote MUST% from this
file.

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

## MUST matrix

Prefixes are **case-sensitive** (`Callers:Foo` is Hybrid, not Callers). `parse`
trims the full input; `raw` is that trimmed string (prefix kept). `target` for
prefixed modes is the remainder after the first matching prefix, then trimmed.
Unprefixed Hybrid sets `target: None`.

| ID | MUST | Observed contract | Test |
|---|---|---|---|
| QG-001 | Unprefixed input is Hybrid | `process_request` → `QueryMode::Hybrid`, `target: None` | `qg_must_matrix` |
| QG-002 | `callers:` selects Callers | remainder is `target` | `qg_must_matrix` |
| QG-003 | `defs:` selects Defs | remainder is `target` | `qg_must_matrix` |
| QG-004 | `imports:` selects Imports | remainder is `target` | `qg_must_matrix` |
| QG-005 | `pattern:` selects Pattern | remainder is `target`; terms = `[target]` | `qg_must_matrix` |
| QG-006 | `literal:` selects Literal | terms preserve case | `qg_must_matrix`, `literal_and_regex_terms_preserve_case` |
| QG-007 | `regex:` selects Regex | terms preserve case | `qg_must_matrix`, `literal_and_regex_terms_preserve_case` |
| QG-008 | `word:` selects Word | terms are lowercased | `qg_must_matrix` |
| QG-009 | `raw` retains the mode prefix | `raw == trimmed input` for every prefixed mode | `raw_keeps_mode_prefix_across_all_modes` |
| QG-010 | `parse` never panics | property over arbitrary strings | `tests/core/properties.rs` `parse_never_panics` |
| QG-011 | Empty target after a prefix is well-defined | `callers:` / `pattern:` → `target: Some("")`, mode still prefixed | `qg_must_matrix` |
| QG-012 | Target remainder is trimmed | `defs:  auth` → target `auth` | `qg_must_matrix` |

## Unsupported / negative matrix

These are **not errors**. The parser does not reject unknown grammar; it
selects the first leading mode prefix or Hybrid-as-text.

| ID | MUST-not / negative | Observed contract | Test |
|---|---|---|---|
| QG-020 | No `sem:` query filter | `sem:foo` is Hybrid; the string is scored as text | `qg_must_matrix` |
| QG-021 | No `path:` query filter | `path:src/` is Hybrid-as-text | `qg_must_matrix` |
| QG-022 | No `lang:` query filter | `lang:rust foo` is Hybrid-as-text | `qg_must_matrix` |
| QG-023 | No composable AND / multi-prefix **in the parser** | `callers:Foo defs:Bar` is Callers with target `Foo defs:Bar`; `Searcher::search` layers the two-channel `AND` form above the parser | `qg_must_matrix` |
| QG-024 | Nested / parenthesized boolean unsupported | `(defs:Foo AND callers:Bar)` is Hybrid-as-text (no leading prefix) | `qg_must_matrix` |
| QG-025 | Prefix match is case-sensitive | `Callers:Foo` is Hybrid, not Callers | `qg_must_matrix` |
| QG-026 | Unknown prefix is Hybrid-as-text | `xyzzy:Foo` is Hybrid | `qg_must_matrix` |

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

Tests: `tests/unit/core/search__conjunction.rs` and
`tests/core/conjunction_queries.rs`.

## What is not supported

- More than two channels in one conjunction
- Unprefixed (hybrid text) sides in a conjunction
- `sem:`, `path:`, `lang:` filter clauses
- Nested / parenthesized boolean expressions

## Related

- [How it works](how-it-works.md) — hybrid ranking overview
- [Semantic search](semantic-search.md) — embed backends
- [Structural patterns](../README.md) — pattern examples in the main README
- [DISCREPANCIES](validation/DISCREPANCIES.md) — registered intentional divergences

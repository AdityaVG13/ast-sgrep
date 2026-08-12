# Symbol and graph-key normalization

This is the fixture-backed contract for Issue #12 and `ast-sgrep-7uz6`.
It links the original divergence to `ast-sgrep-powt` and names the two
normalization surfaces that must remain aligned:

- retrieval preparation: `search/passes/symbol.rs::prefixed_mode_query`
- ranking comparison: `rank.rs::score_normalized_symbol`

## Stored form

The index preserves extractor spelling. `IndexStore::insert_callers` writes
`CallerRow.caller` and `CallerRow.callee` unchanged. `insert_imports` likewise
writes `ImportRow.module_path` unchanged. It does not lowercase, strip module
qualifiers, or rewrite fully-qualified names.

The parity fixture proves the concrete stored keys:

| table column | source | stored value |
| --- | --- | --- |
| `callers.callee` | Rust call `RefreshToken()` | `RefreshToken` |
| `imports.module_path` | TypeScript `from './Utils'` | `./Utils` |

`tests/core/parity.rs` reads these values back from SQLite,
prints them beside the queried forms, and then exercises case variants through
the public `Searcher` API.

## Queried form

`ParsedQuery` preserves the text after `callers:`, `defs:`, or `imports:` in
`target`. `lookup_symbol` returns that target with its casing, punctuation, and
qualification intact. `prefixed_mode_query` replaces scoring terms with this
single complete target so a qualified key is not split into unrelated tokens.

Retrieval then applies channel-specific SQL equivalence:

| mode | predicate | equivalence |
| --- | --- | --- |
| `callers:` | `lower(c.callee) = lower(?)` | exact, ASCII-case-insensitive in SQLite |
| `defs:` | `lower(s.name) = lower(?)` | exact, ASCII-case-insensitive in SQLite |
| `imports:` | `lower(i.module_path) LIKE '%' || lower(?) || '%' ESCAPE '\\'` | escaped substring, ASCII-case-insensitive in SQLite |

The original `powt` failure occurred after SQL retrieval: mixed-case prefixed
queries retained their raw target while ranking compared it against normalized
symbols. `score_normalized_symbol` now normalizes both term and symbol before
exact or substring scoring, so retrieved rows keep a positive score.

## Qualification and remaining boundaries

- Fully-qualified and module-qualified text is stored as extracted. No shared
  FQN canonicalizer currently strips `::`, `.`, `/`, package prefixes, or file
  extensions.
- Callers and definitions require equality after case folding. Imports retain
  the broader substring behavior required for module-path lookup.
- Rust ranking uses Unicode lowercase, while SQLite `lower()` and default
  `LIKE` are ASCII-oriented. Non-ASCII case equivalence is therefore not part
  of the current SQL contract and must be handled by a separate follow-up if
  required.

Any future write-path canonicalization must change both SQL retrieval and
`score_normalized_symbol` together. Changing only one side recreates the
indexed-but-not-retrievable class fixed by `ast-sgrep-powt`.

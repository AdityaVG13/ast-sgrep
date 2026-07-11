# ast-sgrep Composable Query Grammar

This document defines the normative syntax implemented in `crates/ast-sgrep-core/src/query/grammar.rs`. The grammar is a flat, prefix-oriented conjunction language designed to be backward-compatible with legacy single-prefix queries.

## Overview

A query is a sequence of **clauses** separated by whitespace and/or the explicit `AND` keyword. All clauses are evaluated in conjunction: there is no operator precedence because there are no nested expressions. The runtime fuses results from every searchable channel, deduplicates them, and ranks the combined set. `path:` and `lang:` filters apply globally to all channels.

## Lexical tokens

The input is tokenized before parsing.

- **Whitespace** (`\s+`) separates tokens. Leading and trailing whitespace are ignored.
- **Quoted phrase**: a double-quoted string (`"..."`) is a single token; the quotes themselves are not part of the value. Whitespace inside quotes is preserved.
- **Bare word**: a maximal run of characters that are neither whitespace nor `"`. Backslash escapes are decoded while tokenizing.
- **Uppercase `AND`**: an unquoted bare token whose value is exactly `AND` is the explicit conjunction operator. Lowercase `and` is treated as an ordinary word.
- **`OR` and parentheses are not supported.**

### Escapes

Backslash escapes are decoded in both bare and quoted values. The supported escapes are:

| Escape | Result |
|--------|--------|
| `\\` | literal backslash |
| `\"` | literal double quote |
| `\n` | newline (`U+000A`) |
| `\t` | tab (`U+0009`) |
| `\ ` (backslash followed by a space) | literal space |

Any other escape sequence is a syntax error. A trailing backslash at the end of the input is also an error. The parser points to the byte where the escape problem begins (1-indexed in error messages).

### Lexical examples

```text
retry timeout
"retry with backoff"
hello\ world            # one token, value "hello world"
sem:"line\nnext\tword"  # value contains a newline and a tab
```

## Clauses

A clause is one of the following.

### Bare terms and quoted phrases

Any token that is not a recognized prefix clause and is not `AND`/`OR` is a **lexical term**. Lexical terms are joined with a single space and passed through `ParsedQuery::parse` as a hybrid-mode query. The set of lexical terms therefore contributes to scoring and retrieval.

Quoted phrases are treated as a single lexical term, preserving internal whitespace.

### Filter prefixes

Filters take a single value. They may be written inline (`prefix:value`) or with the value in the next token (`prefix: value`). They may appear anywhere in the query and apply to all channels.

| Prefix | Meaning | Duplicate behavior |
|--------|---------|-------------------|
| `sem:` | Semantic/natural-language search clause | error if supplied twice |
| `pattern:` | Structural pattern (ast-sgrep pattern). In the composable grammar this is a filter; it is also a legacy single-prefix query form. | error if supplied twice |
| `path:` | Glob-style path filter | error if supplied twice |
| `lang:` | Language filter | error if supplied twice |

The value of a filter is the literal token text after decoding escapes.

### Mode prefixes

Mode prefixes select a non-hybrid search channel. Exactly one mode prefix may appear in a composable query. If lexical terms are also present, they are combined into the same `ParsedQuery`: the mode target plus the bare terms, with duplicate terms removed.

| Prefix | Channel |
|--------|---------|
| `callers:` | Call-graph callers search |
| `defs:` | Symbol definitions search |
| `imports:` | Import search |
| `literal:` | Exact substring search |
| `regex:` | Rust-regex search |
| `word:` | Word-boundary literal search |

Mode prefixes are also allowed as the sole clause of a legacy query; see backward compatibility below.

### Prefix binding

A prefix binds **only** to its immediate value. There is no grouping beyond the next token.

- Inline form: `prefix:value` consumes the whole token.
- Two-token form: `prefix: value` consumes the value token that follows the prefix token. If `AND` immediately follows the prefix (e.g. `sem: AND x`), the error is `expected a value before AND`.
- A value may not be empty: `sem:` alone reports `sem: requires a value`.

## Conjunction

- `AND` is uppercase only. Lowercase `and` is a normal term.
- Conjunction is **flat**: `a b c`, `a AND b AND c`, and `a b AND c` all parse identically.
- `AND` must appear between clauses. Leading or trailing `AND`, or two `AND`s in a row, is an error.
- Implicit AND has the same flat precedence as explicit `AND`.

## Unsupported syntax

- `OR` is not supported. Use `AND` only.
- Parentheses are not supported. All clauses are conjoined at the top level.
- Any `prefix:` token where `prefix` is alphabetic and not in the recognized prefix list is an error with a message that lists the expected prefixes.

## Errors

The parser reports errors with a 1-indexed byte position and a descriptive message:

```text
query syntax error at byte N: <message>
```

The position is the byte offset of the offending token or character. The philosophy is to fail fast at the first structural problem rather than to attempt recovery.

Representative errors:

- Empty query: `query is empty`
- Leading `AND`: `expected a clause before AND`
- Trailing `AND`: `expected a clause after AND`
- Missing value after prefix: `<prefix>: requires a value`
- `AND` after prefix: `<prefix>: requires a value before AND`
- Duplicate filter: `duplicate <prefix>: clause; provide it only once`
- Duplicate mode: `duplicate mode clause <prefix>:; use only one mode prefix`
- Unknown prefix: `unknown clause <prefix>:; expected sem:, pattern:, path:, lang:, or a mode prefix`
- `OR`: `OR is not supported; combine clauses with AND`
- Parentheses: `parentheses are not supported; all clauses are conjoined`
- Filters without searchable clause: `query has filters but no searchable clause; add terms, pattern:, or sem:`
- Unterminated quote: `unterminated quoted phrase; add a closing double quote`
- Unsupported escape: `unsupported escape \<c>; use \\, \", \n, \t, or escaped space`

## Backward compatibility: legacy single-prefix queries

If a query, after trimming, begins with one of the legacy prefixes and contains **no composable marker** after the prefix, it is passed through `ParsedQuery::parse` unchanged and stored as `legacy`. The legacy prefixes are:

- `callers:`
- `defs:`
- `imports:`
- `pattern:`
- `literal:`
- `regex:`
- `word:`

A **composable marker** is either the token `AND` or any recognized prefix token (`sem:`, `pattern:`, `path:`, `lang:`, or a mode prefix). If any composable marker appears after the leading prefix, the query is parsed by the new composable grammar.

This rule preserves existing queries such as `literal: "A AND B"` (the `AND` is inside a quoted value, not a composable marker) and `regex: foo\s+bar`.

## Channels and result fusion

A query can activate up to three searchable channels simultaneously:

1. **Lexical/mode channel** — from bare terms, or from a single mode prefix (`callers:`, `defs:`, `imports:`, `literal:`, `regex:`, `word:`). The `pattern:` and `sem:` filters are separate channels.
2. **Pattern channel** — from `pattern:`.
3. **Semantic channel** — from `sem:`.

`path:` and `lang:` filters are applied to every active channel. Results from all channels are fused, deduplicated, and ranked as a single list.

## Canonical examples

### Pattern and semantic filters

```text
pattern:"fn $NAME" AND sem:"retry with backoff" path:src/**
```

Parses to:

- `pattern`: `fn $NAME`
- `semantic`: `retry with backoff`
- `path_filter`: `src/**`

### All three channels with lexical terms

```text
retry timeout AND pattern:"fn $NAME" sem:"retry with backoff" path:src/** lang:rust
```

Parses to:

- lexical terms: `retry`, `timeout`
- `pattern`: `fn $NAME`
- `semantic`: `retry with backoff`
- `path_filter`: `src/**`
- `lang_filter`: `rust`

Results from the lexical, pattern, and semantic channels are filtered by the path/language predicates and then merged.

### Mode prefix with extra lexical terms

```text
defs:ParsedQuery AND scoring
```

Produces a `ParsedQuery` in `Defs` mode with target `ParsedQuery` and terms that include both `ParsedQuery` and `scoring` (with duplicates removed).

### Legacy query preserved exactly

```text
literal: "A AND B"
```

Because the only `AND` is inside a quoted value, this is a legacy query and goes through `ParsedQuery::parse` unchanged. The value is `"A AND B"` (including the quotes, matching historical behavior) and the query remains in `Literal` mode.

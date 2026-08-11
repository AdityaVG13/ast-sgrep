# Analysis card: `literal_sql`

| Field | Value |
|---|---|
| Rank | 25 |
| File | `crates/ast-sgrep-core/src/search/passes/literal.rs:67` |
| CC (baseline) | 18 |
| Classification | **`extractable`** |
| Technique | `lookup_table` |
| Pass wave | 5 |
| Resolve | **Cut** |
| Risk | medium |
| Run | `2026-08-10T235424Z-baseline` |

## Summary

LIKE vs GLOB × lang filter SQL selection plus word-mode postfilter.

## Branch groups

| Branch group | Classification | Note |
|---|---|---|
| `case_insensitive × lang SQL matrix` | `accidental_structure` | table of SQL templates |
| `word_mode postfilter` | `essential_domain` | query mode |
| `context file map` | `essential_domain` | excerpt option |

## Technique

Wave 5: select SQL by (case_insensitive, has_lang) key. Preserve escape behavior.

## Parity plan (for transform pass)

- Prefer existing unit/integration tests that touch this path.
- Differential: same inputs before/after for public behavior (error codes, JSON fields, ranking order).
- Re-measure lizard on the enclosing file; **ΣCC must not rise** without justified displacement.

## Status

- Pass 2: classified only — **no product edit**.

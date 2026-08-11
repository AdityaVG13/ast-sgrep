# Analysis card: `argvFor`

| Field | Value |
|---|---|
| Rank | 12 |
| File | `packages/pi/extension/src/codemode/dispatch.ts:231` |
| CC (baseline) | 22 |
| Classification | **`accidental_structure`** |
| Technique | `lookup_table` |
| Pass wave | 5 |
| Resolve | **Cut** |
| Risk | high |
| Run | `2026-08-10T235424Z-baseline` |

## Summary

Tool name → CLI argv switch. Classic dictionary dispatch candidate.

## Branch groups

| Branch group | Classification | Note |
|---|---|---|
| `switch tool cases` | `accidental_structure` | table of builders |
| `index_repo force ternary` | `essential_domain` | reindex vs index |
| `catalog_* no-CLI fallback` | `essential_domain` | sticky-only tools |

## Technique

Map tool → (args)=>string[] builders. Preserve default and catalog fallback semantics.

## Parity plan (for transform pass)

- Prefer existing unit/integration tests that touch this path.
- Differential: same inputs before/after for public behavior (error codes, JSON fields, ranking order).
- Re-measure lizard on the enclosing file; **ΣCC must not rise** without justified displacement.

## Status

- Pass 2: classified only — **no product edit**.

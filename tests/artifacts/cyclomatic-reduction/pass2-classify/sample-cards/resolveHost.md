# Analysis card: `resolveHost`

| Field | Value |
|---|---|
| Rank | 3 |
| File | `packages/pi/launcher/src/index.js:76` |
| CC (baseline) | 29 |
| Classification | **`accidental_structure`** |
| Technique | `guard_clause` |
| Pass wave | 3 |
| Resolve | **Cut** |
| Risk | high |
| Run | `2026-08-10T235424Z-baseline` |

## Summary

Platform package resolution with sequential metadata/version checks. Mapping is already table-driven (HOSTS); remaining CC is nested try/catch fail ladders.

## Branch groups

| Branch group | Classification | Note |
|---|---|---|
| `HOSTS mapping miss` | `essential_domain` | unsupported platform; keep as early fail |
| `requireResolve / readFile / parse try-catch` | `accidental_structure` | replace with guard + fail helpers |
| `os/cpu/libc/version checks` | `essential_domain` | package integrity; keep sequential guards |

## Technique

Convert try/catch blocks to early `fail(...)` after small `readJson` helper. Preserve error codes.

## Parity plan (for transform pass)

- Prefer existing unit/integration tests that touch this path.
- Differential: same inputs before/after for public behavior (error codes, JSON fields, ranking order).
- Re-measure lizard on the enclosing file; **ΣCC must not rise** without justified displacement.

## Status

- Pass 2: classified only — **no product edit**.

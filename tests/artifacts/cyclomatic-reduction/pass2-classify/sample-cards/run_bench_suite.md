# Analysis card: `run_bench_suite`

| Field | Value |
|---|---|
| Rank | 2 |
| File | `crates/ast-sgrep-cli/src/bench.rs:231` |
| CC (baseline) | 29 |
| Classification | **`extractable`** |
| Technique | `extract_method` |
| Pass wave | 4 |
| Resolve | **Cut (extract)** |
| Risk | high |
| Run | `2026-08-10T235424Z-baseline` |

## Summary

CLI bench suite driver: fixture/suite resolve, timed cases, identity checks, JSON vs human report, ratchet.

## Branch groups

| Branch group | Classification | Note |
|---|---|---|
| `fixture/suite selection` | `essential_domain` | user inputs; keep as guards |
| `per-case timing + identity` | `extractable` | extract `run_suite_case` |
| `json vs human print + exit` | `accidental_structure` | extract `emit_suite_report` |
| `bench ratchet` | `essential_domain` | policy gate; keep |

## Technique

Structure-preserving extracts: `run_suite_case`, `emit_suite_report`. Bench-only; lower product risk but still in ΣCC.

## Parity plan (for transform pass)

- Prefer existing unit/integration tests that touch this path.
- Differential: same inputs before/after for public behavior (error codes, JSON fields, ranking order).
- Re-measure lizard on the enclosing file; **ΣCC must not rise** without justified displacement.

## Status

- Pass 2: classified only — **no product edit**.

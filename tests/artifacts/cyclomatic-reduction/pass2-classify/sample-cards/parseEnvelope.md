# Analysis card: `parseEnvelope`

| Field | Value |
|---|---|
| Rank | 1 |
| File | `packages/pi/extension/src/runtime.ts:366` |
| CC (baseline) | 31 |
| Classification | **`essential_domain`** |
| Technique | `extract_method` |
| Pass wave | 7 (optional error-path extract) |
| Resolve | **Keep** |
| Risk | high |
| Run | `2026-08-10T235424Z-baseline` |

## Summary

Machine-protocol envelope validation for CLI/NAPI exec results. Sequential field and exit-code checks are requisite variety for the wire contract.

## Branch groups

| Branch group | Classification | Note |
|---|---|---|
| `output byte limit (stdout/stderr/sum)` | `essential_domain` | DoS bound; keep |
| `nonzero exit + nested JSON failure parse` | `extractable` | duplicate OPERATIONAL_ERROR path; extract helper |
| `JSON parse / object shape / tool / schema / ok` | `essential_domain` | protocol fields; keep |
| `assertVersionTriple` | `essential_domain` | version identity; keep |

## Technique

Do not flatten protocol checks. Optional pass 7: extract `throwOperationalFailure` + `parseFailedEnvelope` to drop nested try/catch without removing varieties.

## Parity plan (for transform pass)

- Prefer existing unit/integration tests that touch this path.
- Differential: same inputs before/after for public behavior (error codes, JSON fields, ranking order).
- Re-measure lizard on the enclosing file; **ΣCC must not rise** without justified displacement.

## Status

- Pass 2: classified only — **no product edit**.

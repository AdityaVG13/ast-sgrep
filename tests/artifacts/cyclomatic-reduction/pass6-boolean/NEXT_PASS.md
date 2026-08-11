# NEXT_PASS.md

Run ID: `2026-08-10T235424Z-baseline`
Completed: **Pass 6 — Boolean / nesting density**
Next: **Pass 7 — Error-path extracts** (`parseEnvelope` optional; launcher residuals)

## Pass 6 outcome (this session)

| Function | CC before → after | Technique | Helpers |
|---|---|---|---|
| `ensureFresh` | 23 → **10** | combine_predicates + decompose | `needsIncrementalIndex` (4), `probeIndexHealth` (1), `runIndex` (3) |
| `assertVersionTriple` | 7 → 7 | compound guard | — |
| `asSearchResponse` | 10 → 10 | nested compound → guard | — |

Touched-file ΣCC: **483 → 481 (−2)**. Extension package total_cc **806 → 804**.

Parity: `packages/pi/extension` npm test **88 passed**. Artifacts: `06-transformed-code/pass6-*`, `07-parity-report-pass6.md`, `08-complexity-scorecard-pass6.md`. Mirror: `tests/artifacts/cyclomatic-reduction/pass6-boolean/`.

## Explicit Keep / Refuse notes

- Health varieties on ensureFresh **kept** (Ashby).
- `summarizeCodemode` sequential/extract attempts **raised** CC or ΣCC — left as nested ternary.
- `wireLinesValid` early-return form **doubled** CC (6→12) — reverted.
- `isValidHitShape` / `indexHealth` — Keep domain OR chains.

## Do next (Pass 7)

1. Load via `.cyclomatic-reduction/LATEST` → `2026-08-10T235424Z-baseline`.
2. **Do not re-baseline** unless scope changes.
3. Error-path extract batch from plan:
   - `parseEnvelope` (CC 31) — extract failed-envelope / OUTPUT_LIMIT ladder only; **Keep** field validation chain
   - Optional launcher residual catch ladders if still hot after pass 3
4. Prefer joint-allowed targeted tests (extension npm; launcher if touched).

## Residual after pass 6 (above ceiling 10)

| Function | After CC | Note |
|---|---:|---|
| parseEnvelope | 31 | pass 7 primary |
| readLineWindow | 25 | Keep |
| isValidHitShape | 18 | Keep |
| indexHealth | 16 | Keep |
| #start | 16 | session-pool |
| summarizeCodemode | 15 | refuse flatten under lizard |
| rebuildIncompatibleIndex | 11 | residual |
| ensureFresh | 10 | at hard ceiling |
| argvFor / searchToolCall | 11 | pass 5 residual |
| resolveHost / Binary / CodemodeAddon | 26/17/18 | launcher |
| literal_sql | 16 | essential residual |
| update_paths | 15 | pass 3 residual |
| index_content_at | 13 | pass 4 residual |

## Mode reminder

Campaign multipass repo-sweep. Pass 7 = error-path `extract_method` on failure ladders.

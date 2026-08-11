# NEXT_PASS.md

Run ID: `2026-08-10T235424Z-baseline`
Completed: **Pass 5 — Lookup-table wave**
Next: **Pass 6 — Boolean / decompose** (optional Keep helpers) or residual table polish

## Pass 5 outcome (this session)

| Function | CC before → after | Technique | Helpers |
|---|---|---|---|
| `argvFor` | 22 → 11 | lookup_table | `ARGV_SPEC`, `argStr` (3) |
| `searchToolCall` | 17 → 11 | lookup_table | `SEARCH_CALL_SPEC` |
| `literal_sql` | 18 → 16 | lookup_table | `LITERAL_SQL`, `literal_sql_template` (1) |

Touched-file ΣCC: **258 → 243 (−15)**.

Parity: cargo check core green; literal_glob + pattern_prefilter + chain_case 7; extension npm 88; argvFor 12-case differential. Artifacts: `06-transformed-code/pass5-*`, `07-parity-report-pass5.md`, `08-complexity-scorecard-pass5.md`. Mirror: `tests/fixtures/cyclomatic-reduction/pass5-lookup/`.

## Do next (Pass 6)

1. Load via `.cyclomatic-reduction/LATEST` → `2026-08-10T235424Z-baseline`.
2. **Do not re-baseline** unless scope changes.
3. Boolean / decompose batch from plan:
   - `ensureFresh` (CC~23) — named predicates + `runIndex(force)` only if still hotspot; **do not remove health varieties**
4. Optional polish: argvFor / searchToolCall still CC 11 (1 over hard 10) — further form collapse only if no behavior risk.
5. Prefer joint-allowed targeted tests.

## Residual after pass 5 (above ceiling 10)

| Function | After CC | Note |
|---|---:|---|
| argvFor | 11 | form interpreter |
| searchToolCall | 11 | form interpreter |
| literal_sql | 16 | word_mode loop essential |
| index_content_at (pass 4) | 13 | full upsert path |
| scan_line_window | 12 | line-scan loop |
| isValidHitShape | 18 | Keep / name only |
| resolveHost | 26 | launcher residual |
| resolveCodemodeAddon | 18 | launcher residual |
| resolveBinary | 17 | launcher residual |
| update_paths | 15 | residual match arms |
| summarizeCodemode | 15 | not yet claimed |
| literal_trigram | 12 | not yet claimed |

## Explicitly out of pass 6

- Bench helpers (`run_bench_suite`, …)
- essential_domain Keep rows (fusion, IVF parse, pattern DSL, …)
- Pass 7 error-path extracts (`parseEnvelope`) unless ensureFresh finishes early

## Mode reminder

Campaign multipass repo-sweep. Pass 6 = combine_predicates / decompose_conditional on optional Keep helpers.

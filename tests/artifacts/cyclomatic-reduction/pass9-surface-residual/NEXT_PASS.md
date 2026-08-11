# NEXT_PASS.md

Run ID: `2026-08-11Tpass9-surface`  
Completed: **Pass 9 — Module residual (surface crates + launcher measure)**  
Next: **Pass 10 — Full-scope Bill re-measure** (crates + packages surface) then residual queue

## Pass 9 outcome (this session)

| Function | CC before → after | Technique | Helpers |
|---|---|---|---|
| `run_bench_suite` | 29 → **24** | shared collapse | `enforce_bench_ratchet` (3), `print_ast_grep_human` (7) |
| `run_bench` | 15 → **9** | same | (shared) |
| `run_search` | 13 → **10** | consolidate predicates | `uses_semantic_channel` (2) |

Touched-file ΣCC: **151 → 149 (−2)**. CLI package total_cc **632 → 630 (−2)**.

Refused pure extracts that raised ΣCC:

1. `run_process` error-path extract → `lib.rs` +3  
2. launcher `assertHostManifestMatches` + addon helpers → `index.js` +6  
3. `measure_suite_case` / `print_suite_human` → first-trial `bench.rs` +3  

Parity: CLI machine_contracts (bench), cli_smoke, lib unit tests green; launcher node tests 13/13 (no product change).  
Mirror: `tests/artifacts/cyclomatic-reduction/pass9-surface-residual/`.

## Explicit Keep / Refuse notes (surface)

- `classify_native`, `cached_pattern_signatures`, `apply_kind_rule` — **Ashby Keep** (lang domain).  
- launcher `resolveHost`/`resolveBinary`/`resolveCodemodeAddon` residuals — **Refuse pure extract** this wave (bill +6); pass-3 guards already applied.  
- `run_process` — **Refuse pure extract** (+3).  
- LSP — no CC>10.  
- MCP `run_stdio` / `scan_line_window` — Defer (read_node already extracted pass 4).

## Residual checks after pass 9 (named)

| Function | CC | Package | Resolve |
|---|---:|---|---|
| `resolveHost` | 26 | launcher | Defer (extract dumps) |
| `run_bench_suite` | 24 | cli | Defer residual case loop |
| `classify_native` | 20 | lang | Keep |
| `cached_pattern_signatures` | 19 | lang | Keep |
| `resolveCodemodeAddon` | 18 | launcher | Defer |
| `resolveBinary` | 17 | launcher | Defer |
| `run_process` / `run_bench_batch` | 16 | cli | Defer / Refuse extract |
| `apply_kind_rule` | 15 | lang | Keep-leaning |
| `run_chain` / `run_watch` | 14 | cli | Defer |
| Core residuals | see pass 8 | core | Keep / Defer as logged |

## Do next (Pass 10)

1. Load `.cyclomatic-reduction/LATEST` → `2026-08-11Tpass9-surface` (and prior baseline run books as needed).  
2. **Full-scope re-measure** (Bill): `crates/` + `packages/pi/launcher/src` + `packages/pi/extension/src` (same campaign scope as pass-1 ledger). Emit package totals + hotspot list CC>10.  
3. Compare to baseline ΣCC **6022** / pass-8 core 2886 / pass-9 surface deltas — honest ledger, no invented speedups.  
4. Only schedule cuts that are **bill-neutral/negative** (shared collapse, dead-branch proof, consolidate). Pure extract without funding → Refuse.  
5. Optional funded targets if Σ allows: `run_bench_suite` residual, `run_chain`, extension Keep residuals only with auth, core only if shared collapse appears.

## Mode reminder

Campaign multipass repo-sweep. Pass 10 is the **full-scope re-measure** checkpoint before further residual waves.

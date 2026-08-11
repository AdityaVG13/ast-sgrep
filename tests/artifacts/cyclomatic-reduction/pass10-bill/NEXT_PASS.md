# NEXT_PASS.md

Run ID: `2026-08-11Tpass10-bill`  
Completed: **Pass 10 — Full-scope ΣCC Bill re-measure + displacement check**  
Next: **Pass 11 — Parity re-check + residual beads / work-queue polish**

## Pass 10 outcome

| Metric | Baseline | Now | Δ |
|---|---:|---:|---:|
| ΣCC | 6022 | **5994** | **−28** |
| Max CC | 31 | **26** | −5 |
| Hotspots CC>10 | 91 | **83** | −8 |
| Functions | 1927 | 1953 | +26 |

- **Displacement check: PASS**
- **Product files changed this pass: ZERO**
- Mirror: `tests/artifacts/cyclomatic-reduction/pass10-bill/`
- Canonical metrics: `bill-summary.json`

## Pass 11 goals (parity + beads)

1. **Campaign parity re-check** (not transform):
   - Targeted floors that cover wave-touched packages:  
     `ast-sgrep-cli` machine_contracts/smoke/lib; `ast-sgrep-core` focused tests for literal/IVF/index if joint-allowed; extension/launcher node tests.
   - Do **not** invent whole-workspace `cargo test` unless Agents/project allows.
   - Write `07-parity-report-pass11.md` aggregating wave parity + any re-run evidence.
2. **Residual queue hygiene**:
   - Promote **at most 3** fundable Defer clusters (D1 launcher / D2 CLI surface / D3 core search-store) — either upgrade existing beads **or** keep markdown `work-queue/` (preferred if open bead flood continues).
   - Do **not** open one bead per hotspot.
3. **No large product transform wave** unless a single obvious Σ-funded shared-collapse appears during parity tooling — default still measure/queue.

## Pass 12 preview

- Final scorecard validation (`validate_cut_branches.py` if present)
- Campaign RESULT block; residual Keep ledger frozen
- Only claim `complete` if authorized scope is under ceiling or every residual is Keep/blocked_with_reason

## Explicit non-goals for pass 11

- API redesign
- Pure extract of `resolveHost` / `run_process` without new bill-negative technique
- Cutting Keep rows (`read_header`, `classify_native`, allowlists, KindRule)

## Work-queue packets (markdown; beads deferred)

See `work-queue/D1-launcher-resolve.md`, `D2-cli-surface.md`, `D3-core-search-store.md` under this run.  
Open tracker already has **50+** beads (gauntlet/perf/MCT) — **markdown work-queue preferred** over adding cyclomatic micro-beads this pass.

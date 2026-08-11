# Pass 11 intent — Differential parity hardening + scorecard + residual queue

## Mode

**No product transform.** Re-run joint-allowed targeted parity for packages touched by passes 3–9; harden residual D1–D3 packets; finalize scorecard narrative for pass-12 convergence.

## In scope

1. Level-1 compile: `cargo check -p ast-sgrep-core -p ast-sgrep-cli -p ast-sgrep-mcp`
2. Extension suite: `packages/pi/extension` `npm test` (passes 4–7 surface)
3. Launcher floor: node tests covering resolve\* / PATH / native packages (pass 3 / 9)
4. CLI targeted: machine_contracts + cli_smoke + lib (pass 5 / 9)
5. Core targeted: parity, e2e_smoke, regex_budget, semantic_ivf_roundtrip, search_correctness_epics, code_prose_fields (pass 8)
6. Residual work-queue D1–D3 full enough for independent agent
7. Scorecard + RESULT + NEXT_PASS → pass 12 absolute convergence (expect ZERO product change)

## Out of scope

- Whole-workspace `cargo test --workspace`
- New ΣCC cuts unless a campaign-caused test failure forces a minimal fix
- Flooding beads (markdown work-queue only)
- Cutting Keep ledger rows

# Pass 14 — Integration full targeted

**Date:** 2026-08-07  
**Scope:** Re-run full mid-loop suite (same as Pass 8) plus embed `dim_probe` with `cloud` feature and `ranking_oracle`.

Isolated target dir: `CARGO_TARGET_DIR=target-pass14` (avoids concurrent workspace cargo lock contention).

Full log: `tests/artifacts/bug-hunt/PASS14_test_run.log`

## Commands + results

| Command | Result |
|---|---|
| `cargo test -p ast-sgrep-core --lib confidence` | **ok** 6 passed |
| `cargo test -p ast-sgrep-core --lib pass3_deep_core` | **ok** 2 passed |
| `cargo test -p ast-sgrep-core --lib restore_synchronous` | **ok** 6 passed |
| `cargo test -p ast-sgrep-cli --lib worker_nonce` | **ok** 1 passed |
| `cargo test -p ast-sgrep-cli --lib machine::tests` | **ok** 6 passed |
| `cargo test -p ast-sgrep-cli --test machine_contracts` | **ok** 20 passed |
| `cargo test -p ast-sgrep-mcp --lib cache_tests` | **ok** 2 passed |
| `cargo test -p ast-sgrep-lsp --lib` | **ok** 6 passed |
| `cargo test -p ast-sgrep-core --test durability_epics` | **ok** 19 passed |
| `cargo test -p ast-sgrep-embed --lib dim_probe --features cloud` | **ok** 5 passed |
| `cargo test -p ast-sgrep-core --test ranking_oracle` | **ok** 1 passed (fixture suite) |

**Totals for this gate:** 80 passed, 0 failed, 0 ignored.

Notes on filter counts vs Pass 8: several `--lib <filter>` suites match more unit tests than in Pass 8 (confidence 6 vs 4, restore_synchronous 6 vs 4, machine::tests 6 vs 4, lsp lib 6 vs 4) because the filter substring matches additional tests added since; all still green.

## Regressions

**None.** No compile breaks, no failing tests. No product code changes in this pass.

## Beads

None filed (no new confirmed bugs).

## Verdict

**ZERO-CHANGE** for product. Docs-only artifact: this file + `PASS14_test_run.log`.

No commit (per mission).

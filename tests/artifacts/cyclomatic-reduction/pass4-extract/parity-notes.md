# Pass 4 — parity notes

## Commands + results

```text
cargo check -p ast-sgrep-core -p ast-sgrep-cli -p ast-sgrep-mcp
→ Finished dev profile ok

cargo test -p ast-sgrep-core --test parity --test e2e_smoke
→ e2e_smoke: 5 passed, 1 ignored
→ parity: 3 passed

cargo test -p ast-sgrep-core --test semantic_v1_rewrite
→ 2 passed

cargo test -p ast-sgrep-cli --lib
→ 10 passed

cargo test -p ast-sgrep-mcp --lib
→ 3 passed

cd packages/pi/extension && npm test
→ 88 passed, 0 failed
```

## Pre-existing

`cargo test -p ast-sgrep-core --lib` still fails compile on unrelated `SearchResponse` field gaps in `search/mod.rs` (same as pass 3). Not introduced by this wave.

## Bill

Combined touched ΣCC **716 → 712 (−4)**. Metric-gaming auditor: pass (named extracts; delete path true collapse).

Full write-up: `.cyclomatic-reduction/runs/2026-08-10T235424Z-baseline/07-parity-report-pass4.md`

# Benchmarks

Published **numbers and write-ups** only. No harness scripts, gold corpora, or
large worktrees live here. Consequently, every numeric row under the results
directory is explicitly historical and unreproducible from this tree. External
artifacts, when retained, are stored by the [Speed benchmark workflow](https://github.com/AdityaVG13/ast-sgrep/actions/workflows/speed.yml);
no retained artifact has been identified for the currently published rows.

```text
benchmarks/
  README.md                 ← you are here
  results/                  ← scored comparisons and baselines
    head-to-head.md
    speed.md
    bakeoff.md
    losses.md
    baselines.md
  studies/                  ← focused analyses
    intent-confusion.md
    prevented-read.md
```

## Start here

| Doc | What it answers |
|-----|-----------------|
| [results/head-to-head.md](results/head-to-head.md) | Canonical cross-tool gate table |
| [results/speed.md](results/speed.md) | Lexical / structural latency notes |
| [results/bakeoff.md](results/bakeoff.md) | Offline bake-off narrative and scores |
| [results/losses.md](results/losses.md) | Where we lose (published deliberately) |
| [results/baselines.md](results/baselines.md) | Pinned floors and provenance |

## Studies

| Doc | Topic |
|------|--------|
| [studies/intent-confusion.md](studies/intent-confusion.md) | Intent / routing observations |
| [studies/prevented-read.md](studies/prevented-read.md) | Capsule / prevented-read notes |

## Product docs

Methodology for readers: [docs/benchmarks.md](../docs/benchmarks.md).

## Correctness smoke (not benchmarks)

```bash
cargo test -p ast-sgrep-core --test parity -j1 -- --test-threads=1
```

## Latency error budgets

Published latency budgets are hard sample thresholds, separate from the measured
tables in `results/`. The cold self-index budget is **285 ms p95**: the prior
258.4 ms p95 plus a 10% same-host variance allowance, rounded up. A baseline
above its threshold must not be published as a passing budget.

`scripts/check-error-budget.py` computes the hard-threshold exceedance rate
directly from hyperfine `times`; for a 95% SLO, `burn_rate = error_rate / 0.05`.
The p95 threshold and burn-rate checks are both gates. A p95 comparison alone is
not an empirical error rate. Example:

```bash
python3 scripts/check-error-budget.py hyperfine_index_self.json --label cold-index-self --threshold-ms 285 --slo 0.95 --baseline-p95-ms 258.4
```

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

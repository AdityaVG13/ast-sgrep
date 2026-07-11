# Performance cost inventory

Historical notes from local profiling of hot paths (lexical / structural /
semantic). Detailed regenerate scripts are **not** shipped in this repository;
published narrative numbers live under [`benchmarks/`](../benchmarks/).

## Where to look

| Document | Focus |
|----------|--------|
| [benchmarks/results/speed.md](../benchmarks/results/speed.md) | Wall-clock and head-to-head timing notes |
| [benchmarks/results/baselines.md](../benchmarks/results/baselines.md) | Pinned floors |
| [benchmarks/results/head-to-head.md](../benchmarks/results/head-to-head.md) | Cross-tool summary |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Index and search pipeline (cost drivers) |

## Cost drivers (summary)

Indexing is dominated by parse/extract, SQLite line/FTS writes, and optional
embedding. Search is dominated by pass selection (literal/symbol/embed), fusion,
and optional ANN probe. See the architecture doc for the current pipeline.

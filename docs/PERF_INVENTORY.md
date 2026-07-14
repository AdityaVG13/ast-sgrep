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

## Multi-term symbol candidate scoring

`best_symbol_score` and `coverage_symbol_score` normalize each candidate symbol
once for the complete term batch. Lowercase ASCII identifiers borrow their existing text;
mixed-case and non-ASCII identifiers retain the previous Unicode lowercase conversion. This
removes one `String` allocation per extra query term for mixed-case candidates and all
normalization allocations for the common lowercase-ASCII case, without changing score
order or values.

Measure the isolated multi-term symbol/caller/definition scoring path with:

```sh
cargo bench -p ast-sgrep-core --bench search -- rank_symbol_candidates_multi_term
```

The expected improvement is bounded to queries that score symbol candidates, especially
queries with multiple terms or many lowercase identifiers. Single-term mixed-case or
Unicode symbols still require one lowercase allocation, while SQLite/FTS- and
embedding-dominated queries should not materially move.

A same-machine Criterion comparison on 2026-07-14 used the command above for both
the checked-out HEAD and this change. The median estimate moved from 1.0042 us to
638.88 ns, a 1.57x speedup (36.4% lower latency). This isolated microbenchmark is
evidence for the normalization hot path, not a claim about end-to-end indexed search.

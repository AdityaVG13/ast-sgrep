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

## Watch-to-search latency is a multi-station path

Watch mode is a tandem pipeline, not a single search queue:

1. notification debounce and coalescing;
2. Indexer::update_paths;
3. Indexer::flush_deferred_rebuilds for Tantivy and IVF sidecars;
4. searches served at the supervisor duty-scaled capacity.

For an arrival rate lambda, record each station service capacity mu_i and wall-clock wait W_i. The practical end-to-end estimate is E[W_sys] approximately sum(E[W_i]); queue occupancy must also satisfy Little law L_i = lambda W_i. Report utilization as rho_i = lambda / mu_i and treat any station approaching rho_i = 1 as the bottleneck. The supervisor duty fraction reduces station 4 capacity and must be included in mu_4.

An end-to-end p99 must therefore come from a wall-clock load run that timestamps all four boundaries. A search microbenchmark, or update_paths timing alone, cannot be reported as watch-to-search p99. The metric plan is to record debounce queue depth and release time, update_paths duration, deferred-rebuild duration, search queue depth, duty fraction, and final response time under the same offered load; publish per-hop and end-to-end percentiles together.

## Do not assume nested duty limits are additive

`scripts/rustc-capped` applies an outer 80% STOP/CONT duty cycle. On Unix, an `asgrep` command also applies its own supervisor duty cycle (80% by default). If `asgrep` is intentionally run through that wrapper, effective wall-time capacity is the product, not the minimum or sum: `0.80 * 0.80 = 0.64` by default. A 50% outer limit with the default inner limit yields 40% capacity. Queue and latency estimates must use that product.

The production policy is to invoke `asgrep` directly. Reserve `rustc-capped` for compiler/build payloads. A workflow that deliberately nests the two limiters must record both configured fractions, the product capacity, and full-wall latency including both STOP intervals; never report the inner `ASGREP_CPU_LIMIT_PERCENT` as effective capacity.

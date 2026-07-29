# Performance cost inventory

Historical notes on hot paths. Detailed regenerate scripts are **not** shipped
here; published narrative numbers live under [`benchmarks/`](../benchmarks/).

## Where to look

| Document | Focus |
|----------|--------|
| [benchmarks/README.md](../benchmarks/README.md) | Index + error-budget gates |
| [benchmarks/results/speed.md](../benchmarks/results/speed.md) | Wall-clock notes |
| [benchmarks/results/baselines.md](../benchmarks/results/baselines.md) | Pinned floors |
| [ARCHITECTURE.md](ARCHITECTURE.md) | Pipeline cost drivers |

## Cost drivers (summary)

Indexing is dominated by parse/extract, SQLite line/FTS writes, and optional
embedding. Search is dominated by pass selection (literal/symbol/embed), fusion,
and optional ANN probe.

Multi-term symbol scoring (`best_symbol_score` / `coverage_symbol_score`)
normalizes each candidate once per term batch. Isolated Criterion probe:

```sh
cargo bench -p ast-sgrep-core --bench search -- rank_symbol_candidates_multi_term
```

## Measurement caveats

- Watch-to-search is a multi-station path (debounce → `update_paths` → deferred
  rebuild → search). Report per-hop and end-to-end percentiles together; a
  search microbench is not watch p99.
- Nested duty cycles multiply (`rustc-capped` × supervisor). Prefer invoking
  `asgrep` directly; if nested, record the product capacity.
- Latency samples collected only during CONT windows are CONT-conditional — do
  not publish them as wall-clock p50/p99 without disclosure.

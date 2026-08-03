# Benchmarks

Recorded speed and quality notes for ast-sgrep. Figures are **historical
measurements**, not portable SLAs. Prefer the ordered reading list below.

## Reading order

1. [head-to-head.md](../benchmarks/results/head-to-head.md) — summary gate table  
2. [speed.md](../benchmarks/results/speed.md) — latency notes  
3. [bakeoff.md](../benchmarks/results/bakeoff.md) — cross-tool bake-off  
4. [losses.md](../benchmarks/results/losses.md) — published regressions  
5. [baselines.md](../benchmarks/results/baselines.md) — pinned floors / provenance  

Studies (optional depth): [intent-confusion](../benchmarks/studies/intent-confusion.md),
[prevented-read](../benchmarks/studies/prevented-read.md).

Folder index: [benchmarks/README.md](../benchmarks/README.md).

The canonical self-corpus quality snapshot and its reproduction command or status live in the [18-query retrieval-quality section of baselines.md](../benchmarks/results/baselines.md#retrieval-quality--self-corpus-18-gold-queries). Do not copy quality figures without that source link.

## Honest caveats

- Hardware, corpus, warm/cold cache, and flags all move the numbers.
- On some foreign corpora the default offline embedder adds little over lexical
  + AST; hybrid and `--no-embed` can score the same.
- Losses are published, not suppressed.

## Local product checks

```bash
cargo test -p ast-sgrep-core --test parity -j1 -- --test-threads=1
cargo build --release -p ast-sgrep-cli -j1
./target/release/asgrep bench . --query process_request --iterations 1
```

## Bench history ratchet + cv_pct

`asgrep bench` / `asgrep bench --suite` JSON output includes `cv_pct` (sample
coefficient of variation over the timed iterations) and writes
`.bench-history.json` (override with `ASGREP_BENCH_HISTORY_PATH`; disable with
`ASGREP_BENCH_HISTORY=0`).

Optional keep-gate: set `ASGREP_BENCH_RATCHET=1` to fail when the current mean
exceeds the prior history mean by more than 50% (`ratchet_pct`). Pass-over-pass
thresholds are intentionally coarse — this is a regression tripwire, not a
microbenchmark SLA.

`speedup_vs_ast_grep` is only emitted under `ast_grep_comparison` for
`pattern:` queries when the ast-grep binary is present; hybrid/token comparisons
are skipped with an explicit `skipped_reason`.

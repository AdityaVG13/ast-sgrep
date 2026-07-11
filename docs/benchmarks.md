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

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

## Bench history keep-gate

Committed SSoT: [`.bench-history/`](../.bench-history/README.md) (`*.latest.json` +
`thresholds.json`). Local `.bench-history.json` is gitignored scratch, not truth.

Keep rules (default-on; disable with `ASGREP_BENCH_RATCHET=0`):

- Primary mean regression **> 3%** vs committed prior → fail
- Suite geomean regression **> 5%** → fail
- `cv_pct > 5` → **quarantine** (ineligible, not a silent keep)
- Missing / placeholder prior → **establish baseline**, not a win keep
- Every decision records `host`, `git_sha`, `profile`
- Batch (`--queries-file`) emits `cv_pct` + history and uses the same rules
- Claiming a **win** keep also requires a HotPath / profile sample (checklist)

`--max-average-ms` in CI is a **host-labeled smoke ceiling**, not the keep
oracle. Competitor latency (ast-grep CLI, ripgrep, UNREPRODUCIBLE
`benchmarks/results/*` rows) is **not** keep and **not** correctness.

`speedup_vs_ast_grep` is only emitted under `ast_grep_comparison` for
`pattern:` queries when the ast-grep binary is present; hybrid/token comparisons
are skipped with an explicit `skipped_reason`. Do not read that field as a keep
gate.

Override history dir with `ASGREP_BENCH_HISTORY_DIR`. Copy a passing `.run.json`
to `.latest.json` only after a keep (`ASGREP_BENCH_HISTORY_COMMIT=1`).

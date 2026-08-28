# Benchmarks

Published fingerprints in `results/` are a **mixed ledger**. Read the
**status tag on each section**, not a file-level banner.

| Tag | Meaning |
|-----|---------|
| `canonical` | Fingerprint others must cite. Regeneration may still be `UNREPRODUCIBLE`. |
| `historical` | Published record. Not a live SLA. |
| `UNREPRODUCIBLE` | This tree cannot regenerate the row (missing harness, gold, corpus, or artifact). |
| `reproducible-in-tree` | Exact command + pins exist here. |

A file-level UNREPRODUCIBLE banner does **not** apply to `reproducible-in-tree`
sections. A reproducible latency section does **not** make historical MRR rows
live. Do not invent replacement MRR/latency wins.

Large external corpora are not vendored. `fuzz/` and `conformance/` are not
shipped; clone-required helper scripts live in `scripts/`.

```text
benchmarks/
  README.md                 ← you are here
  run_eval.sh               ← quality eval pack (raw output is gitignored)
  fixtures/                 ← small labeled corpora
  gold/                     ← gold queries for eval
  results/                  ← scored comparisons
    speed.md                ← CLI latency (live + archive)
    head-to-head.md         ← competitor table
    bakeoff.md              ← quality bake-off (UNREPRODUCIBLE)
    losses.md               ← published retrieval losses
    baselines.md            ← quality fingerprints
  studies/                  ← focused analyses
```

## Start here

| Doc | What it answers |
|-----|-----------------|
| [results/speed.md](results/speed.md) | Current self-corpus CLI latency |
| [results/head-to-head.md](results/head-to-head.md) | Competitor table + historical GATE |
| [results/bakeoff.md](results/bakeoff.md) | Offline quality bake-off |
| [results/losses.md](results/losses.md) | Where we lose (published deliberately) |
| [results/baselines.md](results/baselines.md) | Pinned quality fingerprints |

## Studies

| Doc | Topic |
|------|--------|
| [studies/intent-confusion.md](studies/intent-confusion.md) | Intent / routing observations |
| [studies/prevented-read.md](studies/prevented-read.md) | Excerpt vs whole-file bytes |

## Reproduce

CLI latency (self corpus vs ripgrep / ast-grep): see the command block in
[results/speed.md](results/speed.md).

In-process identity suites (not the CLI competitor table):

```bash
ASGREP_BENCH_RATCHET=0 cargo run --locked --release -p ast-sgrep-cli --bin asgrep -- \
  --json bench tests/fixtures/sample --suite default --fixture sample --iterations 20

ASGREP_BENCH_RATCHET=0 cargo run --locked --release -p ast-sgrep-cli --bin asgrep -- \
  --json bench . --suite self --fixture self --iterations 10
```

Quality pack (writes gitignored `results/raw/`):

```bash
./benchmarks/run_eval.sh
```

## Latency notes

Published CLI numbers are sample measurements, not SLOs. The old 285 ms
cold-index budget was set against a 110-file tree (SHA unrecorded) and is
**not** a passing claim on the current repository.

| Surface | Status | Notes |
|---------|--------|-------|
| cold self-index CLI | `reproducible-in-tree` | 4.58 s p95 on 2026-08-28 (`2285ce29`) |
| warm literal / pattern / semantic CLI | `reproducible-in-tree` | see [speed.md](results/speed.md) |
| in-process Searcher | `reproducible-in-tree` | `asgrep bench`; sub-millisecond warm path, high first-query CV |

## ANN quality error budget

Adaptive IVF has a **0.99 recall@10 SLO** against the same index queried
with all clusters (`probes=all`):

```bash
cargo test -p ast-sgrep-core --test semantic_ivf_roundtrip \
  adaptive_ivf_recall_at_10_stays_within_quality_error_budget -- --nocapture
```

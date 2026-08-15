# Benchmarks

Published quality fingerprints in `results/` are a **mixed ledger**. Read the
**status tag on each section**, not a single file-level banner.

| Tag | Meaning |
|-----|---------|
| `canonical` | Fingerprint others must cite. Regeneration may still be `UNREPRODUCIBLE`. |
| `historical` | Published record. Not a live SLA. |
| `UNREPRODUCIBLE` | This tree cannot regenerate the row (missing harness, gold, corpus, or artifact). |
| `reproducible-in-tree` | Exact command + pins exist here (`scripts/run-benchmarks.sh`, `asgrep bench` + `.bench-history`). |

A file-level UNREPRODUCIBLE banner does **not** apply to `reproducible-in-tree`
sections. A reproducible latency section does **not** make historical MRR rows
live. Do not invent replacement MRR/latency wins.

Release-time gates run the in-tree CLI suites against the sample fixture and
the repository itself. The [Speed benchmark workflow](https://github.com/AdityaVG13/ast-sgrep/actions/workflows/speed.yml)
uploads JSON and fails on identity / hit / keep-gate regression. Large external
corpora are still not vendored.

```text
benchmarks/
  README.md                 ← you are here
  results/                  ← scored comparisons and baselines
    head-to-head.md
    speed.md
    bakeoff.md
    losses.md
    baselines.md
    FLOOR_PROMOTION_PROTOCOL.md
    fusion-scorecard.md
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
| [results/FLOOR_PROMOTION_PROTOCOL.md](results/FLOOR_PROMOTION_PROTOCOL.md) | S2 MEASURED FLOOR MATCH axes (C1 4.304 s stays until checklist + human ACK) |
| [results/fusion-scorecard.md](results/fusion-scorecard.md) | Sub-1ms in-process parts vs multi-ms CLI competitor rows (e2hc.31) |

## Studies

| Doc | Topic |
|------|--------|
| [studies/intent-confusion.md](studies/intent-confusion.md) | Intent / routing observations |
| [studies/prevented-read.md](studies/prevented-read.md) | Capsule / prevented-read notes |

## Product docs

Methodology for readers: [docs/benchmarks.md](../docs/benchmarks.md).

## Executable release gates

```bash
cargo run --locked --release -p ast-sgrep-cli --bin asgrep -- \
  --json --index-path /tmp/asgrep-speed.db \
  bench tests/fixtures/sample --suite default --fixture sample --iterations 10 \
  > speed-results.json
python3 scripts/check-bench-output.py speed-results.json --history-dir .bench-history --label suite:sample:default --smoke-max-average-ms 15

cargo run --locked --release -p ast-sgrep-cli --bin asgrep -- \
  --json --index-path /tmp/asgrep-bakeoff.db \
  bench . --suite self --fixture self --iterations 5 \
  > bakeoff-results.json
python3 scripts/check-bench-output.py bakeoff-results.json --history-dir .bench-history --label suite:self:self --smoke-max-average-ms 100
```

Both suites fail inside the CLI when hit counts, expected result identities, or
the keep-gate miss. The checker also applies committed `.bench-history` keep
rules; `--smoke-max-average-ms` is a host-labeled secondary ceiling, not the
keep oracle. Competitor latency is not keep.

## Latency error budgets

Published latency budgets are hard sample thresholds, separate from the measured
tables in `results/`. A baseline above its threshold must not be published as a
passing budget.

| Surface | Status | Corpus file-count | git SHA | Hard p95 | SLO |
|---------|--------|------------------:|---------|----------|-----|
| cold self-index CLI (archived 110-file) | `historical` | 110 | unrecorded in-tree | 285 ms | 95% |
| cold self-index CLI (current self) | `historical` (breached) | 1,107 | `cea904a` | 285 ms **must not be quoted as passing** | 95% |
| literal CLI fixture | `reproducible-in-tree` (policy + keep-gate) | sample fixture | see `.bench-history` | 15 ms | 95% |
| semantic CLI fixture | `reproducible-in-tree` (policy + keep-gate) | sample fixture | see `.bench-history` | 15 ms | 95% |
| natural-language CLI fixture | `reproducible-in-tree` (policy + keep-gate) | sample fixture | see `.bench-history` | 15 ms | 95% |

### Archived: 110-file cold-index budget

The 285 ms p95 (prior 258.4 ms + 10% same-host variance, rounded up) was set
against a **110-file** self corpus. SHA for that 110-file tree was **not
recorded**. On 2026-08-05 the self corpus was **1,107 tracked files** at
`cea904a`; measured cold index 906–992 ms p95 **breaches** 285 ms. Do not
delete this miss. Do not invent a new passing cold-index number here
(re-baseline is a later measurement, not this honesty pass).

The historical 10 ms self-repo Searcher-query target does not apply to CLI
startup fixtures. Each CLI surface is gated independently; handoff JSON must
retain both `p95_ms` and `burn_rate` rather than collapsing them.

`scripts/check-error-budget.py` computes the hard-threshold exceedance rate
directly from hyperfine `times`; for a 95% SLO, `burn_rate = error_rate / 0.05`.
The p95 threshold and burn-rate checks are both gates. A p95 comparison alone is
not an empirical error rate. Same-host variance is a separate regression gate:
provide `--prior-p95-ms`, `--fingerprint`, and `--prior-fingerprint` to compare
the current p95 with a prior run. A missing or different fingerprint makes drift
non-comparable. Passing the default 10% drift envelope never changes the hard
threshold, exceedance rate, burn rate, or `claim_within_slo`.

**Measured status (2026-08-05):** cold self-index measured 906–992 ms p95 on
the current 1,107-file self corpus (baseline `cea904a` and pr26 `137863f`),
breaching the 285 ms budget set against the historical 110-file corpus. pr21
(`5de7eb0`) originally measured 88.5 s p95 / 107 MiB (eager per-child-node
semantic chunks); the child-chunk cap fix (`0ba34da`, 32 → 2 per parent)
brought it to **2.1 s p95 / 27 MiB** with semantic query latency dropping
42 → 16 ms. Re-baseline the cold-index budget for the current corpus size;
`scripts/run-benchmarks.sh` reproduces the rows.

Example:

```bash
python3 scripts/check-error-budget.py hyperfine_index_self.json --label cold-index-self --threshold-ms 285 --slo 0.95 --baseline-p95-ms 258.4
```

## ANN quality error budget

Adaptive IVF has a **0.99 recall@10 SLO** against the same index queried
with all clusters (`probes=all`). Miss rate is `1 - recall`; quality burn rate
is `miss_rate / 0.01`. The narrowly filtered CI regression measures 64
deterministic queries and fails when burn rate exceeds 1:

```bash
cargo test -p ast-sgrep-core --test semantic_ivf_roundtrip adaptive_ivf_recall_at_10_stays_within_quality_error_budget -- --nocapture
```

## Semantic IVF mmap open budget

The medium fixture contains 10,000 vectors. Dedicated release-perf runs enable the 1 ms warm-open p99 gate explicitly; ordinary correctness runs avoid timing failures on contended hosts while still requiring mapped vectors and byte accounting.

```bash
ASGREP_PERF_ASSERTS=1 cargo test --locked --profile release-perf \
  -p ast-sgrep-core --test semantic_ivf_roundtrip \
  medium_mapped_sidecar_reports_open_p99 -- \
  --exact --nocapture --test-threads=1
```

Cold, fresh-inode, and warm definitions plus the isolated probe are in [`docs/validation/semantic-ivf-mmap.md`](../docs/validation/semantic-ivf-mmap.md).

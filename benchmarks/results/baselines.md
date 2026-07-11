# Baselines

> **Published record** of measured results. No runnable harnesses ship in this tree.

Single source of truth for every MRR / recall / latency claim in this
repository. Any number quoted in docs, commit messages, or bead close reasons
must trace back to a row here or carry its own reproduce command. Scores were
produced by the harness, twice, on the machine below — no hand-edited figures.

## Provenance

| field | value |
|-------|-------|
| date | 2026-07-10 |
| commit | `d3eab74c7f3725bae4b1fab24ea94fe3b58d3601` (d3eab74) |
| machine | Apple M5 Max, 18 cores (arm64), 48 GiB, macOS 26.5, APFS SSD |
| build | `cargo build --profile release-perf -p ast-sgrep-cli` |
| rustc | 1.96.0 |
| python | 3.14.6 |
| competitors | ripgrep 15.1.0, ast-grep 0.44.1, semgrep 1.168.0 |
| timing | hyperfine 1.20.0 |

Corpora are pinned by tag + SHA in `corpora.lock`:

| corpus | ref | SHA |
|--------|-----|-----|
| self (this repo) | d3eab74 | `d3eab74c7f3725bae4b1fab24ea94fe3b58d3601` |
| ripgrep | 14.1.1 | `4649aa9700619f94cf9c66876e9549d83420e16c` |
| flask | 3.0.3 | `c12a5d874c5a014495eb2db8a73f40037bc813ac` |
| tokio | tokio-1.38.0 | `14c17fc09656a30230177b600bacceb9db33e942` |
| express | 4.19.2 | `04bc62787be974874bc1467b23606c36bc9779ba` |

## Retrieval quality — self corpus (18 gold queries)

Reproduce:

```bash
cargo build --profile release-perf -p ast-sgrep-cli
cd benchmarks && ```

| tool | MRR | Recall@k | nDCG@k |
|------|----:|---------:|-------:|
| asgrep hybrid | **0.712** | **0.889** | **0.751** |
| asgrep --no-embed | 0.712 | 0.889 | 0.751 |
| asgrep semantic-only | 0.294 | 0.611 | 0.364 |
| ripgrep (file order) | 0.061 | 0.167 | 0.086 |

Note: 0.712 is lower than the previously published 0.746; the drop landed
with the reviewed correctness fixes in `29129bd` (ranking changed for one
query). The `retrieval_gold.rs` CI gate (MRR >= 0.70) still passes. This
table supersedes the old figure.

## Retrieval quality — foreign-corpus bake-off (k=10)

Reproduce:

```bash
cd benchmarks
```

### ripgrep 14.1.1 (Rust, 14 queries)

| tool | MRR | Recall@k | nDCG@k | wall ms |
|------|----:|---------:|-------:|--------:|
| asgrep hybrid | 0.290 | 0.464 | 0.330 | 28 |
| asgrep --no-embed | 0.290 | 0.464 | 0.330 | 19 |
| ripgrep (file order) | 0.000 | 0.000 | 0.000 | 11 |
| ast-grep structural | 0.143 | 0.214 | 0.162 | 31 |
| **semgrep (reference to beat)** | **0.536** | 0.571 | 0.545 | 1235 |

### flask 3.0.3 (Python, 15 queries)

| tool | MRR | Recall@k | nDCG@k | wall ms |
|------|----:|---------:|-------:|--------:|
| asgrep hybrid | 0.161 | 0.533 | 0.246 | 15 |
| asgrep --no-embed | 0.161 | 0.533 | 0.246 | 13 |
| ripgrep (file order) | 0.162 | 0.600 | 0.259 | 10 |
| ast-grep structural | 1.000 | 1.000 | 1.000 | 16 |
| semgrep | 0.033 | 0.067 | 0.042 | 1254 |

The semgrep 0.536 MRR on the ripgrep corpus is the standing reference for the
`ast-sgrep-6hk` gate (foreign-corpus MRR >= 0.60, beating semgrep honestly).

## Speed — cold index and hybrid NL query latency

Reproduce (hyperfine; index: `--warmup 1 --min-runs 3` with the index dir
removed in `--prepare`; query: `--warmup 3 --min-runs 20` against a warm
index):

```bash
cargo build --profile release-perf -p ast-sgrep-cli
cd benchmarks && # cold index
hyperfine --warmup 1 --min-runs 3 --prepare 'rm -rf /tmp/bl.db' \
  '../target/release-perf/asgrep --index-path /tmp/bl.db index <root>'
# hybrid NL query (warm index)
hyperfine --warmup 3 --min-runs 20 \
  "../target/release-perf/asgrep --index-path /tmp/bl.db --json '<query>' <root>"
```

Queries: self = "where is hybrid ranking fused"; ripgrep = "where does
ripgrep apply gitignore rules"; flask = "where does flask dispatch HTTP
requests".

| corpus | cold index mean | NL query p50 | NL query p95 |
|--------|----------------:|-------------:|-------------:|
| self | 416 ms | 13.4 ms | 14.8 ms |
| ripgrep 14.1.1 | 3.91 s | 29.5 ms | 35.1 ms |
| flask 3.0.3 | 335 ms | 13.3 ms | 14.4 ms |

Cold-index figures include hashed-embedding generation (the default `index`
path). They are larger than the older `run-scale.sh` table in
`docs/benchmarks.md`, which indexed with different roots and machine state;
this table is the pinned reference going forward.

## Watch mode -- per-save incremental index work

Reproduce (synthetic 120-file project, 60 single-file saves, timings parsed
from the watcher's own update lines; includes the kill-9 recovery check):

```bash
cd benchmarks
python3 watch-bench.py --bin ../target/release-perf/asgrep --saves 60
python3 watch-bench.py --bin ../target/release-perf/asgrep --saves 60 --no-embed
```

| config | median | p95 |
|--------|-------:|----:|
| hashed embed (default) | 0.837 ms | 1.062 ms |
| --no-embed | 0.438 ms | 1.156 ms |

Measured 2026-07-10 at the ast-sgrep-48p commit. Sidecar rebuilds (tantivy,
semantic IVF) are deferred out of the save path and flushed after a quiet
period. SIGKILL mid-burst leaves a recoverable index (WAL + per-file
transactions): `PRAGMA integrity_check` ok, queries succeed.

## Noise bounds (second run)

Every suite was run twice back-to-back on the same build:

- **asgrep MRR / Recall@k / nDCG@k: identical to three decimals across runs**
  (ranking is deterministic). Any diff > 0.001 on an unchanged corpus and
  commit is a regression, not noise.
- **semgrep MRR: identical across runs** (0.536 / 0.033).
- **ripgrep and ast-grep file-order rows jitter** because both walk files with
  parallel, nondeterministic traversal (self ripgrep MRR 0.061 vs 0.047;
  ripgrep-corpus ast-grep MRR 0.143 vs 0.179). Treat those two rows as
  order-of-magnitude only.
- **Wall-clock timings: informational, +-30% run-to-run** on a busy machine;
  hyperfine p50 figures are stable to ~10%.

## Rules

1. No number may be quoted without a reproduce command from this file.
2. Rebaselining requires two consecutive runs within the noise bounds above
   and a commit that updates this file and `results.json` together.
3. `eval-bakeoff.py` stamps `results.json` with the live git commit and date;
   never hand-edit `results.json` scores.

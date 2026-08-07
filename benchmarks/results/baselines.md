# Baselines

> **Reproducibility status:** Every numeric row in this report is a historical
> published value and is **unreproducible from this source tree**: the generating
> harnesses, raw corpora, and raw result artifacts are absent. The external
> artifact location is the [Speed benchmark workflow](https://github.com/AdityaVG13/ast-sgrep/actions/workflows/speed.yml).
> No retained artifact is identified there for these historical runs, so this
> link is a storage location, not evidence that a row can currently be regenerated.

> **Published record** of measured results. No runnable harnesses ship in this tree.

Single source of truth for every MRR / recall / latency claim in this
repository. Any number quoted in docs, commit messages, or bead close reasons
must trace back to a row here or carry its own reproduce command. Scores were
produced by the harness, twice, on the machine below — no hand-edited figures.

## Canonical fingerprint rows

One versioned fingerprint per (corpus × metric × config). Other publications
must cite these rows; they must not introduce a second “canonical” value.

| fingerprint id | corpus | config | metric | value | status |
|----------------|--------|--------|--------|------:|--------|
| `self-hybrid-d3eab74` | self @ d3eab74 (18 gold) | default hybrid / `--no-embed` same | MRR | **0.712** | UNREPRODUCIBLE (gold harness absent) |
| `self-hybrid-d3eab74` | self @ d3eab74 (18 gold) | default hybrid | Recall@k | **0.889** | UNREPRODUCIBLE |
| `self-hybrid-d3eab74` | self @ d3eab74 (18 gold) | default hybrid | nDCG@k | **0.751** | UNREPRODUCIBLE |
| `rg-hybrid-default-d3eab74` | ripgrep 14.1.1 (14 gold) | default hybrid (hashed/local embed path as shipped) | MRR | **0.290** | UNREPRODUCIBLE — **canonical** for default hybrid |
| `rg-neural-rerank-d3eab74` | ripgrep 14.1.1 (14 gold) | `neural-embed` + cross-encoder rerank (`ASGREP_NEURAL_EMBED=1 ASGREP_RERANK=1`, see losses.md) | MRR | **0.605** | UNREPRODUCIBLE — different config; **not** interchangeable with 0.290 |
| `self-hist-pre-29129bd` | self (historical unlabeled gold) | historical hybrid | MRR ≈ 0.75 / Recall ≈ 0.94 | — | **SUPERSEDED** by `self-hybrid-d3eab74` (also formerly cited as 0.746); do not quote as current |

**Deprecations:** Do not cite 0.290 and 0.605 as competing “the” ripgrep MRR.
They are two fingerprint rows. Do not cite dual ~0.75 / 0.746 self-corpus
figures alongside 0.712 as current.

## Reproducible rows (d2dv)

These rows regenerate from a clean checkout with **one command and no network
access**, against a gold fixture and harness that ship in this tree:

```bash
./benchmarks/run_eval.sh
```

Raw artifacts are checked in under `benchmarks/results/raw/`, so a later run can
be diffed against the recorded one rather than compared to a summary.

Determinism is a property of the harness, not an aspiration: two consecutive
runs reproduce every retrieval, A/B, and token figure exactly. The token step
builds its own pinned index because the native and agent envelopes embed the
snapshot generation, and reusing an index whose generation keeps incrementing
would drift the byte totals by a digit at a time.

| fingerprint id | corpus | config | metric | value | status |
|----------------|--------|--------|--------|------:|--------|
| `self-gold12-reproducible` | self @ `benchmarks/gold/self.json` (12 gold queries) | default hybrid | MRR | **0.676** | REPRODUCIBLE |
| `self-gold12-reproducible` | self (12 gold queries) | default hybrid | nDCG | **0.751** | REPRODUCIBLE |
| `self-gold12-reproducible` | self (12 gold queries) | default hybrid | Recall@1 | **0.458** | REPRODUCIBLE |
| `self-gold12-reproducible` | self (12 gold queries) | default hybrid | Recall@5 | **0.792** | REPRODUCIBLE |
| `self-gold12-reproducible` | self (12 gold queries) | default hybrid | Recall@20 | **1.000** | REPRODUCIBLE |
| `self-tokens-gold12` | self (12 gold queries, `--limit 10`) | compact vs agent-capsule | emitted bytes | **11,428 vs 44,622 (-74.4%)** | REPRODUCIBLE |
| `self-tokens-gold12` | self (12 gold queries, `--limit 10`) | compact vs native | emitted bytes | **11,428 vs 56,868 (-79.9%)** | REPRODUCIBLE |
| `self-budget300-gold12` | self (12 gold queries, `--limit 10`) | `--budget-tokens 300` vs agent-capsule | emitted bytes | **10,395 vs 44,622 (-76.7%)** | REPRODUCIBLE |

These are **not** comparable to the historical unreproducible rows below: a
different corpus definition, a different gold set, and a different commit.
They do not supersede those rows; they are the first rows in this file that a
reader can actually regenerate.

### Negative result: default embeddings add nothing measurable here

The A/B in the same harness (`--ab no-embed`) reports **every delta as exactly
0.000** -- MRR, nDCG, and Recall at 1, 5, 20 are identical with and without the
default embedding channel on this corpus:

| comparison | delta MRR | delta nDCG | delta Recall@5 |
|------------|----------:|-----------:|---------------:|
| hybrid minus `--no-embed` (self, 12 gold) | 0.000 | 0.000 | 0.000 |

This reproduces the warning already recorded for the historical rows, now with
a runnable command behind it. It is recorded here rather than dropped, per the
negative-ledger rule. It does **not** prove embeddings are worthless in
general: this corpus is small, self-referential, and its queries are answerable
by exact and structural channels. It does mean **no claim of semantic lift may
cite this corpus**, and that a foreign held-out corpus is required before the
default embedding path can be called valuable.

### Token budget: measured reduction at unchanged recall (m38g)

`--budget-tokens` assigns a detail level per result (metadata / signature /
block / full) under one response-wide ceiling, rather than truncating every
excerpt to the same size. At a 300-unit budget it emits
**76.7% fewer bytes than the agent-capsule envelope**, clearing the 50%
acceptance gate.

Recall is unchanged, and the harness proves it rather than assuming it: a
budget selects DETAIL, never which results are returned, so
`self-budget-non-inferiority.json` compares the returned result-id lists with
and without the budget across all 12 gold queries and fails the run if any
differ. Current status: **identical result sets on 12/12 queries**, so
Recall@1/@5/@20 are unchanged by construction.

Absolute byte totals move as this repository changes, because the `self` corpus
IS the working tree. Rows above were produced at commit `007b255`. Percentages
are the stable figures; treat absolute bytes as corpus-dated.

### Coverage and what is still missing

Implemented and reproducible: MRR, nDCG, Recall@1/@5/@20, the hybrid vs
no-embed A/B, emitted-byte token efficiency per output format, and the
reliability invariants as executable tests (single-generation responses,
crash-safe generation activation, durability pragmas).

Not implemented, and deliberately not faked with placeholder numbers:

- **Foreign corpora** (ripgrep, Flask, and other held-out repositories). These
  need pinned external checkouts that this harness cannot fetch offline, so no
  foreign-corpus row is claimed.
- **Calibration error** (Brier / ECE). `confidence` currently comes from an
  inspectable agreement heuristic, not a fitted model, so a calibration number
  would measure an arbitrary constant rather than a trained predictor.
- **Definition/reference resolution accuracy and graph-edge precision by
  resolution tier.** These require the `SymbolId` / `Resolution` tiers, which
  are separate open work; there are no resolution tiers to report against yet.
- **Foreign-corpus token efficiency.** The reduction above is measured on
  the self corpus only.
- **Agent-level token metrics** (tokens read before the correct edit site, tool
  calls to correct file and symbol). These need an agent-in-the-loop harness,
  not a retrieval harness.

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

The original run used a `corpora.lock` file that is not shipped in this tree. Its pinned tag and SHA values are preserved here:

| corpus | ref | SHA |
|--------|-----|-----|
| self (this repo) | d3eab74 | `d3eab74c7f3725bae4b1fab24ea94fe3b58d3601` |
| ripgrep | 14.1.1 | `4649aa9700619f94cf9c66876e9549d83420e16c` |
| flask | 3.0.3 | `c12a5d874c5a014495eb2db8a73f40037bc813ac` |
| tokio | tokio-1.38.0 | `14c17fc09656a30230177b600bacceb9db33e942` |
| express | 4.19.2 | `04bc62787be974874bc1467b23606c36bc9779ba` |

## Retrieval quality — self corpus (18 gold queries)

**Reproduction status:** unavailable from this tree. The recorded run used an 18-query eval gold file and retrieval harness that are not checked in; the current `tests/fixtures/ranking/cases.json` is a different schema and corpus. `asgrep eval --gold <gold.json> <root> --ab <mode>` is the supported evaluator shape, but without the original gold file it cannot reproduce these rows.

| tool | MRR | Recall@k | nDCG@k |
|------|----:|---------:|-------:|
| asgrep hybrid | **0.712** | **0.889** | **0.751** |
| asgrep --no-embed | 0.712 | 0.889 | 0.751 |
| asgrep semantic-only | 0.294 | 0.611 | 0.364 |
| ripgrep (file order) | 0.061 | 0.167 | 0.086 |

Note: 0.712 is lower than the previously published ~0.75 / 0.746
(`self-hist-pre-29129bd`, **superseded**); the drop landed
with the reviewed correctness fixes in `29129bd` (ranking changed for one
query). The historical run recorded a `retrieval_gold.rs` gate (MRR >= 0.70), but that harness is not present in this tree. This table supersedes the old figure and remains a published record, not a currently reproducible result.

## Retrieval quality — foreign-corpus bake-off (k=10)

**Reproduction status:** unavailable from this tree. The foreign corpora can be recovered from the pinned SHAs above, but their gold labels and the cross-tool bake-off harness are not checked in. Running `cd benchmarks` alone performs no evaluation.

### ripgrep 14.1.1 (Rust, 14 queries)

**Canonical default-hybrid fingerprint:** `rg-hybrid-default-d3eab74` → **MRR 0.290**.
The neural+rerank figure **0.605** is fingerprint `rg-neural-rerank-d3eab74` only
(see [`losses.md`](losses.md)); it is not the default hybrid row.

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

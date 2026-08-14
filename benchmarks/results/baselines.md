# Baselines

> **Ledger mix:** section tags below (`canonical` / `historical` /
> `UNREPRODUCIBLE` / `reproducible-in-tree`). Vocabulary:
> [`benchmarks/README.md`](../README.md). A section tagged
> `UNREPRODUCIBLE` cannot be regenerated from this tree. The candidate
> `./benchmarks/run_eval.sh` pack is **not** a live quality fingerprint.

> External artifact location (historical dumps, not proof of regeneration):
> [Speed benchmark workflow](https://github.com/AdityaVG13/ast-sgrep/actions/workflows/speed.yml).

Single source of truth for every MRR / recall / latency claim in this
repository. Any number quoted in docs, commit messages, or bead close reasons
must trace back to a row here or carry its own reproduce command. Scores were
produced by the harness, twice, on the machine below — no hand-edited figures.

## Canonical fingerprint rows

**Status: `canonical` + `UNREPRODUCIBLE`.** Cite these ids. The gold + eval
harness that produced them is not in this tree.

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

## Candidate evaluation pack (no canonical run yet)

**Status: `historical` (negative ledger).** `./benchmarks/run_eval.sh` exists
and refuses dirty worktrees. It has **not** produced a canonical fingerprint.

The tree now contains a gold fixture and a harness:

```bash
./benchmarks/run_eval.sh
```

The harness refuses dirty worktrees and writes generated output under
`benchmarks/results/raw/`, which remains untracked. A previous dirty run was
withdrawn because its corpus referenced experimental generation-layout code
that is no longer part of this change. **No value from that run is canonical or
reproducible, and no performance, token-reduction, or retrieval-quality claim
may cite it.** This note remains as the required negative ledger.

The live `self` corpus changes with the worktree and is therefore weak evidence
for retrieval-quality deltas. Use a frozen or foreign corpus before claiming a
quality change. The repository PPMI lexicon is reported as explainable evidence
but is deliberately not fed into ranking; no retrieval lift is claimed.

Still unmeasured here: foreign-corpus quality and token efficiency, confidence
calibration, graph precision by resolution tier, multi-field semantic vectors,
and agent-in-the-loop token/tool-call outcomes.

## Provenance

**Status: `historical`.** Pins for the UNREPRODUCIBLE quality/speed rows below.

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

**Status: `canonical` citation of `self-hybrid-d3eab74` + `UNREPRODUCIBLE`.**
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

**Status: `canonical` for `rg-hybrid-default-d3eab74` / `rg-neural-rerank-d3eab74` + `UNREPRODUCIBLE`.**
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

**Status: `historical` + `UNREPRODUCIBLE` for the table below.** The hyperfine
shape is documented; original `<root>` corpora and `corpora.lock` are not in
this tree, so these numbers are not a live keep. In-tree latency gates are
`asgrep bench` + `.bench-history` (`reproducible-in-tree`).

Illustrative hyperfine shape (does **not** regenerate the published table
without the missing corpora):

```bash
cargo build --profile release-perf -p ast-sgrep-cli
hyperfine --warmup 1 --min-runs 3 --prepare 'rm -rf /tmp/bl.db' \
  './target/release-perf/asgrep --index-path /tmp/bl.db index <root>'
hyperfine --warmup 3 --min-runs 20 \
  "./target/release-perf/asgrep --index-path /tmp/bl.db --json '<query>' <root>"
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

**Status: `historical` + `UNREPRODUCIBLE`.** `watch-bench.py` is **not** in this
tree. Do not treat the commands as a live harness. Numbers below are a
published record only.

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

## Fusion / stage budget map (e2hc.31)

**Status: `historical` (index, no new fingerprint).** Sub-1ms median is the
in-process `CORE_PARTS` gate (`tests/core/sub1ms.rs`), not CLI vs competitors.
Scorecard: [`fusion-scorecard.md`](fusion-scorecard.md). CLI p95 vs ripgrep /
ast-grep: [`speed.md`](speed.md). Quality ids above unchanged.

## Rules

1. No number may be quoted without a section status tag plus a row in this
   file (or another results doc that cites a fingerprint id here).
2. Rebaselining a `canonical` quality row requires the gold + eval harness
   restored in-tree. Until then the fingerprint stays `UNREPRODUCIBLE`.
3. `eval-bakeoff.py` / `results.json` stamping is **absent** from this tree.
   Do not hand-edit historical scores. Do not claim a new MRR win in a close
   reason.

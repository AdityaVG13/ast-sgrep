# ast-sgrep vs context.dev sgrep — sgrep's published suite, non-cloud arms (2026-10-14)

**Status: measured, reproducible-in-tree.** Harness:
[`benchmarks/sgrep_suite/run_suite.py`](../../sgrep_suite/run_suite.py) (`prep`
re-downloads and hash-verifies every input). Scored rows:
[`sgrep-suite-2026-10-14-rows.jsonl`](sgrep-suite-2026-10-14-rows.jsonl),
[`sgrep-suite-2026-10-14-latency-rows.json`](sgrep-suite-2026-10-14-latency-rows.json),
aggregates in [`sgrep-suite-2026-10-14-summary.json`](sgrep-suite-2026-10-14-summary.json) /
`.csv`, setup in [`sgrep-suite-2026-10-14-setup.json`](sgrep-suite-2026-10-14-setup.json).
Raw per-call stdout/stderr (3,880 gz records, 54 MB) retained at
`~/.hermes/cache/scratch/sgrep-bench/records/` at run time; regenerate with the
harness if pruned.

Scope: the retrieval, latency, and setup arms of
[sgrep's benchmark suite](https://github.com/context-dot-dev/sgrep/blob/main/benchmarks/README.md).
The SWE-bench Verified agent arm is **excluded** — it needs paid Modal/OpenAI
calls. CK / jevgrep / ripgrep arms were not rerun; only ast-sgrep vs sgrep.

## Binaries (pinned)

| arm | binary | SHA-256 | provenance |
|---|---|---|---|
| asgrep | local `release-ship` build, reports `asgrep 2.5.4` | `545ee9bc…1959df` | this tree @ `f2bbc3d2` + dirty diff `bc768201…`; **unpublished** (2.6.0 candidate content, version string not yet bumped) |
| sgrep | local `cargo build --release --locked` | `fe03f4ca…83f2a` | source `context-dot-dev/sgrep@0fbe8e5` — the exact commit their published run benchmarked (their binary hash `0ba79953…` differs; different build host/profile). The public v0.3.0 release predates that commit's ranking change and was **not** used for scored rows. |

Machine: macOS 26.6.2, arm64 (Apple Silicon), shared host, `caffeinate -i`
during runs. Sequential fresh CLI process per query.

## Fidelity (calibration vs their published rows)

- RepoQA corpus: per-scope SHA-256 **60/60 equal** to the hashes in their
  `latency-replay.json`. Queries: raw needle descriptions; the 120 latency-panel
  query hashes **120/120 equal** theirs. Full 600-ID set identical to theirs.
- My sgrep arm vs their published `sgrep-main` rows: **rank-exact on 600/600
  RepoQA queries**; coverage (half/full @2000/8000) agrees on 596–598/600.
  CoSQA: rank-exact 239/500 but every Hit@k aggregate matches theirs to three
  decimals (their snippet-file naming is unpublished; mine is `NNNN.py`, and
  sgrep embeds file paths — near-tie orderings flip, bucket totals don't).
- Gold lines derive from the RepoQA needle **byte** span (`start_line` in the
  dataset points at the blank line before the def). Coverage = o200k-token
  walk over deduped returned lines, stop at first line exceeding budget
  (their runner is unpublished; this reimplementation reproduces their
  per-query coverage on 99%+ of queries).

## Results — retrieval (one pass, warm indexes, errors/timeouts = 0 both arms after env fix below)

### RepoQA (600 queries, 60 repos, 6 languages)

| | Hit@1 | Hit@5 | Hit@10 | Hit@100 | MRR@10 | ≥50%@2k | ≥50%@8k | Full@8k |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| **sgrep** | **.662** | **.897** | **.943** | **.973** | **.757** | **.750** | **.895** | **.895** |
| asgrep | .198 | .440 | .572 | .672 | .304 | .237 | .340 | .335 |

### CoSQA (500 queries, 6,267 snippets)

| | Hit@1 | Hit@5 | Hit@10 | Hit@100 | MRR@10 |
|---|---:|---:|---:|---:|---:|
| **sgrep** | **.292** | **.574** | **.702** | **.948** | **.417** |
| asgrep | .038 | .182 | .236 | .358 | .096 |

Per-language RepoQA MRR@10 (asgrep / sgrep): cpp .240/.694, go .298/.801,
java .281/.729, python .299/.756, rust .357/.817, typescript .347/.747.

## Results — latency (their 120-query panel, prepare + 6 repeats, 360 warm calls/arm, 0 errors, 0 ranked-source mismatches)

| | RepoQA p50 | RepoQA p95 | CoSQA p50 | CoSQA p95 |
|---|---:|---:|---:|---:|
| **asgrep** | **15.8 ms** | **51.3 ms** | **12.6 ms** | **16.0 ms** |
| sgrep | 57.3 ms | 87.7 ms | 181.3 ms | 266.5 ms |

Paired per-query median ratio asgrep/sgrep: RepoQA **0.264** (95% CI
0.244–0.287), CoSQA **0.067** (0.050–0.072). My sgrep panel p50s (57.3 / 181.3 ms)
track their published M4 numbers (56.5 / 190.7 ms). **Not quality-adjusted**:
asgrep is faster per call while returning the gold far less often.

## Results — setup & storage (empty index, assets cached)

| | RepoQA first use (60 scopes) | CoSQA first use | Index storage (both datasets) |
|---|---:|---:|---:|
| sgrep | median 133.5 ms, total 8.3 s (first search incl. query) | 477 ms | 173 MB |
| asgrep | median 1,039.9 ms, total 104.9 s (`asgrep index`, excl. first query; first-query median +15.2 ms) | 5,748 ms (+8.9 ms first query) | 2,582 MB |

sgrep setup includes its first query (index-on-first-search); asgrep setup is
the explicit index command. asgrep indexes also carry symbols/callers/imports
for its graph features — a broader artifact, 15× the bytes.

## Method notes / no-cheating record

- Default invocations only. sgrep: `sgrep QUERY . --model code --json -n 50|100`
  (their protocol). asgrep: `asgrep QUERY . --json --limit 50|100
  --index-path ./.asgrep/index.db` (`--index-path` only pins the per-scope
  index, the analogue of sgrep's dedicated `SGREP_CACHE_DIR`). No ranking
  flags, no dataset-specific tuning of either tool, no query edits, no
  retries, no outlier removal, 180 s timeout, failures stay in denominators.
- asgrep ran its shipped default stack (FTS/trigram + `semantic-v2` 256-d
  model-free embeddings; neural embeddings are feature-gated off in this
  build and were not enabled). sgrep ran its shipped potion-code-16M static
  embeddings. Observed output cardinality: asgrep returned a median of ~20
  distinct files on RepoQA and ~27 on CoSQA despite `--limit 50/100`
  (internal response budgets) — measured as-is.
- **Environment-defect correction:** the first Stage-2 pass ran sgrep with
  `HF_HUB_OFFLINE=1` before its `c`/`tsx` parser assets were cached, erroring
  70 RepoQA queries. Those error records are preserved
  (`records-err70-env/`); after warming the parsers (unmeasured), only those
  70 queries were re-run. This matches their protocol (assets cached) and is
  why calibration reaches 600/600.
- One scorer crash mid-run (tiktoken special-token in corpus text) was fixed
  and affected rows re-scored from retained native outputs — no tool was
  re-executed for it.

## Limits

One shared Apple-Silicon host; one-pass retrieval timing is secondary (rank
order ran method-by-method; latency claims rest on the rotated/interleaved
panel). CoSQA snippet-file naming is a reconstruction guess. asgrep numbers
describe this unpublished tree state, not a released 2.6.0. SWE-bench
localization parity or non-parity vs sgrep remains **unmeasured**.

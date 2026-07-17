# Head-to-head results

> **Reproducibility status:** Every numeric row in this report is a historical
> published value and is **unreproducible from this source tree**: the generating
> harnesses, raw corpora, and raw result artifacts are absent. The external
> artifact location is the [Speed benchmark workflow](https://github.com/AdityaVG13/ast-sgrep/actions/workflows/speed.yml).
> No retained artifact is identified there for these historical runs, so this
> link is a storage location, not evidence that a row can currently be regenerated.

> **Published record** of measured results. No runnable harnesses ship in this tree.

This consolidated GATE table reports only measurements already recorded in repository artifacts; it does **not** combine or extrapolate runs. Lower latency is better. Times are wall-clock p50 milliseconds, rounded to two decimals from the raw values below.

## Results

| Win class | Scale / suite | asgrep | Comparator | Result | Evidence |
|---|---:|---:|---:|---:|---|
| Warm lexical query | 23,000 files, 24 queries | **46.22 ms** aggregate p50 | ripgrep 253.99 ms | **24/24 wins; 5.50x faster** | *(historical machine-readable dump; not in-tree)* |
| Warm lexical query | 100,000 files, 24 queries | **156.46 ms** aggregate p50 | ripgrep 1,317.30 ms | **24/24 wins; 8.42x faster** | *(historical machine-readable dump; not in-tree)* |
| Structural query | 23,000 files | **18.93 ms** query-median p50 | ast-grep 188.77 ms | **9.97x faster; parity clean** | *(historical machine-readable dump; not in-tree)* |
| Structural query | 100,000 files | **19.34 ms** query-median p50 | ast-grep 1,347.97 ms | **69.68x faster; parity clean** | *(historical machine-readable dump; not in-tree)* |
| Structural hand-pattern suite | 29 unchanged patterns | **1,520.6 ms** sum of per-pattern p50s | Semgrep 31,875.3 ms | **20.96x faster** | *(historical machine-readable dump; not in-tree)* |
| Retrieval quality | ripgrep, 14 gold queries | **0.605 MRR** | Semgrep hand-patterns 0.536 MRR | **+0.069 MRR** | [`losses.md`](losses.md) |

The three speed win classes hold in the artifacts:

1. **Warm lexical:** `asgrep_wins == queries == 24` at both scales, with no unexplained result diffs.
2. **Structural:** asgrep's query-median p50 is lower and `parity_clean == true` at both scales.
3. **Semgrep suite:** `31,875.276 / 1,520.555 = 20.96x` after published rounding.

Retrieval quality is separate because it is a relevance result, not a speed result.

## Exact artifact cross-check

| Artifact field | 23k / suite value | 100k value |
|---|---:|---:|
| lexical `asgrep_median_p50_ms` | `46.21808583000001` | `156.45849174` |
| lexical `rg_median_p50_ms` | `253.98837289` | `1317.3037741500002` |
| lexical `asgrep_wins / queries` | `24 / 24` | `24 / 24` |
| structural `asgrep_query_median_p50_ms` | `18.933438000000002` | `19.344875000000002` |
| structural `ast_grep_query_median_p50_ms` | `188.7684375` | `1347.9688125` |
| structural `ast_grep_over_asgrep` | `9.970108836018053` | `69.68092647277379` |
| structural `parity_clean` | `true` | `true` |

The Semgrep artifact stores `asgrep_sum_p50_ms = 1520.555`, `semgrep_sum_p50_ms = 31875.276`, `speedup_x = 20.96`, `patterns = 29`, match totals `51 / 19`, and `semgrep_unique_locations = 0`. The retrieval publication records `0.605 / 0.536` in [`losses.md`](losses.md), alongside all 14 reciprocal-rank rows.

## Losses and caveats

Read the query-level retrieval losses in [`losses.md`](losses.md). Three queries are explicit asgrep losses: `rg_std_printer` (rank 10 vs 1), `rg_json_output` (rank 2 vs 1), and `rg_overrides` (rank 5 vs 1). `rg_search_core` is a shared top-10 miss. The comparison is intentionally difficult but asymmetric: asgrep receives natural-language intents directly; Semgrep receives a hand-authored structural pattern per intent.

- **Warm lexical is indexed-vs-scan.** asgrep's index is built before timing; ripgrep scans on each query. These rows measure repeated-query latency, not cold index construction.
- **No cold-start win is claimed.** Index construction and first-query overhead are excluded. Measured cold-index rows and losses remain in [`SPEED.md`](speed.md).
- **The lexical aggregate is a median across 24 query p50s,** not one monolithic command latency. The artifact reports zero unexplained result diffs.
- **Structural parity is normalized by relative file and one-based line.** Each tool has one discarded prefix run and five measured runs; the statistic is median wall-clock time.
- **The Semgrep suite is fixed, not universal.** Its 29 unchanged shorthand patterns come from the bake-off. The ratio does not generalize to arbitrary Semgrep rules.
- **Semgrep rejects 20 of 29 patterns.** Those rows measure process startup plus rejection, not a successful full-corpus scan. They remain to avoid translating or dropping inputs after measurement design. Errors and normalized diffs are retained in *(historical machine-readable dump; not in-tree)*.
- **Match counts are not directly equivalent for rejected/bare-expression patterns.** Accepted structural patterns have no Semgrep-only normalized location; two accepted bare identifiers are strict asgrep supersets under differing bare-expression semantics.
- **Machine and run conditions matter.** Use provenance embedded in each JSON artifact. Do not compare a differently built binary or loaded host as though it were the same run.
- **Older small-corpus losses remain published.** [`SPEED.md`](speed.md) records ripgrep winning lexical search on two of three earlier 82–917-file corpora. The 23k/100k result is a different 24-query suite and does not erase those losses.

## Reproduce and verify

Build the measured profile, then run harnesses or inspect the recorded aggregate. Commands that write JSON overwrite the published artifact.

```bash
cargo build --profile release-perf -p ast-sgrep-cli \
  --features neural-embed,rerank

# Reproduce the published lexical aggregate from recorded evidence.
jq '.scales | with_entries(.value = .value.aggregate)' \
  benchmarks/results-lexical-speed.json

# Structural: one discarded run plus five measured runs.
  --bin target/release-perf/asgrep \
  --sizes 23000,100000 --runs 6 \
  --output benchmarks/results-structural-speed.json

# Fixed 29-pattern suite: one warmup plus five measured runs.
  --bin target/release-perf/asgrep --warmups 1 --runs 5 \
  --output benchmarks/results-semgrep-patterns.json

# Retrieval bake-off: exact environment from losses.md.
ASGREP_NEURAL_EMBED=true ASGREP_RERANK=true \
ASGREP_RERANK_WEIGHT=20 ASGREP_RERANK_BATCH_SIZE=1 \
RAYON_NUM_THREADS=1 ASGREP_NEURAL_INTRA_THREADS=1 \
ASGREP_RERANK_INTRA_THREADS=1 \
    --bin target/release-perf/asgrep
```

For corpus pins, versions, host metadata, feature flags, and noise, treat [`SPEED.md`](speed.md), [`losses.md`](losses.md), and linked JSON as authoritative rather than this rounded summary.

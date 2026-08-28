# Head-to-head results

> **Ledger mix:** see [`benchmarks/README.md`](../README.md). The 2026-08-28
> self-corpus block is `reproducible-in-tree`. Older GATE rows are
> `UNREPRODUCIBLE`.

This file reports only measurements already recorded in repository artifacts.
It does not combine or extrapolate runs. Lower latency is better.

## 2026-08-28 measured (self corpus, 445 tracked files)

**Status: `reproducible-in-tree`.** CLI `hyperfine` on a `git ls-files` copy.
Raw protocol: [`speed.md`](speed.md).

| Win class | asgrep p95 | Comparator p95 | Result |
|---|---:|---:|---|
| Warm lexical (`literal:SearchHit`) | 19.0 ms | ripgrep 11.1 ms | **ripgrep** (1.7×) |
| Structural (`pattern:SearchHit`) | 129 ms | ast-grep 26.5 ms | **ast-grep** (4.9×); latency-only, not match-set |
| Warm semantic NL | 20.3 ms | — | indexed semantic; no scan-tool analogue |
| Cold index | 4.58 s | — | no scan-tool analogue |

Warm lexical is indexed-vs-scan: asgrep's index is built before timing;
ripgrep scans on each query. No cold-start win is claimed.

## Historical GATE (23k / 100k, 2026-07-10)

**Status: `historical` + `UNREPRODUCIBLE`.** Generating dumps are not in
this tree. Latency-only. `parity_clean` in the source dump meant normalized
(path, line) timing pairs succeeded, **not** match-set equality.

| Win class | Scale | asgrep | Comparator | Result |
|---|---:|---:|---:|---|
| Warm lexical | 23,000 files, 24 queries | 46.22 ms aggregate p50 | ripgrep 253.99 ms | 24/24; 5.50× |
| Warm lexical | 100,000 files, 24 queries | 156.46 ms aggregate p50 | ripgrep 1,317.30 ms | 24/24; 8.42× |
| Structural | 23,000 files | 18.93 ms query-median p50 | ast-grep 188.77 ms | 9.97× latency-only |
| Structural | 100,000 files | 19.34 ms query-median p50 | ast-grep 1,347.97 ms | 69.68× latency-only |
| Structural hand-pattern suite | 29 patterns | 1,520.6 ms sum of p50s | Semgrep 31,875.3 ms | 20.96× |
| Retrieval quality | ripgrep, 14 gold | 0.605 MRR (`rg-neural-rerank-d3eab74`) | Semgrep 0.536 MRR | +0.069; **not** default hybrid 0.290 |

Those large-corpus rows do not erase small-corpus losses: on the current
self tree, ripgrep and ast-grep win the CLI races above. Query-level
retrieval losses remain in [`losses.md`](losses.md).

## Caveats

- The lexical 23k/100k aggregate is a median across 24 query p50s, not one
  command. The Semgrep suite is a fixed 29-pattern shorthand set; Semgrep
  rejected 20 of 29 patterns (startup plus rejection, not a full scan).
- Structural `parity_clean` is latency-only.
- Machine and flags matter. Do not compare a differently built binary as
  though it were the same run.

# Native pattern search work-span profile

Measured in `release-perf` on the DGX Spark against this repository with 20 Rayon workers and pattern `Searcher::new($$$ARGS)`:

```bash
ASGREP_PATTERN_PROFILE_FIXTURE=. \
  cargo test --locked --profile release-perf -p ast-sgrep-core \
  --test pattern_prefilter -- --include-ignored --nocapture
```

The profile runs each sample three ways: one worker without the literal prefilter, one worker with it, and the production Rayon pool with it. It rejects the sample if any run returns a different hit identity set. Five repeated samples produced:

| Metric | Median | Range |
|---|---:|---:|
| Indexable files considered | 38 | fixed |
| Files rejected by SIMD prefilter | 27 | fixed |
| Candidate files parsed | 11 | fixed |
| Hits | 9 | fixed |
| No-prefilter one-worker baseline | 393.4 ms | 386.0-400.4 ms |
| Prefiltered T1 | 130.5 ms | 130.4-130.7 ms |
| T-infinity | 49.17 ms | 49.15-49.19 ms |
| Measured 20-worker span | 78.24 ms | 50.33-80.13 ms |
| Brent upper bound | 55.70 ms | 55.67-55.71 ms |
| Serial fraction | 0.163% | 0.155-0.249% |
| Prefilter speedup | 3.02x | 2.96-3.07x |
| Observed parallel speedup over T1 | 1.67x | 1.63-2.60x |

The SIMD prefilter prevents parsing 71.1% of indexable files and reduces measured one-worker runtime by roughly two thirds. After filtering, parse and match work is about 99.5% of T1; walk plus deterministic result ordering is about 0.16%.

T1/T-infinity caps ideal speedup near 2.65x for this query regardless of additional cores because one candidate file dominates the critical path. The median measured span exceeds the algebraic Brent bound by about 40%; the fastest samples approach the bound. That gap is scheduler, pool-entry, and measurement overhead omitted from the ideal work-span model, not unaccounted serial algorithmic work.

This rejects broad N-core expansion as the next optimization. The largest remaining term is candidate parse and match work, specifically the maximum per-file task. Index-backed pattern signatures already avoid reparsing when an indexed signature answers the query. Further gains should extend that incremental path or split oversized candidate files, not optimize the sub-1% walk and ordering terms.

The old 4.6% embedding and 3.0% SQLite break-even targets do not transfer directly to structural matching. Here the literal prefilter clears them with a measured 67% one-worker reduction. After it lands, content-hash and indexed-signature reuse target the dominant parse and match term and therefore remain higher leverage than micro-optimizing SQLite, walking, or ordering.

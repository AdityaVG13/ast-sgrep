# Instrumentation contract (stage attribution)

Profiling-only wall attribution for index/search hot paths. **Does not change algorithms, thresholds, or cache sizes.**

| | |
|---|---|
| Gate | `ASGREP_PERF_PROFILE=1` (boolish) |
| Optional sink | `ASGREP_PERF_PROFILE_PATH` (JSONL append; default stderr) |
| Implementation | `crates/ast-sgrep-core/src/perf_profile.rs` |
| Sample | `benchmarks/results/perf_profile_sample.jsonl` |

Events: `perf.profile.run_start`, `perf.profile.sample_collected`, `perf.profile.span_summary`, `perf.profile.run_complete`.

Index exclusive stage names used by [stage-timers-post-T1R.md](validation/stage-timers-post-T1R.md):
`index_walk_parse`, `sqlite_upsert`, `semantic_ivf_build`. `embed_hash` is nested inside prepare.

# Instrumentation contract (stage attribution)

Profiling-only measurement for attributing wall time to known hot stages.
**Does not change algorithms, thresholds, or cache sizes.**

## Run id

| Field | Value |
|-------|--------|
| Run id | `2026-08-06-perf-profile-pass8` |
| Focus | Stage timers for index/search hot paths |
| Gate | `ASGREP_PERF_PROFILE=1` (boolish) |

Smoke capture: [`benchmarks/results/perf_profile_sample.jsonl`](../benchmarks/results/perf_profile_sample.jsonl)

---

## Inventory (what already existed)

| Surface | Location | Role |
|---------|----------|------|
| `env_flag` / boolish `ASGREP_*` | `crates/ast-sgrep-core/src/env_flag.rs` | Shared flag parsing |
| `pipeline_parts` microbench | `crates/ast-sgrep-core/src/pipeline_parts.rs` | In-process sub-1ms part timing (bench harness, not production) |
| CLI bench / suite | `crates/ast-sgrep-cli/src/bench.rs` | Wall times for suite cases |
| IVF open probe example | `crates/ast-sgrep-core/examples/semantic_ivf_open_probe.rs` | One-shot open latency |
| `ASGREP_PERF_ASSERTS=1` | tests + `docs/validation/semantic-ivf-mmap.md` | Warm-path p99 **assert** gate (not stage logs) |
| Cost narrative | `docs/PERF_INVENTORY.md` | Human inventory of cost drivers |
| Baselines | `benchmarks/results/baselines.md` | Canonical published numbers only |
| Supervisor / duty cycle | CLI supervisor | CPU limit, not stage attribution |
| `eprintln!` failures | index walk / prepare | Error path only |

**Gap filled by this pass:** no env-gated, structured stage log on production hot paths (`index_all`, blake3 hash, SQLite bulk upsert, IVF `build_from_flat`, `Searcher::search`).

---

## Enable

```bash
# stderr JSON lines
ASGREP_PERF_PROFILE=1 asgrep index /path/to/root

# append to a file (preferred for capture)
ASGREP_PERF_PROFILE=1 \
ASGREP_PERF_PROFILE_PATH=benchmarks/results/perf_profile_sample.jsonl \
  asgrep index /path/to/root --json

ASGREP_PERF_PROFILE=1 \
ASGREP_PERF_PROFILE_PATH=benchmarks/results/perf_profile_sample.jsonl \
  asgrep "your query" /path/to/root --json
```

Boolish values: `1` / `true` / `yes` / `on` (case-insensitive). Flag is read once per process (`OnceLock`).

**Overhead when off:** one atomic/OnceLock check per instrumented entry; no allocations, no I/O.

---

## Events (JSONL)

Each line is one JSON object with `"event"`:

| Event | When |
|-------|------|
| `perf.profile.run_start` | Enter `index_all` or `Searcher::search` |
| `perf.profile.sample_collected` | End of a wall-clock `Span` (not every per-file hash sample) |
| `perf.profile.span_summary` | End of run: `{ span, cumulative_us, count, p50_us, p95_us, category, evidence }` |
| `perf.profile.run_complete` | End of run: `{ wall_us, label, run_id }` |

`perf.profile.hypothesis_evaluated` is reserved; not emitted until a hypothesis harness wires it.

---

## Spans (stage names)

| Span | Category | Where | Evidence |
|------|----------|-------|----------|
| `index_walk_parse` | `index` | `Indexer::index_all` | WalkDir + parallel `prepare_file` (read / hash / tree-sitter extract) |
| `embed_hash` | `index` | `prepare_file` | blake3 `hash_content` per file (aggregated samples) |
| `sqlite_upsert` | `index` | `Indexer::index_all` | Bulk `upsert_file` transaction |
| `semantic_ivf_build` | `semantic` | `SemanticAnnIndex::build_from_flat` | k-means IVF build |
| `search_query` | `search` | `Searcher::search` | Mode dispatch + finish (includes cache path) |

Module: `crates/ast-sgrep-core/src/perf_profile.rs`.

---

## Smoke procedure

1. Build: `cargo build -p ast-sgrep-cli`
2. Cold index sample fixture into a temp dir with profiling path set.
3. Warm search against that index with the same path (append).
4. Confirm `run_start` / `span_summary` / `run_complete` lines for `index_all` and `search_query`.

Do not quote wall times from the smoke file as product claims; they are debug traces only (see AGENTS.md benchmark honesty).

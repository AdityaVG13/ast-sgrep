# Stage wall timers post-T1-R (hoy3.2)

MEASURE only. No product source change. Existing `ASGREP_PERF_PROFILE` events already
separate prepare vs serial upsert vs IVF kmeans. No new probe points.

## Provenance

| Field | Value |
|---|---|
| Run id | `20260814T013532Z` |
| Tree SHA | `9be8d52` (`feat/golden-assert-testkit`) |
| Binary | `target/release-perf/asgrep` (Mach-O arm64, mtime 2026-08-13 17:22) |
| Host | Darwin arm64, macOS 26.5 |
| Isolation | local Darwin (same host class as hoy3.1 samply; not the C4 Linux 1.934 s mean) |
| Corpus root | `/Users/aditya/AI/ast-sgrep-wt-nz7i` |
| Files indexed | **443** (61 skipped) |
| Semantic chunks | **5675** |
| ANN / IVF | **on** (`semantic_ivf_present: true`, hashed `semantic-v2`, dim 256) |
| e2e `/usr/bin/time` | **3.00 s** real / 5.03 s user |
| `index_all` wall | **2.979 s** (`perf.profile.run_complete.wall_us`) |
| Raw JSONL | `tests/artifacts/perf/20260814T013200Z/stage_timers_post_T1R.jsonl` (gitignored) |

This is **not** C4 residual mean 1.934 s / p95 1.965 s (different host, SHA, file count).
Do not overwrite C4. Ratios on this host are the deliverable.

n=1 cold run, so mean = p50 = p95 for the exclusive index stages.

## Exclusive stages

`embed_hash` samples sit inside `index_walk_parse`. Do not add them to the exclusive sum.
`semantic_ivf_build` runs after the upsert span drops (`rebuild_dirty_sidecars`).

| Stage | Event span | Mean / p50 / p95 (s) | % of `index_all` wall |
|---|---|---:|---:|
| prepare (parallel walk+parse) | `index_walk_parse` | 0.415 | **13.94%** |
| serial upsert | `sqlite_upsert` | 1.954 | **65.60%** |
| IVF kmeans | `semantic_ivf_build` | 0.529 | **17.77%** |
| other (advertise, sidecar I/O, lexicon, …) | e2e remainder | 0.080 | **2.69%** |

Exclusive named stages sum to 97.31% of `index_all` wall. Remainder is not a hidden upsert overlap.

Nested `embed_hash`: 443 samples, cumulative 7.7 ms (0.26% of wall). Not a stage.

## UPSERT residual vs 5% reopen_gate

**yes** -- serial `sqlite_upsert` is **65.60%** of cold-index-self wall on this host
(≥5%). Wall share is much larger than hoy3.1 exclusive-CPU sqlite (9.37%) because
prepare is parallel (0.42 s wall, high CPU) while upsert is capacity-1
(C13/C22). This packet does **not** open a multi-connection UPSERT product bead.

C15 (upsert residual impact) moves from open [E] toward **[V] on this host/SHA**:
serial upsert is the majority of cold-index wall. Do not treat that as a C4
absolute or as license to ship multi-conn here.

## Reproduce

```bash
rm -f /tmp/asgrep-hoy32-s2-cold.db /tmp/asgrep-hoy32-s2-cold.db-wal /tmp/asgrep-hoy32-s2-cold.db-shm
ASGREP_PERF_PROFILE=1 \
ASGREP_PERF_PROFILE_PATH=/tmp/hoy32_stage_timers.jsonl \
  ./target/release-perf/asgrep --json --index-path /tmp/asgrep-hoy32-s2-cold.db index .
python3 -c '
import json
from pathlib import Path
rows=[json.loads(l) for l in Path("/tmp/hoy32_stage_timers.jsonl").read_text().splitlines() if l.strip()]
wall=next(r["wall_us"] for r in rows if r.get("event")=="perf.profile.run_complete")
excl={"index_walk_parse","sqlite_upsert","semantic_ivf_build"}
for r in rows:
    if r.get("event")!="perf.profile.span_summary":
        continue
    pct = 100 * r["cumulative_us"] / wall
    kind = "EXCL" if r["span"] in excl else "nested"
    print(r["span"], r["cumulative_us"] / 1e6, f"{pct:.2f}%", kind)
'
```

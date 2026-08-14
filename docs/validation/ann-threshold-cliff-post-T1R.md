# ANN threshold cliff post-T1-R (hoy3.4)

MEASURE only. `DEFAULT_ANN_THRESHOLD` remains **2000**. No product default change.

## Provenance

| Field | Value |
|---|---|
| Run id | `20260814T014600Z` |
| Tree SHA | `0c5e83a` (`feat/golden-assert-testkit`) |
| Binary | `target/release-perf/asgrep` (Mach-O arm64) |
| Host | Darwin arm64, macOS 26.5 |
| n | **5** cold-index runs, hyperfine `--warmup 0`, nearest-rank p95 `idx = floor((n-1)*95/100)` |
| Gate | `chunk_count >= 2000` (`should_use_ann` / `ASGREP_ANN_THRESHOLD`) |
| Raw | `tests/artifacts/perf/20260814T014600Z/ann_{below,above,above_flat}.json` (gitignored) |

Pre-T1 SC3 +1–3 s is **stale (C11)**. Do not quote it as post-T1-R magnitude.

Sidecar path is `parent(index.db)/semantic.ivf`. DBs sharing `/tmp` share one sidecar -- this run used isolated dirs.

## Corpora (synthetic Python, 100 files each)

Planted token `zx9q_hoy34` in `m000.py` on both.

| Band | Root | fns/file | symbols | chunks | IVF sidecar |
|---|---|---:|---:|---:|---|
| below gate | `/tmp/hoy34_below` | 9 | 900 | **1799** | absent |
| above gate | `/tmp/hoy34_above` | 11 | 1100 | **2199** | present |

Same file count. Above has 200 extra tiny functions (~400 extra chunks). That confounds the paired Δ; the isolate row holds the corpus fixed.

## Cold-index wall (seconds)

| Condition | chunks | IVF | mean | p95 | min | max |
|---|---:|---|---:|---:|---:|---:|
| below, default 2000 | 1799 | off | 0.142 | 0.142 | 0.138 | 0.152 |
| above, default 2000 | 2199 | on | 0.265 | 0.268 | 0.258 | 0.268 |
| above, `--ann-threshold 999999` | 2199 | off | 0.170 | 0.174 | 0.163 | 0.176 |

| Δ | mean | p95 | Label |
|---|---:|---:|---|
| paired below→above | **+0.123 s** | **+0.126 s** | [E] mixed (gate + 400 chunks) |
| isolate IVF on vs off, 2199 chunks | **+0.095 s** | **+0.094 s** | **[V] this host/corpus** |

IVF incremental is ~**95 ms** (~36% of the 2199-chunk IVF-on mean). Not +1–3 s.

## Quality

| Probe | Result |
|---|---|
| planted `zx9q_hoy34` @10 | top-1 `m000.py` / `planted_hoy34_marker` on below, IVF-on, and IVF-off; scores 0.8862 |
| `return value` @20, IVF-on vs IVF-off, **same** 2199 corpus | Jaccard **1.0** (20/20) |

This is a synthetic near-duplicate function corpus, not a retrieval gold. Identical @20 does **not** prove recall@k invariance on real trees. It is enough to refuse a silent default change: no measured search win, and build cost is real.

## Conclusion

- Cliff magnitude post-T1-R, labeled host/synthetic: **~0.095 s** IVF-on minus IVF-off at 2199 chunks ([V] here).
- Paired 1799 vs 2199 Δ is larger (~0.12 s) and **[E]** as a pure gate effect.
- **No default change.** FREEZE ANN-THR SKIP stands. Human ACK + real-corpus recall@k required before touching `DEFAULT_ANN_THRESHOLD`.
- Do not treat sample IVF-off (~0.042 s class, C18) vs self ANN-on as this cliff.

## Reproduce

```bash
# corpora: 100 Python files, 9 vs 11 defs (planted token in m000.py)
hyperfine --warmup 0 --runs 5 \
  --prepare 'rm -f /tmp/hoy34_idx_below/index.db /tmp/hoy34_idx_below/index.db-wal /tmp/hoy34_idx_below/index.db-shm /tmp/hoy34_idx_below/semantic.ivf' \
  --export-json /tmp/hoy34_below_hf.json \
  './target/release-perf/asgrep --json --index-path /tmp/hoy34_idx_below/index.db index /tmp/hoy34_below'
# same for above (default threshold) and above with --ann-threshold 999999 in an isolated dir
```

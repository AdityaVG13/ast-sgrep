# Semantic IVF mmap validation

## Contract

Sidecar format 2 keeps the `ASIVF\0` magic, uses a fixed validated header, stores the centroid/posting index first, aligns vectors to a 4096-byte boundary, and maps vector bytes read-only through the sealed `ast-sgrep-mmap` crate (the only intentional `unsafe` in the workspace; product crates `forbid(unsafe_code)`). Opening a sidecar may decode the much smaller cluster index, but it does not read or allocate the vector payload. Writers create and fsync a unique temporary file before publication; existing mappings continue to reference their prior inode. Windows publication that is blocked by another process's mapping keeps the prior sidecar and the new in-memory index, leaving the stale marker set for a later retry.

## Fixture

- Host: Apple M5 Max (`Mac17,6`), Darwin 25.5.0 arm64.
- Rust compilation: RCH remote compilation helper, release-perf profile.
- Population: 10,000 vectors, dimension 8, deterministic IVF clusters.
- Sidecar: 365,056 bytes; vector payload 320,000 bytes.
- Samples: 100 per reported percentile; p99 is the 99th sorted observation.

## Results

| Metric | p99 | Definition |
|---|---:|---|
| Cold open | 0.963 ms | Each sample used a unique sidecar written with macOS `F_NOCACHE`, fsynced, then opened in a fresh process. |
| Fresh-inode open | 0.135 ms | Unique inode per sample with ordinary OS page-cache policy; file preparation excluded from the timed region. |
| Warm open | 0.037 ms | Repeated read-only opens of the same page-cached sidecar after an untimed warmup. |

Warm p99 is 27.0 times below the 1 ms acceptance ceiling. The benchmark separately reports mapped vector bytes and resident decoded-index bytes, and fails if the vector payload becomes owned memory.

## Targeted commands

```bash
ASGREP_PERF_ASSERTS=1 cargo test --locked --profile release-perf \
  -p ast-sgrep-core --test semantic_ivf_roundtrip -- \
  --nocapture --test-threads=1

cargo build --locked --profile release-perf -p ast-sgrep-core \
  --example semantic_ivf_open_probe
```

The cold run prepares 100 unique files with `F_NOCACHE` before invoking `semantic_ivf_open_probe open`; file creation and cache-control work are outside the timed region. Do not relabel the in-process `fresh_inode_p99_ns` metric as disk-cold latency.

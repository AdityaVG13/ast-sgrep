# T1-R sidecar bit-identity (hoy3.5)

Docs only. No product code change.

T1-R is a **cost/eval** lever (C4 walls), not a promise that IVF sidecar bytes
or similarity scores match pre-T1-R dumps.

## What is identical

| Claim | Statement |
|---|---|
| **C8** | For L2-unit vectors, exact real cosine equals the inner product. Algebraic, not a float proof. |
| **T1-B kmeans** | Parallel per-row assignment + serial row-order centroid reduce is bit-identical to the pre-T1 **multi-copy serial path under the same metric** (`semantic_ann.rs` `build_from_flat` comment). |

Same-metric means: same `dot_similarity` (or same `cosine_similarity`), same
renorm, same `k` / iterations. It does **not** mean pre-T1 cosine dumps equal
post-T1 unit-dot dumps.

## What is not identical (C9)

| Side | Path |
|---|---|
| Pre-T1-R typical | `cosine_similarity`: f64 accumulators, divide by L2 norms, cast to f32 |
| Post-T1-R search/kmeans | unit-renorm then `dot_similarity`: simsimd `f32::dot` when `dim >= 64` (embed dim is 256), else scalar f32 sum |

simsimd f32 dots are **not** bit-identical to the f64 cosine path. Argmax may
still agree often. Sidecar bytes (centroids, assignments, published IVF frame)
are **not** guaranteed equal across the T1-R metric boundary. Do not fail
goldens or round-trip tests that compare pre-T1 IVF files to post-T1 files and
call that a product regression.

C4 mean 1.934 s / p95 1.965 s is a wall win, not identity evidence.

## Fingerprint (C21)

`compute_ann_fingerprint` binds the derived sidecar to generation inputs.
Mismatch → rebuild. Do not force old bytes onto a new fingerprint.

## Operator rule

- Compare sidecars only within one metric + fingerprint.
- Residual-leaf CPU (hoy3.1) and stage walls (hoy3.2) do not restore
  pre-T1 sidecar identity.
- Campaign notes in `tests/artifacts/perf/opt-20260806/L9_CHANGE.md` (when
  present) are the historical write-up; this file is the in-tree operator doc.

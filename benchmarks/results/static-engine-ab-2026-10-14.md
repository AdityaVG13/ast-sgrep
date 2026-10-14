# Static embed engine A/B — 2026-10-14

Phase-1 evidence for the native static embedding engine
(`docs/plans/native-embed-engine.md`). Frozen reconstruction suite:
`benchmarks/sgrep_suite/run_suite.py`, 600 RepoQA + 500 CoSQA frozen
queries, identical scorer for every arm, fresh process per query. Backend
identity asserted from stored index meta (`embed_backend`), never from
requested flags. Per-arm summaries are local evidence under
`~/.hermes/cache/scratch/sgrep-bench/bench-<arm>/summary-*.json`
(ephemeral scratch; this file is the durable record).

Provenance: tree at HEAD `f2bbc3d2` plus this effort's changes (committed
with this file). Arms ran on successive `release-ship` builds of that tree,
each containing exactly one more lever than the previous; the final (1c)
binary sha256 `b3e858af…80530`. sgrep column is the competitor's pinned
binary on the same suite, reference only (its home suite).

## Results (MRR@10 / Hit@1 / Hit@10 / Hit@100, p50 ms)

| Arm | RepoQA (600) | CoSQA (500) |
|---|---|---|
| hashed (baseline) | .304 / .198 / .572 / .672, 16.4 | .096 / — / — / .358, — |
| static 1a (engine only) | .304 / .198 / .572 / .672, 15.2 | .107 / — / — / .358, — |
| static 1b (+ ungate) | .478 / .333 / .797 / .907, 18.8 | .177 / .102 / .384 / .806, 15.5 |
| static 1c (+ magnitude fusion) | .494 / .345 / .825 / .907, 21.5 | .197 / .124 / .410 / .804, 17.2 |
| semantic-only diagnostic | .522 / .365 / .847 / .892, 9.4 | .100 / .056 / .226 / .560, 12.6 |
| sgrep (reference) | .757 / .662 / — / .973, 59.6 | .417 / — / — / .948, — |

Index wall-time, same 61 corpus roots: hashed ~107 s → static ~63 s.

## Interpretation

- **1a ≈ 0 on RepoQA, +.011 on CoSQA.** The cascade ran the embed pass
  only as thin-page rescoring; recorded contributors show embed reached
  returned hits in 4/600 RepoQA queries (rank changed on exactly 1).
  Engine quality was structurally invisible there. Where embed did vote
  (CoSQA, 277/500), static already beat hashed 43:22 on changed ranks.
- **1b is the main effect.** Embed as a global candidate generator for
  Conceptual intent: RepoQA 279 queries better / 55 worse, CoSQA 305 / 69,
  every RepoQA language up. Cost ≈ +3 ms p50.
- **1c is a smaller, consistent gain** (+.016 RepoQA, +.020 CoSQA MRR@10).
  Gate diagnostic motivating it: embed channel alone (.522) outranked the
  fused 1b result (.478) on RepoQA — rank-only RRF dilutes a decisive
  cosine under weak multi-channel corroboration. On CoSQA the relation
  inverts (.100 alone vs .177 fused); lexical channels carry CoSQA, and
  1c did not harm that.
- Fused still trails semantic-only on RepoQA ranking precision (.494 vs
  .522) while beating it on recall (Hit@100 .907 vs .892). Recorded as a
  Phase-4 diagnostic candidate, not tuned against here.

## Default-path regression check (negative result, pre-existing)

`self_gold_has_no_loss` (ignored test, indexes this repo with the default
hashed backend) fails 5/17 — all `conceptual-*` queries. Mutant control:
with the 1c magnitude vote multiplied by 0, the identical 5 queries miss.
The failures therefore pre-date Phase 1 (gold fixture pins an older
commit; working tree carries unrelated in-flight changes) and are not
caused by these levers. They become the Phase-2 acceptance target under
the static backend. `numerical_fusion` contract suite: 8/8 pass,
including the new magnitude-vote contract (zero-magnitude mutant fails
it, as required).

## Phase 2A/2B verification (2026-10-14)

2A flipped the default (`EmbedPreference::Auto` → Static; chain terminal
fallback → static engine). 2B added model-identity migration: an Auto
`index` run over a legacy store performs the proven-complete rewrite
with an explicit log line; Auto search refuses a stale-model store
until then.

Frozen-suite arm, **no `ASGREP_EMBED_BACKEND` in the environment**,
checked-in driver `benchmarks/sgrep_suite/run_asgrep_arm.py`
(reindexes each corpus repo, asserts the stored backend from meta):

- stored backend: `static` (asserted)
- RepoQA MRR@10: **.494**, CoSQA MRR@10: **.197** (both match the
  Phase-1 static best), p50 19 ms / 16 ms

Driver caveat recorded for future runs: an earlier scratch driver passed
`--index-path` at index time while `run_suite` searches via default
`.asgrep` discovery (parent walk) — the searched index was silently not
the indexed one, producing a spurious .304. `run_asgrep_arm.py` never
uses `--index-path` and asserts `.asgrep/index.db` exists per repo.

`self_gold_has_no_loss` with the static default: **2/17 misses**
(was 5/17 hashed). Residuals: `conceptual-snapshot-consistency`
(misses the `crates/ast-sgrep-core/src/search/types.rs` leg in top-20)
and `conceptual-follow-up-derivation` (misses
`crates/ast-sgrep-core/src/search/planner.rs`). Ranking-quality
residuals owned by Phase 2D/3; not tuned against.

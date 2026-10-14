# Native Embed Engine — Plan

Status: **Phase 1 landed; Phase 2A+2B landed** (static is the default;
stale-model stores refuse under Auto and migrate on the next `index`
run). Remaining Phase-2 items (2C deletions, 2D ablation) and Phase 3
below are approved; each step still lands as its own verified commit. All quality numbers cited here trace to
[`benchmarks/results/static-engine-ab-2026-10-14.md`](../../benchmarks/results/static-engine-ab-2026-10-14.md)
and the frozen reconstruction suite in `benchmarks/sgrep_suite/`.

## Why a built-in engine

Peer tools (swe-grep class, sgrep) rent a side model: server, first-run
download, or hosted inference. asgrep instead ships the inference engine in
the binary — a static token-embedding table with a native WordPiece
tokenizer and mean-pooling. What that buys, structurally:

- **Zero model-load latency per invocation.** asgrep is a one-shot CLI; a
  table lookup is microseconds and needs no warm daemon.
- **Deterministic vectors.** Same binary → same embedding on every machine;
  indexes are portable and reproducible.
- **Tunability peers don't have.** Pooling can be field-aware against
  asgrep's own chunk format (symbol / path / body), and the table itself can
  later be replaced by our own distilled weights.
- **No network, no cache directory, no runtime dependency.** The vendored
  table is compiled in with `include_bytes!` and hash-gated in `build.rs`;
  corrupted or swapped weights fail the build.

Honest boundary: native inference is not "model-free". The binary carries
learned parameters (~31 MB f16 table + ~1 MB vocab); it removes the model
*runtime*, not the learned asset.

## Phase 1 — landed

Three levers, each measured separately on the frozen suite (600 RepoQA +
500 CoSQA queries, identical scorer; backend verified from stored index
meta, never from requested flags):

| Lever | Change | Where |
|---|---|---|
| 1a | `StaticEmbedder`: native WordPiece + vendored MIT potion-code-16M-v2 table (placeholder weights), behind existing `Embedder` trait; dim 256 = hashed dim, no schema change | `crates/ast-sgrep-embed/` |
| 1b | Embed channel ungated for Conceptual intent: permanent global candidate generator instead of thin-page-only rescoring; requires a learned backend (`is_learned()`), default hashed path untouched | `crates/ast-sgrep-core/src/search/mod.rs` |
| 1c | Magnitude vote in fusion: semantic channel adds `w · rrf(0) · (s / s_top)` beside its rank vote, so a decisive cosine isn't outvoted by weak multi-channel corroboration; non-semantic scores bit-identical | `crates/ast-sgrep-core/src/fusion.rs` |

Outcome (frozen-suite A/B, details in the results doc): engine swap alone
moved nothing because the cascade gate starved the channel (embed
contributed in 4/600 RepoQA queries). Ungating was the main effect;
magnitude fusion added a smaller, consistent gain on both datasets. The
static engine also indexes substantially faster than the hashed XOF
pipeline. Remaining gap to sgrep's home-suite numbers is documented, not
hidden.

Phase 1 leaves defaults unchanged: the static backend is opt-in via
`ASGREP_EMBED_BACKEND=static` / `EmbedPreference::Static`. (Superseded
by Phase 2A below.)

## Phase 2A+2B — landed

- **2A (91347dc8):** `EmbedPreference::Auto` resolves to Static in the
  embed chain and in model-identity resolution; the chain's terminal
  fallback is the static engine (Semantic is an explicit chain entry,
  no longer the silent trapdoor). Verified: no-env frozen-suite arm
  stores `embed_backend=static` and reproduces RepoQA MRR@10 .494.
- **2B:** model-identity migration. `semantic_identity_matches`
  compares the stored `embed_model` against the *active* preference's
  identity, so an Auto build over a legacy (hashed) store takes the
  proven-complete rewrite path and logs the migration; the rewrite
  purges `embed_cache` rows for other models. Under Auto, search
  refuses a stale-model store with the migration instruction (explicit
  preferences keep stored-wins semantics). Targeted single-file updates
  refuse to mix vector spaces. Pinned by
  `tests/core/semantic_layout_rewrite.rs`
  (`auto_index_migrates_legacy_semantic_store_to_static`, mutant-killed).
- `self_gold_has_no_loss` under the static default: 5 misses → 2
  (`conceptual-snapshot-consistency` misses the `search/types.rs` leg;
  `conceptual-follow-up-derivation` misses `search/planner.rs`).
  Both are ranking-quality residuals for 2D/Phase 3, not migration
  defects; not tuned against.

## Phase 2 — remaining (approved)

Work items:

1. ~~Flip `EmbedPreference::Auto` to resolve to Static.~~ (2A, done.)
   Remove `HashedEmbedder`, the concept-expansion path, the
   `semantic-v2` model identity, and the fastembed/ONNX neural path
   (`ASGREP_NEURAL_EMBED`) with its Cargo dependencies. (2C; rerank
   goes with it, fastembed leaves the tree entirely.)
2. ~~Index migration~~ (2B, done — implemented as full re-embed via the
   proven-complete rewrite path on identity mismatch, with an explicit
   log line and dead-model `embed_cache` purge; search under Auto
   refuses a stale-model store until then.)
3. Ablate compensation scaffolding that only existed to contain hashed
   noise, one piece at a time, each gated on the frozen suite: thin-page
   fallback condition, conceptual weight shape, field-rescore heuristics.
   Keep whatever still earns its place with the learned backend. (2D)
4. Docs/contracts: `crates/ast-sgrep-embed/CONTRACT.md`,
   `docs/semantic-search.md`, `docs/fusion-ranking.md`. (2F)

Acceptance criteria:

- `self_gold_has_no_loss` passes with the static backend. (2A+2B state:
  2/17 residual misses, named above; closing them is 2D/Phase-3 work.)
- Frozen suite: no regression vs Phase-1 static numbers on either dataset
  (2A verified .494 no-env); fresh-index storage bytes and index wall-time
  measured and recorded.
- ~~Old-index behavior covered by a test~~ (2B, done).

Risks: first-run re-embed cost on large existing indexes; binary size step
(~+32 MB) becomes unconditional.

## Phase 3 — our own weights (offline distillation)

Replace the placeholder MIT table with weights trained for this engine.
The engine contract (WordPiece vocab dimension, 256-d rows, mean-pool,
cosine) stays fixed, so this is an asset swap behind the same pinned-hash
gate — no runtime change.

1. Corpus: license-clean code + natural-language pairs (docstrings,
   comments, commit messages, identifier↔prose). No benchmark queries or
   gold labels from the frozen suite in any training split.
2. Teacher: a strong open code-embedding model run offline; distill its
   token-level geometry into a static table (Model2Vec-style), then
   fine-tune the table on the pair corpus. Training runs on the Spark GPU.
3. Tokenizer: keep the current vocab for index compatibility, or version
   the model identity (`static:<name>:256`) and take the Phase-2 migration
   path if the vocab changes.
4. Acceptance: beats the placeholder table on the frozen suite (both
   datasets, no per-language collapse) **without any tuning against the
   suite** — the suite is the judge, never the training signal. Validate
   on a held-out pair set during training.

If Phase 3 slips, the MIT placeholder table is license-clean to keep
indefinitely.

## Phase 4 — candidates (each starts with its own gate diagnostic)

- **Field-aware pooling.** Field vectors (symbol / path / body) are already
  embedded and stored; test per-field pooling weights at query time vs the
  current field-rescore. Gate: semantic-only diagnostic by field.
- **Confidence-scaled fusion.** The 1c magnitude vote is relative
  (`s / s_top`). Test margin-based confidence (top-vs-bulk separation) so
  the embed channel can also *abstain* on flat distributions. Gate:
  offline simulation on retained per-query records before any code change.
- **ANN/IVF revisit** — only if corpus sizes make brute-force cosine the
  bottleneck *and* the retry-condition predicates in
  `docs/progress/perf-negative-results.md` are satisfied. Brute force at
  current scale is a few ms; do not pre-build this.

## Non-goals

- Hosted inference, sidecar model servers, first-run weight downloads.
- LLM-in-the-loop query handling at search time.
- Chasing a competitor's home-suite number by tuning on that suite.

## Decision log

- 2026-10-14: weights vendored in-repo + `include_bytes!` (over build-time
  fetch and first-run download) — true single binary, offline builds.
- 2026-10-14: dim 256 retained to avoid an index schema change in Phase 1.
- 2026-10-14: placeholder table = minishlab/potion-code-16M-v2 (MIT),
  pinned by sha256 in `crates/ast-sgrep-embed/assets/README.md`.

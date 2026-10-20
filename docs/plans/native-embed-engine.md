# Native Embed Engine — Plan

Status: **Phases 1–2 landed, including 2D** (static is the only engine;
hashed/neural/rerank deleted; stale-model stores refuse and migrate on the
next `index` run; `self_gold_has_no_loss` 17/17 under the static default,
2026-10-14, and still 17/17 after 2D on 2026-10-15). Phase 3 first
candidate was measured on 2026-10-15 and rejected: it lost the frozen
suite, and the placeholder table was restored. **Phase 3 was then
cancelled outright by direction on 2026-10-17**: no training runs, no
distilled or fine-tuned tables, in any form. The quality path forward is
engine-side program logic only (see "Phase 5" below). All quality numbers cited here trace to
[`benchmarks/results/baselines.md`](../../benchmarks/results/baselines.md)
and [`benchmarks/results/static-engine-ab-2026-10-14.md`](../../benchmarks/results/static-engine-ab-2026-10-14.md),
using the frozen reconstruction suite in `benchmarks/frozen_suite/`.

## Why a built-in engine

Peer tools of the rented-side-model class rent a side model: server, first-run
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
- **No network, no cache directory, no model.** The lookup table is compiled
  into the binary with `include_bytes!`. asgrep does not ship a model, download
  a model, or load one at runtime. There is no model file, no ONNX runtime,
  and no sidecar. A teacher used offline to try a new table is not part of the
  program and must not be vendored.

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
pipeline. The remaining gap to the internal reference's home-suite numbers is
documented in the internal measurement ledger, not hidden.

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
- `self_gold_has_no_loss` under the static default: 5 misses → 2 → **0
  (17/17, 2026-10-14)**. The last two residuals
  (`conceptual-snapshot-consistency`, `conceptual-follow-up-derivation`)
  were closed by mechanism, not tuning: term-fair admission in the symbol
  pass, corpus IDF weighting in def scoring, whole-body parent chunks,
  and lift-only field rescoring (see Decision log). Zero gold edits.

## Phase 2 — remaining (approved)

Work items:

1. ~~Flip `EmbedPreference::Auto` to resolve to Static.~~ (2A, done.)
   ~~Remove `HashedEmbedder`, the concept-expansion path, the
   `semantic-v2` model identity, and the fastembed/ONNX neural path
   (`ASGREP_NEURAL_EMBED`) with its Cargo dependencies.~~ (2C, done;
   rerank went with it, fastembed left the tree entirely.)
2. ~~Index migration~~ (2B, done — implemented as full re-embed via the
   proven-complete rewrite path on identity mismatch, with an explicit
   log line and dead-model `embed_cache` purge; search under Auto
   refuses a stale-model store until then.)
3. ~~Ablate compensation scaffolding that only existed to contain hashed
   noise, one piece at a time, each gated on the frozen suite: thin-page
   fallback condition, conceptual weight shape, field-rescore heuristics.
   Keep whatever still earns its place with the learned backend.~~ (2D,
   done 2026-10-15.) Judgments: field rescoring kept but lift-only
   (`max(primary, mix)`); conceptual channel-weight shape kept after flat
   weights lost on both frozen datasets; thin-page condition removed after
   the paired frozen arm improved RepoQA and a failure-first fixture proved
   the old gate omitted semantic evidence from a full structural page.
   Details: `benchmarks/results/static-engine-ab-2026-10-14.md` and
   `docs/progress/surface-deferrals.md`.
4. Docs/contracts: `crates/ast-sgrep-embed/CONTRACT.md`,
   `docs/semantic-search.md`, `docs/fusion-ranking.md`. (2F)

Acceptance criteria:

- ~~`self_gold_has_no_loss` passes with the static backend~~ — done
  2026-10-14, 17/17 (`cargo test -p ast-sgrep-core --test
  identifier_ranking self_gold_has_no_loss -- --ignored`; test stays
  `#[ignore]` for its ~2 min runtime).
- ~~Frozen suite: no regression vs Phase-1 static numbers on either
  dataset~~ — done 2026-10-14 on the final tree (RepoQA MRR@10 **.521**,
  CoSQA **.240**), then superseded by the canonical 2D arm on 2026-10-15
  (RepoQA **.533**, CoSQA **.240**), with zero errors/timeouts. Paired
  arms, index-invocation time, and provenance are recorded in
  `benchmarks/results/baselines.md` and the detailed static-engine report.
- ~~Old-index behavior covered by a test~~ (2B, done).

Risks: first-run re-embed cost on large existing indexes; binary size step
(~+32 MB) becomes unconditional.

## Phase 3 — our own weights (offline distillation)

> **CANCELLED 2026-10-17 by user direction.** No training runs in any form;
> the MIT placeholder table is the permanent substrate. This section is
> retained as history. The capability delta a better table was meant to buy
> is pursued as engine-side program logic (Phase 5).

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

- **Field-aware pooling.** Gated 2026-10-15, not implemented. Reordering
  the returned semantic-only page by an alternate field mix lost to the
  engine's own score (RepoQA .629 vs .669, CoSQA .138 vs .166). Retry
  only with a full retrieval arm whose weights are frozen before the arm
  and that beats score-ordered semantic-only on both datasets.
- **Confidence-scaled fusion.** Gated 2026-10-15, not implemented. The
  1c vote is still `s / s_top`. Top-vs-second separation does not
  separate the datasets (median .041 vs .032), and a pre-declared 0.10
  source switch does not beat fused on both. Retry only with pre-fusion
  per-candidate channel ranks and a cut fixed before reading them.
- **Best-cosine rank floor.** Gated 2026-10-15, not implemented. Pinning
  the best cosine to rank 1 scores .654 / .211 and loses CoSQA.
- **Cosine-support demotion.** Kept 2026-10-15. A hit below
  `1 - DECISIVE_MARGIN_RATIO` of the best embed score loses non-embed
  votes; an embed-absent hit does not. Frozen suite, no reindex:
  RepoQA .571 → .637, CoSQA .240 → .317, Hit@100 unchanged on both.
  The stricter rule that also zeroed embed-absent hits failed CoSQA
  Hit@100 and was not shipped.
- **ANN/IVF revisit** — only if corpus sizes make brute-force cosine the
  bottleneck *and* the retry-condition predicates in
  `docs/progress/perf-negative-results.md` are satisfied. Brute force at
  current scale is a few ms; do not pre-build this.

## Phase 5 — engine-only quality campaign (2026-10-17, active)

Direction: ast-sgrep ships no model and trains nothing. Whatever a better
table was meant to contribute is built as deterministic program logic in
the engine backend. Every lever is gated before it touches the tree:
failure-first fixture, paired frozen arms (suite is judge, never tuning
signal), and the 120-query holdout v1 (`benchmarks/phase3_holdout/`,
placeholder bar MRR@10 .531) as the independent judge for any
quality-affecting engine change. Current bars: frozen worktree arm
.719/.317 (lever-1 section in `benchmarks/results/baselines.md`) vs
the internal reference .757/.417 (internal measurement ledger).

Stage 1 (2026-10-17): `ASGREP_FUSION_DUMP` capture in
`crates/ast-sgrep-core/src/fusion.rs` (env-gated, one JSONL line per
fusion invocation, per-result pre-fusion channel ranks) — the record the
rejected confidence-scaled-fusion and rank-floor levers' retry predicates
demand — plus CoSQA loss attribution over the frozen 500
(`benchmarks/frozen_suite/fusion_capture.py`). Lever order after diagnosis:
query decomposition, margin-rule calibration, pseudo-relevance feedback,
per-language structure policy (cpp holdout .219), field-aware pooling arm
(existing predicate).

Stage 1 verdict (2026-10-17, evidence in `benchmarks/results/baselines.md`
"engine-only campaign Stage 1" section): CoSQA misses are 0% admission,
34% fusion vote-stacking, 66% semantic-depth. No winner is semantic-absent.
RepoQA is healthy (12 true admission misses of 43).

**L-M1 — corroboration vote cap (pre-declared 2026-10-17, β = 1.0).**
Rejected pre-code: the stacking class was a classification artifact —
in 48/54 stacking-buried CoSQA misses the winner's own semantic rank is
BETTER than gold's (med −9), so corroboration rides winners rather than
manufacturing them; the cap would bite all 54 mostly to demote deserved
wins. Full entry with retry predicate in the conformance ledger. The
fixture also showed the pre-declared "must not lose" contract encoded
the already-rejected hard-pin (any near-rank rival survives via the
rank-independent magnitude vote) — arithmetic caught it before code.

**L-Q1c — short-question rewrite (2026-10-17): KEPT.** Offline genealogy
(same frozen probe lists, scratch `lq1-lists.json`): uniform multi-probe
RRF (orig + boilerplate-strip) +.0174 CoSQA / −.0031 RepoQA — rejected,
trades datasets; conditioned RRF fusion +.0076 / 0; conditioned REPLACE
+.0102 / 0 — implemented. `rewrite_short_question` in `query.rs` strips
recognized scaffolding (leading "how to / what('s| is) … way [to] /
is there a way to", trailing "in|using|with|for python") from the raw
query before any lane sees it, with the frozen sim stop list;
mode-prefixed queries are never rewritten (byte-fidelity); all-scaffolding
queries keep the original. Fixture `tests/core/short_question_rewrite.rs`
(red → green, mutant-killed). Paired frozen arm (binary `94a93997…` vs
control `63cdc851…`, same tree minus the rewrite): RepoQA bit-identical
(0/600 rank deltas; .7194 / .928 / .972 unchanged), CoSQA .3169 → .3271
(+.0102), Hit@1 .226 → .234, Hit@10 .574 → .588, Hit@100 .884 → .880
(net −2 queries — the lever's only cost), 0 errors; the engine arm
reproduced the sim exactly. Holdout v1: 0/120 queries match the pattern
— cannot fire, no regression by construction; current-tree holdout
refresh MRR@10 .5654 (supersedes the .531 row via tree movement, not
lever effect).

**Semantic-depth status (post L-Q1c, 2026-10-17): CLOSED for program
logic.** Every remaining axis was measured and is negative — PRF is dead
on arrival (the semantic top hits ARE the distractors); corpus-structure
features are coin flips; and the field-combination probe (capture
`bench-fields-capture/`) shows the winner beating gold under
max-over-fields in 113/133 in-pool misses with gold's best field = body
(108/133, docs 0) — the table's preference is consistent across pooled,
per-field, max, and sum views, so no combination rule separates them.
The CoSQA depth tail is static-table geometry; only a new table (excluded
by direction) changes it. Open pointers with predicates:
leading-language strip (r4, ceiling ≈ +.005) and the limit-coupled
rescore-pool floor (measured ceiling ≈ +.001, design wart: quality
varies with requested page size) — see
`docs/progress/conformance-negative-results.md`.

**Geometry-wall reframe review (2026-10-17, same day): all three
"another way to think about it" reframes measured, all negative.**
(1) No alternate license-clean static table exists — HF-wide audit, 8
model2vec+code models, potion-code-16M-v2 is the best in existence.
(2) No top-k set structure to exploit — misses and successes have
identical top-10 mutual similarity; MMR best case +.0019.
(3) No synonym geometry in the table (json·serialize = .15) and query-
side ADDITION of any kind is catastrophic: L-Q3 (repo-PPMI expansion on
the embed query, env `ASGREP_QEMBED_EXPANDED=1`) measured −.1502 CoSQA /
−.0467 RepoQA in a paired arm. Durable synthesis: under mean-pooled
static queries the only query-side lever class that can win is DELETION
(L-Q1c, kept); addition has no margin. The plateau stands as a
measured property of the pinned table, not of missing program logic.

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
- 2026-10-14: gold residuals closed by four mechanism fixes, each with a
  failure-first fixture in `tests/core/identifier_ranking.rs` /
  `field_weight_unit.rs`: term-fair admission (per-term quota + round-robin
  fill, `search/passes/symbol.rs`), corpus IDF in def scoring
  (`rank.rs::term_idf`, Robertson BM25 idf from trigram df), whole-body
  parent chunks beside pattern fragments (`semantic_chunk.rs`), lift-only
  field rescore (`search/field_weight.rs`). No weights were tuned against
  the gold; the gold file is untouched.
- 2026-10-15: Phase 3 second candidate rejected. Fine-tuning the
  placeholder itself on name-stripped commit subjects, held out by
  entire repo, left holdout MRR@10 at .273. The rejected Qwen table
  scores .253 on that holdout. No suite run. Placeholder stays.
- 2026-10-15: Phase 3 first candidate rejected. Distilling
  `Qwen/Qwen3-Embedding-0.6B` into the existing vocab and contrastively
  fine-tuning on docstring/comment pairs raised held-out pair MRR@10 from
  .523 to .571, then lost the frozen suite (RepoQA .449 vs .533, CoSQA
  .233 vs .240, every language down). Placeholder table restored. The
  suite was not a training signal. Details:
  `benchmarks/results/static-engine-ab-2026-10-14.md`.
- 2026-10-15: path words in the primary embed text rejected. Fresh
  indexes, release binary: RepoQA .701 vs .698, CoSQA unchanged at
  .317, java .652 vs .681. Text change and recipe stamp removed.
  Retry only if a pre-declared arm beats both datasets with no
  language drop. Details in the static-engine results doc.
- 2026-10-15: 2D closed by paired frozen-suite ablations, not by retuning:
  flat conceptual weights lost (.509/.201 vs shaped .521/.240), so the
  shape stays; deleting only the thin-page condition improved RepoQA to
  .533 with CoSQA rank-identical, and
  `full_structural_page_still_considers_semantic_evidence` failed under the
  restored gate before passing after deletion.
- 2026-10-15: confidence-scaled fusion rejected (separation does not
  separate the datasets; no fusion code change). English `;` in a
  paragraph was classifying the query as structural and coverage-sorting
  it; fixing that, with `foo();` still structural, moved fused RepoQA
  .533 → .571 with CoSQA and the 435 non-semicolon RepoQA ranks
  identical. Details in the results doc.

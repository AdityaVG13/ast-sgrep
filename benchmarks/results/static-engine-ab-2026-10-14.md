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

`self_gold_has_no_loss` with the static default had **2/17 misses** at
this checkpoint (was 5/17 hashed). Those residuals were subsequently
closed by general retrieval mechanisms and re-verified at 17/17; see the
final-tree section below.

## Phase 2 final-tree verification (2026-10-14)

The final Phase 2 mechanisms changed index content and ranking after the
2A arm: whole-body parent semantic chunks, schema-17 rebuild of legacy
fragment-only chunk layouts, term-fair symbol admission, corpus-IDF
def/caller weighting, and lift-only field rescoring. The frozen suite was
therefore rerun from freshly built indexes rather than treating .494/.197
as evidence for the shipped tree.

Command (prepared frozen corpus; release binary built with
`cargo build --release -p ast-sgrep-cli`):

```bash
env -u ASGREP_EMBED_BACKEND -u ASGREP_NEURAL_EMBED \
  python3 benchmarks/sgrep_suite/run_asgrep_arm.py \
  --bin /Users/aditya/Developer/ast-sgrep/target/release/asgrep \
  --bench-dir /Users/aditya/.hermes/cache/scratch/sgrep-bench/bench-final \
  --out-dir /Users/aditya/.hermes/cache/scratch/sgrep-bench/bench-final/out \
  --expect-backend static
```

Candidate provenance: HEAD
`56492b0e38d09420c62ed00c28d6e2afe0d71db3` plus the uncommitted Phase 2
worktree; binary SHA-256
`054f5dfb542db667d9bd7ccd5817ddbf25f2bdb6f55caa9086ef3d339eeda96d`;
Apple M5 Max, macOS 26.6.2. Stored backend asserted `static`.

| Dataset | Phase 2A MRR@10 | Final MRR@10 | Final Hit@1 / @10 / @100 | Final p50 / p95 |
|---|---:|---:|---:|---:|
| RepoQA-600 | .494 | **.521** | .370 / .855 / .942 | 22.9 / 48.5 ms |
| CoSQA-500 | .197 | **.240** | .162 / .466 / .884 | 19.0 / 21.1 ms |

Errors and timeouts: zero on both datasets. Every RepoQA language's
MRR@10 was at or above the 2A arm (cpp .431, go .504, java .488,
python .517, rust .568, typescript .616); Java Hit@10 moved .77 → .76
while its MRR rose slightly. Aggregate indexing time was 91.1 s across
61 roots. The resulting 61 `index.db` files total 3,089,219,584 bytes;
including IVF and other `.asgrep` sidecars present after retrieval, the
payload is 3,354,782,939 bytes.

Interpretation is bounded: this proves no regression—and, on this run, a
quality gain—relative to the Phase 2A static arm. It is not a new
competitor comparison and not a release certificate. Latency and storage
costs are recorded above rather than hidden by the quality result.

## Phase 2D ablation verification (2026-10-15)

Phase 2D judged the two remaining hashed-era compensation levers one at a
time on the final-tree frozen indexes. The retrieval-only control reproduced
all 1,100 final-tree ranks above exactly (RepoQA MRR@10 .520545, CoSQA
.239806), so paired deltas below attribute to the named lever rather than
index drift.

### Conceptual channel-weight shape — kept

Flattening every conceptual channel weight to 1.0 through
`ASGREP_INTENT_WEIGHTS` lost on both datasets: RepoQA .509268 vs control
.520545 (39 reciprocal-rank gains / 95 losses among changed ranks) and
CoSQA .201453 vs .239806 (17 / 119). No production weight changed.

### Thin-page fallback condition — removed

The old gate skipped the known-files restricted embed pass on
identifier/literal/structural hybrid queries whenever structural hits
already filled the requested page (`hits.len() >= limit`). Removing only
that condition scored RepoQA .533002 vs .520545 (38 reciprocal-rank gains /
41 losses; the gains were larger in magnitude) and left all 500 CoSQA ranks
byte-identical. In that single sequential retrieval-only comparison, p50
moved RepoQA 24.9 → 26.3 ms and CoSQA 23.4 → 22.8 ms; those timings are an
observed cost signal, not a latency certificate.

Failure-first pin:
`tests/core/identifier_ranking.rs::full_structural_page_still_considers_semantic_evidence`.
With the old gate restored it failed for the named reason — on a full page
the target's contributors were `[Asgrep, Def]`, with no `Embed` evidence.
After deleting the condition, the same fixture passed on cold and warm
lanes. The full `identifier_ranking` file passed 21/21 non-ignored tests,
and `self_gold_has_no_loss` passed 17/17 under the change.

Canonical confirmation used the checked-in driver after rebuilding the
release binary:

```bash
cargo build --release -p ast-sgrep-cli
python3 benchmarks/sgrep_suite/run_asgrep_arm.py \
  --bin /Users/aditya/Developer/ast-sgrep/target/release/asgrep \
  --bench-dir /Users/aditya/.hermes/cache/scratch/sgrep-bench \
  --out-dir /Users/aditya/.hermes/cache/scratch/sgrep-bench/bench-2d \
  --expect-backend static
```

Candidate provenance: HEAD
`56492b0e38d09420c62ed00c28d6e2afe0d71db3` plus the uncommitted Phase 2/2D
worktree; binary SHA-256
`bc8b0a34c076d22c6e2186012ddb2427bc0874c4daf6fe5084c92707bd911d0b`;
Apple M5 Max, macOS 26.6.2. The canonical ranks matched the paired T1 arm
on all 1,100 queries.

| Dataset | Pre-2D MRR@10 | 2D MRR@10 | 2D Hit@1 / @10 / @100 | 2D p50 / p95 |
|---|---:|---:|---:|---:|
| RepoQA-600 | .521 | **.533** | .377 / .868 / .947 | 26.2 / 48.7 ms |
| CoSQA-500 | .240 | **.240** | .162 / .466 / .884 | 21.8 / 24.1 ms |

Errors and timeouts were zero. RepoQA MRR@10 by language: cpp .471,
go .507, java .497, python .522, rust .581, typescript .620. The driver's
61 per-root `index` invocations totaled 253.9 s. Because the kept 2D change
is query-time only, this arm does not establish fresh-index storage bytes;
the final-tree fresh-index totals above remain the storage record.

## Phase 3 candidate — rejected (2026-10-15)

Not canonical. The placeholder table remains the shipped weights. One
candidate, one suite run, no retune after seeing these numbers.

Teacher `Qwen/Qwen3-Embedding-0.6B` (Apache-2.0), last-token distill into
the existing 63,457-token vocab, PCA 1024→256, then InfoNCE fine-tune on
docstring/comment pairs from 38 permissive repos. Frozen-suite queries and
gold spans were excluded (2 pairs dropped). Held-out pair MRR@10 rose from
.523 to .571. That holdout is the same task as training (a function's own
docstring against that function), not the suite task.

```bash
cargo build --release -p ast-sgrep-cli
python3 benchmarks/sgrep_suite/run_asgrep_arm.py \
  --bin /Users/aditya/Developer/ast-sgrep/target/release/asgrep \
  --bench-dir /Users/aditya/.hermes/cache/scratch/sgrep-bench \
  --out-dir /Users/aditya/.hermes/cache/scratch/sgrep-bench/bench-phase3 \
  --expect-backend static
```

The arm's stored model, read back from an indexed corpus after the run,
was `static:asgrep-code-256-v1:256`. Candidate binary SHA-256
`23355029090c9b540458cb43ed29bcb9fd4f911d15ef9a4ddfddd205e2763a3a`.
Weight sha256
`5a025c02c1f31fe8f8fb6619e3a9d4d6304a560286ad6f49c2703350496f9246`.
61 index invocations, 270.3 s. Errors and timeouts: zero.

| Dataset | 2D placeholder | Phase 3 candidate | Candidate Hit@1 / @10 / @100 |
|---|---:|---:|---:|
| RepoQA-600 | .533 | .449 | .305 / .780 / .910 |
| CoSQA-500 | .240 | .233 | .148 / .470 / .782 |

RepoQA MRR@10 by language, candidate vs 2D: cpp .392 vs .471, go .435 vs
.507, java .365 vs .497, python .463 vs .522, rust .537 vs .581,
typescript .504 vs .620. Every language dropped. The placeholder bytes
and model identity were restored; the rebuilt release binary matches the
2D binary SHA-256
`bc8b0a34c076d22c6e2186012ddb2427bc0874c4daf6fe5084c92707bd911d0b`.

## Phase 4 gate — confidence-scaled fusion rejected (2026-10-15)

Not a code change. Semantic-only on the potion indexes (retrieval-only,
`ASGREP_SEMANTIC_ONLY=1`, no reindex, binary the 2D SHA above) scored
RepoQA MRR@10 **.617** and CoSQA **.166**, against fused 2D **.533** /
**.240**. The gap is real and larger than the pre-2D .494 vs .522 note.
Top-vs-second embed-score separation does not tell the two datasets
apart (median gap2 .041 RepoQA, .032 CoSQA). A pre-declared switch at
the engine's existing 0.10 decisive-margin ratio, using semantic rank
when separated and fused rank otherwise, scored .569 / .236. That is an
upper bound on a query-level source switch, not a fusion formula, and
it does not beat fused on both datasets. No fusion code changed.
Records: `~/.hermes/cache/scratch/sgrep-bench/bench-sem-current/`.

## English-semicolon intent fix (2026-10-15)

Kept. `looks_structural` treated any `;` as a code pattern, so a
paragraph ("raise an exception; if so") was Structural. Finish then
sorted those queries by excerpt-term coverage before score, and fusion
used flat channel weights instead of the conceptual shape. All 165
RepoQA queries containing `;` are prose (54–253 tokens). The other 435
RepoQA queries and all 500 CoSQA queries contain no `;`.

Failure-first: `intent_classify_weights_routing_and_scoring` failed
with Structural vs Conceptual before the fix, passed after, and failed
again when `;` was put back in the marker list. The marker list was
then restored to the fix. `foo();` stays structural; a parenthetical
and a backtick clause do not.

Retrieval-only arm, no reindex, potion indexes
(`static:potion-code-16M-v2:256`), binary SHA-256
`4c41dca986a0ef8685e3f485b30316971f5232f5c638551ec4bea85f78d7343f`.
HEAD `56492b0e38d09420c62ed00c28d6e2afe0d71db3` plus this uncommitted
worktree. Apple M5 Max. Errors and timeouts: zero.

| Dataset | 2D MRR@10 | After fix | Hit@1 / @10 / @100 | p50 |
|---|---:|---:|---:|---:|
| RepoQA-600 | .533 | **.571** | .417 / .898 / .968 | 25.2 ms |
| CoSQA-500 | .240 | **.240** | .162 / .466 / .884 | 22.9 ms |

CoSQA ranks are identical on all 500 queries. RepoQA ranks are identical
on the 435 queries without `;`. The 165 semicolon queries moved .410 →
.550 (83 reciprocal-rank gains / 42 losses; gain 41.17, loss 18.08).
RepoQA by language: cpp .503, go .522, java .559, python .572, rust
.605, typescript .669. Every language is above its 2D figure. p50 is
one sequential comparison, not a latency certificate.

A page-local field-weight reorder of the semantic-only hits did not
beat ordering those same hits by the engine's own score (RepoQA mix
.629 vs score .669; CoSQA mix .138 vs score .166). Field weights were
not changed.

## Fusion still trails embed-only after the semicolon fix (2026-10-15)

Paired rerun on this binary, same potion indexes, no reindex. Semantic-only
(`ASGREP_SEMANTIC_ONLY=1`) is the control. Binary SHA-256
`4c41dca986a0ef8685e3f485b30316971f5232f5c638551ec4bea85f78d7343f`.
The earlier .617 figure was the pre-fix binary and is superseded.

| Dataset | Fused | Semantic-only | Sem Hit@1 / @10 / @100 |
|---|---:|---:|---:|
| RepoQA-600 | .571 | **.661** | .510 / .938 / .988 |
| CoSQA-500 | **.240** | .166 | .102 / .328 / .806 |

RepoQA semantic-only by language: cpp .608, go .693, java .618, python
.718, rust .662, typescript .669. The fused gap is concentrated in go
(.171) and python (.146). Of 144 RepoQA queries where the best cosine
file is rank 1 under semantic-only and not under fusion, the fused
winner's cosine is a median 0.82 of the gold's. The extra channels are
not tie-breaking; they are promoting a worse cosine.

A hard floor that pins the best cosine to rank 1, leaving every other
fused rank in place, scores RepoQA .654 and CoSQA .211. That beats
fused RepoQA and loses CoSQA by .029. Rejected. Not a fusion code
change. Records: `~/.hermes/cache/scratch/sgrep-bench/bench-sem-clause/`.

The test-path penalty is not the aggregate cause. Eight of those 144
golds sit on a path `is_test_path` marks, and one of those is a false
positive (`internal/test/assertions.go`, a Go package, not a test
file). Nineteen of 100 Go golds are themselves `*_test.go`, so widening
the penalty to that suffix would bury golds semantic-only ranks well.
No test-path code changed.

## Cosine-support demotion kept (2026-10-15)

A hit whose embed score is below 0.90 of the query's best embed score
loses its non-embed channel votes. The cut is `1 - DECISIVE_MARGIN_RATIO`
(0.10), frozen before the capture, not fitted to the 0.82 median. A hit
the embed channel never scored keeps its other votes. Zeroing those
was a separate rule and failed: pre-critic CoSQA Hit@100 .870 to .862,
all four drops embed-absent. That stricter rule was not shipped.

Retrieval-only, no reindex, potion indexes
(`static:potion-code-16M-v2:256`). Shipping binary SHA-256
`a8d454408a3a363eed8b34cd9457116a9a51aac17ae7719b965ad9c786bda07d`.
The env-gated arm and this default binary agree on all 1,100 ranks.
Errors and timeouts: zero. p50 is one sequential comparison, not a
latency certificate (RepoQA 25.8 ms, CoSQA 22.5 ms).

| Dataset | Fused control | After demotion | Hit@1 / @10 / @100 |
|---|---:|---:|---:|
| RepoQA-600 | .571 | **.637** | .513 / .900 / .968 |
| CoSQA-500 | .240 | **.317** | .226 / .574 / .884 |

Hit@100 is unchanged on both (.968 / .884). Reciprocal-rank moves at
@10: RepoQA 149 gains / 41 losses (gain 48.32, loss 8.87); CoSQA 170
gains / 20 losses (gain 43.72, loss 5.18). RepoQA by language: cpp
.550, go .609, java .615, python .629, rust .668, typescript .753.
Every language is above the fused control. Records:
`~/.hermes/cache/scratch/sgrep-bench/bench-fusion-ship/`.

## Description clamp kept (2026-10-15)

The 0.90 fusion cut drops extra-channel votes, then the critic puts a
below-margin hit back above the best cosine. On a description — a query
with a sentence boundary (newline or `". "`) — those hits are clamped
under the best embed after the critic. A phrase is not a description.
An unconditional clamp of the same shape scored CoSQA .263 on a replay
of the ship records and was not shipped.

Retrieval-only, no reindex, same potion indexes. Debug binary SHA-256
`22a9c7012ce846d17fe49421a88954de23e3d8ae504abbe699cf1d557aa57d0e`,
HEAD `56492b0e` plus this uncommitted worktree. CoSQA ranks match the
shipping binary on all 500 queries, so the opt-level difference did
not move ranks. Errors and timeouts: zero. p50 is one sequential
comparison, not a latency certificate (RepoQA 57.8 ms, CoSQA 55.2 ms,
debug binary).

| Dataset | After demotion | After description clamp | Hit@1 / @10 / @100 |
|---|---:|---:|---:|
| RepoQA-600 | .637 | **.667** | .552 / .908 / .968 |
| CoSQA-500 | .317 | **.317** | .226 / .574 / .884 |

Hit@100 is unchanged on both (.968 / .884). CoSQA ranks are identical
on all 500 queries. RepoQA: 88 reciprocal-rank gains / 43 losses at
@10; 467 of 600 ranks unchanged. By language: cpp .520 (-.029), go
.688, java .656, python .686, rust .679, typescript .774. cpp is the
only language that dropped, and it stays above semantic-only on this
index (.511). Aggregate RepoQA now beats that semantic-only arm
(.661). Records:
`~/.hermes/cache/scratch/sgrep-bench/bench-desc-clamp/`.

## Embed-absent cap on descriptions kept (2026-10-15)

The description clamp left embed-absent defs alone. On a description
those defs were the remaining hits above the best cosine. Capping
their score under the best embed is not the rejected zeroing rule:
the hit stays on the page, and a phrase query is unchanged.

Retrieval-only, no reindex, same potion indexes. Debug binary SHA-256
`c6ba76cf93f3df94db948374192c1278c5b63674c1679e9c4e1f390333b7cf63`,
HEAD `56492b0e` plus this uncommitted worktree. Errors and timeouts:
zero. p50 is one sequential comparison, not a latency certificate
(RepoQA 64.5 ms, CoSQA 60.0 ms, debug binary).

| Dataset | Description clamp | After embed-absent cap | Hit@1 / @10 / @100 |
|---|---:|---:|---:|
| RepoQA-600 | .667 | **.698** | .588 / .912 / .968 |
| CoSQA-500 | .317 | **.317** | .226 / .574 / .884 |

Hit@100 is unchanged on both. CoSQA ranks are identical on all 500
queries. RepoQA: 45 reciprocal-rank gains / 1 loss at @10
(`python-locustio__locust-01`, rank 2 to 4); 554 of 600 ranks
unchanged. By language, all six up: cpp .547, go .749, java .681,
python .745, rust .681, typescript .782. Python and go now beat
semantic-only on this index (.718 and .693). Records:
`~/.hermes/cache/scratch/sgrep-bench/bench-desc-absent/`.

## Phase 3 second candidate rejected (2026-10-15)

Not a weight swap. The placeholder table stays. A fine-tune of that
table on name-stripped commit subjects, held out by entire repo, did
not move the holdout (MRR@10 .2733 to .2734, 599 pairs). The rejected
Qwen table scores .253 on the same holdout. Neither candidate was
allowed onto the frozen suite. A fused-page reorder that promotes the
stronger field max, replayed on the description-clamp records, scores
RepoQA .677 against .698 and drops five of six languages. Not shipped.

## Path words in the primary vector rejected (2026-10-15)

Not shipped. Letter-bearing path tokens prepended to the primary embed
text, with a recipe stamp so an unchanged index re-embeds. Release
binary, fresh indexes, potion table (`static:potion-code-16M-v2:256`).
Errors and timeouts: zero. Indexing 105.4 s across the 61 scopes.
Records: `~/.hermes/cache/scratch/sgrep-bench/bench-path-words/out/`.

| Dataset | Kept arm | Path words | Hit@1 / @10 / @100 |
|---|---:|---:|---:|
| RepoQA-600 | .698 | .701 | .588 / .923 / .968 |
| CoSQA-500 | .317 | .317 | .226 / .574 / .884 |

CoSQA did not move. RepoQA Hit@1 and Hit@100 did not move. By language:
cpp .560, go .761, java .652, python .754, rust .707, typescript .773.
Java dropped .029 and typescript dropped .009 against the kept arm
(.681 / .782). The text change and the recipe stamp were removed.



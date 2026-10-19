# Baselines

> **Ledger mix:** section tags below (`canonical` / `historical` /
> `UNREPRODUCIBLE` / `reproducible-in-tree`). Vocabulary:
> [`benchmarks/README.md`](../README.md). A section tagged
> `UNREPRODUCIBLE` cannot be regenerated from this tree. The latest release
> comparison fingerprint is the 2026-10-19 paired benchmark section; historical
> sections below are their own dated fingerprints and do not certify the
> current candidate.

> External artifact location (historical dumps, not proof of regeneration):
> [Speed benchmark workflow](https://github.com/AdityaVG13/ast-sgrep/actions/workflows/speed.yml).

Single source of truth for every MRR / recall / latency claim in this
repository. Any number quoted in docs, commit messages, or bead close reasons
must trace back to a row here or carry its own reproduce command. Each dated
fingerprint owns its machine, corpus, configuration and sample-count limits.

## Engine-only campaign Stage 1: fusion capture + CoSQA loss attribution (2026-10-17)

**Status: measured on the uncommitted worktree, reproducible-in-tree after
frozen corpus preparation; not a release certificate.** Binary
`63cdc851afce2b96478d782b79ce0001ded256279265534453437151b9bbdff2`
(includes the env-gated `ASGREP_FUSION_DUMP` capture,
`tests/core/fusion_capture.rs`). Harness:
`benchmarks/sgrep_suite/fusion_capture.py`; full rows and dump under the
scratch bench root in `bench-fusion-capture/` (capture on) and
`bench-fusion-control/` (capture off). Capture proven rank-neutral: 0/1100
rank mismatches capture-vs-control AND capture-vs-`bench-lever1-gate` rows.
The current tree (including the morning's `intent.rs` edit) reconfirms the
lever-1 arm exactly: RepoQA .719 / CoSQA .317 MRR@10, 0 errors/timeouts.

Loss attribution over the frozen 1,100 (miss = rank > 10 or absent):

| dataset | misses | upstream admission | fusion-buried (sem rank < final) | of which: semantic-led winner | of which: vote-stacking winner |
|---|---:|---:|---:|---:|---:|
| CoSQA-500 | 213 | 0 | 125 | 71 | 54 |
| RepoQA-600 | 43 | 12 | 7 | — | — |

- CoSQA gold's best channel is semantic in 133/213 misses; structural
  channels are nearly absent on gold (lexical present 18/213). Gold's
  semantic rank when present: median 13 over all misses, median 9 over the
  fusion-buried subset (p25 3, p75 19).
- **No winner is semantic-absent** (0/125): the fusion losses are vote
  STACKING — a winner with comparable-or-better cosine plus multi-channel
  corroboration outranks a semantic-only gold — not structure outvoting
  semantics. 71 winners simply have the better cosine (semantic-depth
  problem, P2); 54 are stacking (P1).
- P2 budget: 159/213 CoSQA misses are semantic-depth (the cosine itself
  prefers distractors) — query-side semantic shaping territory; this is
  where most of the .100 CoSQA gap to sgrep (.417) lives.

Refinement (same day, per-candidate winner ranks): the 54 "stacking"
cases are mostly an artifact — 48/54 winners hold the BETTER semantic
rank themselves (med −9); corroboration rides winners, it does not
manufacture them. L-M1 (corroboration vote cap) was rejected pre-code on
this evidence; see the conformance ledger entry with retry predicate.

## Engine-only campaign: L-Q1c short-question rewrite — KEPT (2026-10-17)

Binary `94a93997bed9b14e6a7c3cdc2fc28ecd29a7b943700c6aeb1bbe69f29e468480`
(uncommitted worktree, includes the Stage-1 fusion capture). Paired frozen
arm vs control `63cdc851…` (same tree minus the rewrite):

| dataset | MRR@10 | Hit@1 | Hit@10 | Hit@100 | errors |
|---|---|---|---|---|---|
| RepoQA ctl | .7194 | .608 | .928 | .972 | 0 |
| RepoQA arm | **.7194 — bit-identical, 0/600 rank deltas** | .608 | .928 | .972 | 0 |
| CoSQA ctl | .3169 | .226 | .574 | .884 | 0 |
| CoSQA arm | **.3271 (+.0102)** | .234 | .588 (+10/−3) | .880 (+2/−4) | 0 |

Harness `benchmarks/sgrep_suite/fusion_capture.py --no-capture`; rows
`bench-lq1-arm/` vs `bench-fusion-capture/` (control), scratch bench root.
Mechanism: `rewrite_short_question` (`query.rs`) strips recognized
short-question scaffolding (leading "how to / what('s| is) … way [to] /
is there a way to", trailing "in|using|with|for python") from the raw
query before any lane sees it, with the frozen sim stop list;
mode-prefixed queries are never rewritten (pattern byte-fidelity);
all-scaffolding queries keep the original. Fixture
`tests/core/short_question_rewrite.rs` (red → green, mutant-killed).
Offline genealogy on the same frozen probe lists (scratch `lq1-lists.json`,
`lq1_sim.py`): uniform multi-probe RRF +.0174 CoSQA / −.0031 RepoQA
(rejected — trades datasets); conditioned RRF fusion +.0076 / 0;
conditioned replace +.0102 / 0 — implemented; the engine arm reproduced
the sim exactly.

Holdout v1 (engine-candidate gate): 0/120 queries match the rewrite
pattern — the lever cannot fire; no regression by construction.
Current-tree holdout refresh: **MRR@10 .5654**, Hit@1 .458, Hit@10 .817,
Hit@100 .933, 0 errors (`bench-holdout-lq1-arm/summary.json`). Supersedes
the recorded .531 placeholder row via intervening tree movement
(boilerplate strip, cosine support, and other keeps since that binary) —
identical ranks by construction, not a lever effect.

## Phase 2D static-engine frozen suite (2026-10-15)

**Status: measured, reproducible-in-tree after frozen corpus preparation;
local uncommitted candidate, not a release certificate.** Detailed record:
[`static-engine-ab-2026-10-14.md`](static-engine-ab-2026-10-14.md#phase-2d-ablation-verification-2026-10-15).
Candidate: HEAD `56492b0e38d09420c62ed00c28d6e2afe0d71db3` plus the Phase
2/2D worktree (binary SHA-256
`bc8b0a34c076d22c6e2186012ddb2427bc0874c4daf6fe5084c92707bd911d0b`), run on
an Apple M5 Max under macOS 26.6.2. The stored backend was asserted as
`static` by `benchmarks/sgrep_suite/run_asgrep_arm.py`.

| Dataset | MRR@10 | Hit@1 | Hit@10 | Hit@100 | p50 |
|---|---:|---:|---:|---:|---:|
| RepoQA-600 | .533 | .377 | .868 | .947 | 26.2 ms |
| CoSQA-500 | .240 | .162 | .466 | .884 | 21.8 ms |

This canonical 2D arm supersedes the Phase 2 final-tree arm (.521 / .240)
after removal of the thin-page condition. The conceptual channel-weight
shape was retained: the paired flat-weight arm scored .509 / .201 against a
control that reproduced the .521 / .240 ranks exactly. Errors and timeouts
were zero on both datasets. The arm's 61 per-root `index` invocations took
253.9 s in aggregate. Index-content bytes were not re-baselined here because
the 2D keep is query-time only; the prior fresh-index totals remain in the
detailed record. This does **not** supersede the competitor comparison
below: sgrep was not rerun.

## br-dx0 lever-1 boilerplate strip (2026-10-17) — measured, not canonical

**Status: measured on the uncommitted worktree, not canonical.** Query-time
only (`tokenize_for_scoring` drops ~45 genre-boilerplate terms for queries
over 15 tokens). Paired arms on the frozen 600/500 sets via
`benchmarks/sgrep_suite/run_asgrep_arm.py`, Apple M5 Max, backend `static`,
zero errors/timeouts both arms. Full rows:
`bench-control/` (control) and `bench-lever1-gate/` (lever 1) under the
scratch bench root.

| Arm | Binary SHA-256 | RepoQA-600 | CoSQA-500 |
|---|---|---:|---:|
| Worktree control (no lever 1) | `823c2437…422c1` | .698 / .588 / .912 | .317 / .226 / .574 |
| Worktree + lever 1 | `d366a635…7038` | .719 / .608 / .928 | .317 / .226 / .574 |

Cells are MRR@10 / Hit@1 / Hit@10. Lever-1 delta: **+.022 RepoQA MRR**,
CoSQA bit-identical (the length threshold never fires on short CoSQA
queries). Per-language MRR control -> lever 1: cpp .547 -> .613,
go .749 -> .759, java .681 -> .682, python .745 -> .755, rust .681 -> .727,
typescript .782 -> .780 — no collapse (typescript -.002 is noise). Note: both
arms score far above the 2D canonical row (.533 / .240) measured 2026-10-15
on an earlier state of this worktree — the intervening tree movement, not
lever 1, owns that gap; this section does not supersede the canonical row.

## Round-2 lever R2-1: no-embed hybrid merges the full lexical pass (2026-10-17)

**Status: measured on the uncommitted worktree (includes lever 1), not
canonical.** Query-time only (`search_hybrid` extends the hit set with
`lexical_pass` when `!use_embed`; same-line duplicates merge by max score
in finish dedup; embed-on path unreachable by construction). Paired arms on
the frozen 600/500 sets via `benchmarks/sgrep_suite/run_asgrep_arm.py` with
`ASGREP_NO_EMBED=1`, Apple M5 Max, backend `<none>`, zero
errors/timeouts both arms. Full rows: `bench-noembed/out/` (base) and
`nocascade-fix/` (fix) under the scratch bench root.

| Arm | Binary SHA-256 | RepoQA-600 | CoSQA-500 |
|---|---|---:|---:|
| Worktree, no-embed (base) | prior arm | .318 / .177 / .683 | .100 / .038 / .244 |
| Worktree + R2-1, no-embed | `bb44013a…560069` | .337 / .188 / .715 | .108 / .038 / .284 |

Cells are MRR@10 / Hit@1 / Hit@10. R2-1 delta: **+.020 RepoQA MRR, +.008
CoSQA MRR**. The signature is admission recovery: RepoQA Hit@100
.820 -> .908, CoSQA Hit@100 .350 -> .722 — the cascade's per-term caps plus
flat prefilter scores stranded lexical evidence past the page; the merge
restores it. Latency cost of the extra FTS pass: p50 19.7 -> 23.4ms
(+19%), p95 44.7 -> 45.7ms. Failure-first test
`noembed_hybrid_keeps_lexical_gold_past_prefilter_caps`
(`tests/core/cascade_planner.rs`); flipped-condition mutant killed.

## Phase 3 own-weights candidate (2026-10-15) — rejected

**Status: measured, not canonical.** The shipped table is still the 2D
placeholder. Full record:
[`static-engine-ab-2026-10-14.md`](static-engine-ab-2026-10-14.md#phase-3-candidate--rejected-2026-10-15).
One suite run of a Qwen3-Embedding-0.6B distillation, after a held-out
pair gain (.571 vs .523) and with the frozen suite excluded from training.

| Dataset | 2D placeholder | Candidate |
|---|---:|---:|
| RepoQA-600 | .533 | .449 |
| CoSQA-500 | .240 | .233 |

CoSQA candidate MRR@10 was .233 (placeholder .240). Every RepoQA language
dropped. Candidate binary SHA-256
`23355029090c9b540458cb43ed29bcb9fd4f911d15ef9a4ddfddd205e2763a3a`.
Placeholder restored; restored binary matches the 2D hash above.

## sgrep head-to-head on sgrep's published suite (2026-10-14)

**Status: measured, reproducible-in-tree; not a release certificate.**
Full report, rows, and provenance:
[`sgrep-suite-2026-10-14.md`](sgrep-suite-2026-10-14.md) (harness
`benchmarks/sgrep_suite/run_suite.py`). Non-cloud arms only; the paid
SWE-bench agent arm was not run. asgrep arm = unpublished local
release-ship build of this tree (`545ee9bc…`, reports 2.5.4); sgrep arm =
source build of their benchmarked commit `0fbe8e5`. Corpus/query hashes
60/60 and 120/120 equal theirs; my sgrep ranks match their published rows
on 600/600 RepoQA queries.

| RepoQA-600 | MRR@10 | Hit@1 | ≥50%@8k | panel p50 |
|---|---:|---:|---:|---:|
| sgrep | .757 | .662 | .895 | 57.3 ms |
| asgrep | .304 | .198 | .340 | 15.8 ms |

| CoSQA-500 | MRR@10 | Hit@1 | panel p50 |
|---|---:|---:|---:|
| sgrep | .417 | .292 | 181.3 ms |
| asgrep | .096 | .038 | 12.6 ms |

Setup/storage also favor sgrep (133.5 ms median first search, 173 MB vs
1,039.9 ms median index, 2,582 MB). asgrep wins only warm per-query latency,
which is not quality-adjusted. Any citation of these numbers must carry the
report's limits (one host, unpublished asgrep build, CoSQA file-naming
reconstruction, SWE-bench unmeasured).

## 2.6.0 release paired benchmark: 2.5.4 vs 2.6.0 vs sgrep (2026-10-19)

**Status: measured, reproducible-in-tree; the release-comparison fingerprint
behind the 2.6.0 README/docs tables.** Protocol:
`benchmarks/sgrep_suite/run_suite.py` phases setup/retrieve/latency/report, two
asgrep arms run identically and sequentially on one idle host (Apple M5 Max,
macOS 26.6.2), frozen 60-repo RepoQA + flat CoSQA corpora/queries hardlink-
copied unchanged from the 2026-10-14 bench root, each arm with its own fresh
`.asgrep` indexes and its own copy of the warm sgrep cache. The sgrep arm is
the same pinned binary in both runs (in-pair context, not a new head-to-head).

| Fingerprint | Value |
|---|---|
| asgrep 2.5.4 arm | tag build of `v2.5.4` (d456bb39), SHA256 `6c178ae779125c0fd9af0446e92992ca607425f3048a9fa4990ae5607beb8f8a`, reports 2.5.4 / index_schema 16 |
| asgrep 2.6.0 arm | release tree at the 2.6.0 mechanical bump, SHA256 `60f591f0b288cb03a2d8114bd6d7c96c54bc35be3915fea5fd0037da0b442cbd`, reports 2.6.0 / index_schema 17 |
| sgrep (both arms) | source build of commit `0fbe8e5`, SHA256 `743950b84b32b9c5bf1182980f909d94d2d659a2e298074f9bc8b1461347391e` |
| Harness | `benchmarks/sgrep_suite/run_suite.py` @ `9ceec783` |
| Errors/timeouts | 0/0 across all 4,400 asgrep+sgrep query runs |

| RepoQA-600 | MRR@10 | Hit@1 | Hit@5 | Hit@10 | ≥50%@8k | panel p50 |
|---|---:|---:|---:|---:|---:|---:|
| asgrep 2.5.4 (this run) | .297 | .190 | .442 | .563 | .340 | 16.6 ms |
| asgrep 2.6.0 (this run) | **.719** | .608 | .870 | .928 | .790 | 22.2 ms |
| sgrep (calibrated 2026-10-14 row) | **.757** | **.662** | **.897** | **.943** | **.895** | 59.6 ms |

| CoSQA-500 | MRR@10 | Hit@1 | Hit@5 | Hit@10 | panel p50 |
|---|---:|---:|---:|---:|---:|
| asgrep 2.5.4 (this run) | .104 | .054 | .156 | .240 | 13.8 ms |
| asgrep 2.6.0 (this run) | **.327** | .234 | .452 | .588 | 22.1 ms |
| sgrep (calibrated 2026-10-14 row) | **.417** | **.292** | **.574** | **.702** | 185.0 ms |

Cold setup per arm (suite `setup` phase, fresh indexes):

| | 2.5.4 | 2.6.0 |
|---|---:|---:|
| RepoQA index median (60 scopes) | 1,296 ms | 817 ms |
| RepoQA index p95 | 12,195 ms | 4,310 ms |
| CoSQA index (flat corpus) | 5,518 ms | 7,383 ms |
| First query after index (RepoQA median) | 13.8 ms | 21.7 ms |
| Storage, both datasets | 2.58 GB | 3.35 GB |

Notes. **The sgrep reference row is the calibrated 2026-10-14 head-to-head**
(same corpus bytes, same pinned binary, 600/600 RepoQA rank agreement with
sgrep's published rows), not this run's in-pair sgrep control: rerunning sgrep
against hardlink corpus copies at new paths broke its cache lineage and
under-measured it (.7227/.3763 MRR, only 366/600 published-row agreement).
Those in-pair sgrep numbers, identical across both arms, are retained only as
an arm-stability control and must not be cited as sgrep's quality. Against the
calibrated row, 2.6.0 is **behind sgrep on every quality metric** (RepoQA
−.038 MRR / −.015 Hit@10 / −.105 coverage; CoSQA −.090 MRR / −.114 Hit@10)
while leading every latency metric (2.7×/8.4× faster warm) with fully offline
operation. Warm p50 rises ~6-8 ms over 2.5.4 (always-on embed channel) and
storage grows ~30% (vectors) — priced, not hidden. 2.6.0's index is faster per
RepoQA scope despite embedding everything; the flat CoSQA corpus pays the
embedding cost.

### Self-corpus surfaces (2026-10-19, published 2.5.4 bytes vs 2.6.0)

Same-day companion measurement mirroring the 2026-10-11 self-corpus protocol:
20 alternating pristine-index observations; 22-query battery at 20
observations/arm after two discarded warmups; fresh processes; `RAYON_NUM_THREADS=2`;
frozen corpus = the committed raw bundle corpus (`f2bbc3d2`, 740 tracked files,
678 indexed). 2.5.4 arm = published npm `@ast-sgrep/darwin-arm64@2.5.4` bytes
(SHA256 `f4763352…a8ccb`, byte-identical to the 2026-10-11 baseline); 2.6.0 arm
= release tree SHA256 `60f591f0…42cbd`. Warm rounds via
`benchmarks/warm_distinct.mjs` with the bundle's frozen `corpus-manifest.json`
(240 identifiers), four rounds per arm.

| Surface | Published 2.5.4 | 2.6.0 | Note |
|---|---:|---:|---|
| Pristine index p50 / p95 | 17.998 / 19.067 s | 13.485 / 14.830 s | index.db 293.7→352.9 MB (+20%), semantic.ivf 13.4→19.6 MB |
| `literal:SearchHit` p50 / p95 | 5.079 / 6.1 ms | 5.998 / 6.7 ms | flat |
| `pattern:SearchHit` p50 / p95 | 6.524 / 6.9 ms | 6.552 / 7.6 ms | flat |
| `defs:*` battery (5 queries) p50 | 3.8–4.2 ms | 4.2–4.9 ms | flat |
| Conceptual hybrid (11 NL queries) p50 | 6.3–9.5 ms | 24.9–33.7 ms | always-on embed channel, disclosed |
| `semantic:` prefixed expression p50 | 6.654 ms | 27.958 ms | same cost class |
| Pure `semantic` subcommand p50 | 13.073 ms | 16.814 ms | mild |
| Warm Code Mode round p50 (240 ids, 4 rounds) | 0.67–0.76 ms | 4.27–4.84 ms | per-query embed cost |
| Self-gold MRR / nDCG (17 q, hybrid) | 0.847 / 0.853 | 0.882 / 0.881 | Oct-11 ranking regression repaired, net improved; R@20 .941→1.0 |
| Self-gold no-embed ablation MRR | 0.847 | 0.924 | embed mildly negative on this symbol-heavy self fixture; strongly positive on the frozen external suite (+.42/+.22 MRR above) |

The 2026-10-11 candidate's 255–402 ms conceptual-latency disasters are gone
(br-smf lexical-discovery repairs landed); the remaining hybrid-query cost
(~3-4× fresh-process, ~6× warm-round) is the deliberately-bought static embed
channel. Neither the self fixture nor the external suite alone describes the
product; both are recorded here.

## 2.6.0 regression investigation (br-smf, 2026-10-11)

**Status: measured partial repair; not a nonregression keep or release certificate.**
Raw samples, source hashes, focused-test command, binary hashes and warm per-query
observations are retained in `2.6.0-20261011.json` under `regression_investigation`.
The original candidate refresh below remains unchanged, including its failures.

Parent-run stage profiles and cross-index controls locate the dominant slowdown
in distinct-file lexical discovery. Cold CI discovery scanned ordered indexed
lines for every term; warm discovery scanned residual records and repeatedly
lowercased non-ASCII text. The repair uses bounded ASCII trigram preimage queries
with full Rust verification and generation-cached record-local lowercase bytes.
The combined source/folded-byte cache stays within the existing packed-byte cap.
Neither candidate quotas nor gold nor fusion/ranking coefficients changed.

| Fresh-process, page-cached query | Published 2.5.4 p50 | Partial repair p50 |
|---|---:|---:|
| `how does hybrid search work` | 7.228 ms | 7.500 ms |
| `semantic:credential renewal` | 7.588 ms | 8.788 ms |
| `literal:SearchHit` | 6.162 ms | 6.137 ms |

Twenty interleaved observations per arm after three warmups, same frozen corpus
and each arm's preserved index. Ordinary release control and partial-repair
release-ship builds were measured separately. This still is **not** a rebuilt
same-toolchain 2.5.4 source control. Semantic-expression latency remains worse;
hybrid repair CV also exceeds the keep envelope. No nonregression claim.

Warm distinct-query rounds: published p50 range **0.753–0.767 ms**, partial
repair **1.060–1.140 ms**, across four rounds per arm. This is still a regression.
Exact hit-for-hit comparisons of all **17** self-gold queries between unfixed
and repaired candidates with identical `--limit20` arguments pass. That preserves,
**not fixes**, the candidate's previously documented ranking regression. A first
comparison against saved `--limit10` previews was invalid; its mismatch and the
matched-argument correction remain in the JSON evidence.

Focused remote verification: **26 passes**, zero failures, across `literal_unit`,
`line_corpus_unit`, `literal_warm_cold_parity` and `cascade_planner`. The `edİ` →
`edi` mutation fails without Unicode preimage alternatives, then passes with them;
a scalar-table proof pins Rust's non-ASCII-to-ASCII lowercase preimages.

Actual tested release-ship SHA256:
`50ed19427c8bf3f09905e120ea428e60babd1bd34b9c40b3894426e3995670d1`.
DSR's isolated custom-profile build succeeded but its collector packaged the stale
ordinary-release binary; the actual tested executable was copied from the declared
Cargo target's `release-ship/asgrep` and separately hashed. No DSR upstream edit.
Build receipt: `/Users/aditya/.hermes/cache/scratch/wf_astsgrep_regression_c1cd79568c/dsr-candidate-two-anchors/ast-sgrep-v2.5.4-manifest.json`.

### Ranking repair and file-filter follow-up

Declaration-only lexical hits and their multi-line AST definitions double-voted
in weighted RRF. The focused fusion contract detects that defect, including lazy
definition excerpts. The repair preserves candidates/provenance and independent
body evidence without changing channel weights, gold or quotas.

On all 17 frozen self-gold queries, identical `--limit20` arguments and the same
preserved candidate index, common-scored aggregates are:

| Runtime | MRR | nDCG | Recall |
|---|---:|---:|---:|
| Published 2.5.4 | 0.847059 | 0.853016 | 0.941176 |
| Unrepaired candidate | 0.812325 | 0.834978 | 0.941176 |
| Ranking + discovery repair | 0.864706 | 0.865792 | 0.941176 |

Aggregate ranking recovered, not every ordinal: the secondary snapshot target
is rank5 versus published rank4; the primary snapshot target is restored to rank1.
The previously absent follow-up target remains absent. No gold was changed.

A conservative, generation-owned lowercase byte-trigram file filter skips only
impossible residual substring scans; collisions still require exact verification.
Its 512 bytes/file are charged against the existing packed-corpus cap. Full
reused-worker responses for 240 distinct identifiers and all 17 cold gold-query
hit arrays are identical with versus without this filter.

| Warm navigation, median of eight round p50s | Latency |
|---|---:|
| Published 2.5.4 | 0.763354 ms |
| Ranking repair, without file filter | 1.100459 ms |
| Ranking repair, with file filter | 0.845479 ms |

Four order-swapped batches, two 240-query rounds per arm, ten unmeasured warmups;
same corpus, manifest, index, limit8 and worker surface. Fresh-process shared-index
controls, 40 observations/arm, still miss published parity: hybrid p50
7.766041/8.290583 ms, prefixed semantic 8.153187/9.739250 ms, literal
6.582625/6.870167 ms (published/repaired). These are page-cached process-inclusive
measurements, not cold-storage latency. All samples/tails, source/native hashes
and per-query scores are in `regression_investigation.ranking_and_filter_repair`.

Focused remote verification on spark-1672: **60 passes**, zero failures, one
pre-existing ignored self-index test, across seven named core suites. Inverting
the file certificate fails four existing corpus tests; misclassifying declaration
spans fails the fusion contract. Both mutants were restored before the passing run.

DSR command: `dsr build ast-sgrep --target darwin/arm64 --allow-dirty --no-sync
--jobs 1 --output-dir <scratch>/dsr-ranking-filter-candidate`, with the recorded
scratch config selecting `release-ship`: build **PASS**, collector still packaged
the ordinary-release artifact. Actual measured profile executable SHA256:
`622cd1f7266f9f7c13cb0ecad6b8f9059d6ea3c07876db9910109a457222d30d`.
Receipt: `/Users/aditya/.hermes/cache/scratch/wf_astsgrep_regression_c1cd79568c/dsr-ranking-filter-candidate/ast-sgrep-v2.5.4-manifest.json`.

**Release remains blocked.** Remaining cold/warm latency, a rebuilt same-toolchain
old-source control and the complete prerelease/Pi refresh are not cleared by this
bounded repair. Earlier partial measurements above remain historical observations.

## 2.6.0 candidate refresh (2026-10-11)

**Status: canonical measured prerelease snapshot, not an optimization keep or
release certificate.** Machine-readable observations, command argument templates,
corpus hashes and compiler/tool pins: [2.6.0-20261011.json](2.6.0-20261011.json).
Older dated sections below remain historical evidence, not current release numbers.

### Fingerprint and measurement limits

- Frozen corpus: all 740 tracked files at `f2bbc3d2768db3d54658b0be4f15d14777b18505`,
  12,579,244 source bytes. Every frozen file matches its Git blob. Manifest SHA256:
  `f6597eef9a4d93c616f67f69754cc9d05ddcd7f49ae5acbb5ad047c55c9a0a86`.
- Baseline: published npm `@ast-sgrep/darwin-arm64@2.5.4`; executable SHA256
  `f476335298a6af293c35163c3a1143a9beea8941debd183331fcd71b05aa8ccb`.
- Candidate: prerelease native production bytes from `a2e66f22`, unchanged by
  `f2bbc3d2`'s release-script repair; executable SHA256
  `9786bbf48fc572675efafc5199ed46185efb985e59968e12ee05e23acda541aa`.
  Embedded version is still **2.5.4**: this is a 2.6.0 candidate, not a published release.
- Host: Apple M5 Max, 18 logical CPUs, 48 GiB RAM, macOS 26.6.2, APFS/internal SSD.
  Candidate uses Rust 1.97.1, release profile, no default features; Node 22.23.3.
  Runtime `RAYON_NUM_THREADS=2`; no governor, CPU-affinity or global cache changes.
- This is **published binary versus prerelease binary**, not a matched-toolchain,
  one-lever source A/B. No neural model or reranker is active.
- Twenty alternating observations/arm for pristine indexing and CLI queries;
  CLI queries discard two warmups. OS pages are warm; fresh processes are **not**
  disk-cold runs. p50 is the sample median; p95/p99 are empirical nearest-rank
  percentiles. Twenty samples do not establish a tail SLA. CVs are retained;
  several exceed the 5% keep envelope. Do not certify small differences as wins.

### Pristine indexing and size

| Metric | Published 2.5.4 | Candidate |
|---|---:|---:|
| Index wall p50 | 19.172 s | 12.699 s |
| Index wall p95 | 21.293 s | 15.251 s |
| Indexed files | 678 | 678 |
| Extracted symbols | 6,691 | 6,691 |
| index.db bytes | 293,654,528 | 293,662,720 |
| semantic.ivf bytes | 13,405,184 | 13,405,184 |

Pristine index p50 is 33.8% lower in this comparison;
index size is essentially unchanged. This does not erase the search regressions.

### Fresh-process CLI queries

All cells are milliseconds, indexed corpus, limit 20, `--no-auto-index`.

| Query | 2.5.4 p50 / p95 | Candidate p50 / p95 |
|---|---:|---:|
| `literal:SearchHit` | 6.294 / 7.057 | 6.545 / 6.947 |
| `pattern:SearchHit` | 7.289 / 8.383 | 7.517 / 7.915 |
| `semantic:credential renewal` | 7.926 / 11.425 | 377.366 / 402.818 |
| `how does hybrid search work` | 7.125 / 11.115 | 263.541 / 267.402 |
| `defs:dedup_hits` | 7.379 / 8.010 | 6.792 / 8.242 |
| `defs:compute_ann_fingerprint` | 4.526 / 5.254 | 4.639 / 5.059 |
| `defs:begin_bulk_tx` | 4.643 / 5.205 | 4.718 / 5.468 |
| `defs:index_generation` | 5.012 / 5.449 | 5.005 / 5.494 |
| `defs:to_compact_json` | 5.107 / 5.531 | 5.167 / 5.894 |
| `literal:PRAGMA synchronous` | 5.892 / 6.364 | 5.954 / 6.655 |
| `literal:BEGIN DEFERRED` | 5.636 / 6.147 | 5.634 / 6.367 |
| `callers:integrity_check` | 4.926 / 6.429 | 5.104 / 6.685 |
| `word:SnapshotStamp` | 5.680 / 6.105 | 5.783 / 6.248 |
| `index write durability profile` | 7.476 / 8.203 | 311.533 / 322.662 |
| `compact output path interning` | 7.777 / 8.239 | 362.449 / 375.502 |
| `consistent search snapshot generation` | 8.303 / 8.968 | 315.888 / 318.644 |
| `penalize misleading identifier collisions when ranking results` | 8.661 / 9.796 | 386.886 / 396.800 |
| `derive the next command to run from the best result` | 8.192 / 8.697 | 255.226 / 260.264 |
| `combine two search channels in a single query` | 7.788 / 8.737 | 313.434 / 317.182 |
| `reciprocal rank fusion across evidence channels` | 9.889 / 10.903 | 349.794 / 381.644 |
| `remember query embeddings between searches` | 10.238 / 10.796 | 295.243 / 305.706 |
| `asgrep semantic "credential renewal"` (pure subcommand) | 12.186 / 13.051 | 12.431 / 13.087 |

`semantic:credential renewal` is the prefixed search expression; `asgrep semantic`
is the pure semantic subcommand. They are not interchangeable benchmark paths.
The first untimed pure-subcommand observations were 202.269 / 193.007 ms;
those single initialization observations are not cold-latency percentiles.
The candidate's full-repository conceptual discovery is substantially slower.
This refresh does **not** approve that regression or attribute it to one source lever.

### Scan/server references

Tools: ripgrep 15.2.0, ast-grep 0.50.0, tgrep 1.1.0,
fff-mcp 0.11.0 (`95fd777c2529fc7b4d7572dabff64cc07268f2c5`),
Semgrep native core 1.180.0. Twenty measured observations/reference.

| Reference | p50 ms | p95 ms | Scope |
|---|---:|---:|---|
| rg | 16.260 | 17.624 | `rg -n SearchHit .`, scan |
| grep | 64.058 | 71.036 | `grep -rn --exclude-dir=.git SearchHit .`, scan |
| ast-grep-full | 272.353 | 284.613 | Rust `SearchHit` pattern, generated file included |
| ast-grep-handwritten | 72.382 | 80.182 | Same pattern, `trigram_static_data.rs` excluded |
| tgrep-warm-server | 6.721 | 8.673 | Warm prepared server; client process wall time |
| fff warm MCP | 0.226 | 0.340 | Ranked 20/50 shown, latency-only |
| Semgrep native core | 571.919 | 609.370 | One explicit Rust `SearchHit` rule, 381 targets, 206 matches, zero reported errors |

These compare indexed ranked retrieval with scan/parse-per-query or prepared
servers, **not match-set or quality parity**. The generated-file control matters.
Semgrep's Python CLI fails importing `mcp.server.fastmcp`; the native core
reference bypasses it without changing the installation. This is **not** a
refresh of the missing historical 29-pattern Semgrep suite.

### Warm Code Mode and Pi

`warm_distinct.mjs`: 20 alternating blocks/arm, 240 distinct source-derived
identifiers/block, ten warmup calls, reused NDJSON worker. Values below are
medians of the 20 per-block percentiles, not fresh CLI timings or a pooled SLA.

| Code Mode metric | 2.5.4 | Candidate |
|---|---:|---:|
| Round p50 | 0.715 ms | 10.914 ms |
| Round p99 | 1.941 ms | 29.064 ms |

Pi: the existing `p100-bench.mjs` generator, fake Pi host, actual old/new
extension and matching native addon; 20 alternating blocks/arm. Baseline is
published `pi-ast-sgrep@2.5.4` with `typebox@1.0.0`. The 3,000-file fixture is
synthetic and is **not** the full-repository Code Mode workload above. Startup
search follows a 600 ms session-start wait; first calls returned hits, but the
old/new hit counts differ, so these timings do not establish rank parity.

| Pi metric | 2.5.4 p50 ms | Candidate p50 ms |
|---|---:|---:|
| First search, no index, 3,000 files | 341.805 | 329.955 |
| Second search | 0.353 | 0.391 |
| Full reindex, 3,000 files | 14522.522 | 14629.103 |
| Session-start first search, 300 files | 5.173 | 4.817 |
| Session-start first search, 3,000 files | 10.559 | 10.336 |
| Warm distinct calls (600 observations/arm) | 0.402 | 1.007 |
| Calls during reindex (160 observations/arm) | 0.464 | 0.803 |

All four sample/default and self identity-suite runs pass their named hit-count
and identity checks. Their per-case first-query and warmed means/CVs are retained
separately in the bundle; an in-process mean is not a CLI percentile. The self
suite's conceptual cases also expose the candidate slowdown.

### Retrieval quality and ablations

Same existing 17-query self gold, same frozen corpus, primary cutoff per query.
Common one-to-one gold scoring of saved native top-20 hit lists agrees with
both executables' `eval` aggregates. No replacement gold was authored.

| Self metric | 2.5.4 | Candidate |
|---|---:|---:|
| MRR | 0.847 | 0.812 |
| nDCG@k | 0.853 | 0.835 |
| Recall@k | 0.941 | 0.941 |
| Recall@1 | 0.706 | 0.676 |
| Recall@5 | 0.941 | 0.853 |
| Recall@20 | 0.941 | 0.941 |

The ranking regression is not a changed scoring formula: saved first-rank
observations agree with the common scorer. `conceptual-follow-up-derivation`
remains a miss in both arms. `no-embed` and `concat-embed` ablations have zero
aggregate deltas in both arms. This gold does not demonstrate an embedding win.

- Native-semantic vocabulary fixture: three queries; both releases MRR/nDCG
  1.000, Recall@1 0.500 and Recall@5/20 1.000. Removing repository vocabulary
  lowers nDCG by 0.258 and Recall@5/20 by 0.333 in both releases; MRR is unchanged.
- Invent-path fixture: six queries; both releases MRR/nDCG/Recall@1/5/20 1.000.
  Small authored fixtures are not foreign-repository or neural-quality evidence.
- Graph fixture: all four labeled edges correct in both arms; each of
  file-local-unique, repository-unique, name-only and SCIP-occurrence has one
  prediction. Compiler-exact, import-resolved, ambiguous and unresolved tiers
  have no observations and remain `null`, not synthetic precision scores.

### Complete stdout bytes and budget check

Seventeen self queries, limit 10, same ranked response within each arm. UTF-8
stdout bytes include framing, whitespace and the final newline; no tokenizer.
Old/new rankings differ, so cross-version byte differences are not a
serialization-only optimization claim.

| Format | 2.5.4 bytes | Candidate bytes |
|---|---:|---:|
| native | 111,795 | 105,896 |
| agent | 125,358 | 118,270 |
| agent-capsule | 78,302 | 74,164 |
| compact | 20,292 | 19,306 |
| compact-budget300 | 17,874 | 17,056 |

Budgeted/unbudgeted compact output preserves the ranked hit identities for all
17 queries in both arms. Re-exercising `run_eval.sh` against the frozen corpus
reproduces the candidate byte totals and budget check. That measurement-only
pack explicitly records `dirty_worktree=true`, `reliability_tests_run=false`;
it is not an assertion that reliability tests or a release certificate passed.

### Adaptive IVF and mmap

Existing adaptive fixture: 2,048 vectors, dimension 32, seed `0x5D0_036`,
256 queries, recall@10 0.9984375 against all-cluster retrieval. Candidate
counts range 1,784..1,855 (mean 1,821.305), below the existing 95% ceiling;
recall passes the 0.99 error-budget gate. Timed exact/adaptive calls include
HashSet result materialization, not only the ANN microkernel.

The existing ignored tradeoff test ran 20 times at each size/probe setting.
Recall and candidate fractions are deterministic; latency is the median of
20 run-level average microseconds/query, **not a query tail percentile**.

| Vectors | Probe fraction | Recall@10 | Candidate fraction | Median average us/query |
|---:|---:|---:|---:|---:|
| 2048 | 50% | 0.931250 | 0.511459 | 34.544 |
| 2048 | 75% | 0.989062 | 0.754547 | 46.688 |
| 2048 | 90% | 0.998437 | 0.888893 | 52.904 |
| 2048 | 100% | 1.000000 | 1.000000 | 56.422 |
| 10000 | 50% | 0.982812 | 0.499580 | 161.861 |
| 10000 | 75% | 0.996875 | 0.749023 | 225.989 |
| 10000 | 90% | 1.000000 | 0.899686 | 259.245 |
| 10000 | 100% | 1.000000 | 1.000000 | 280.399 |

Mmap fixture: 10,000 vectors, dimension 8, sidecar 365,056 bytes including
320,000 vector bytes; 100 observations/mode. All opens require mapped vectors.
The writer uses `F_NOCACHE` plus fsync for the cold-policy case, outside timing;
no global cache purge was performed, so this is not guaranteed physical disk-cold.

| Opening mode | p50 ms | p99 ms | Definition |
|---|---:|---:|---|
| cold | 0.269188 | 0.752667 | Unique inode, writer F_NOCACHE, fresh process |
| fresh-inode | 0.086896 | 0.100750 | Unique inode, ordinary page-cache policy, fresh process |
| warm | 0.094104 | 0.129250 | Same page-cached inode, fresh process |
| In-process warm | 0.037542 | 0.095333 | Same process/inode, one untimed opening |

The bundle pins both probe executable hashes: the new repeated-opening argument
is measurement support, not a production mmap change. Current in-process warm
p99 remains below 1 ms; this does not certify other machines or storage policies.

### Unavailable historical measurements

Git history and tracked-tree inspection did not recover the missing originals.
Do not replace them with invented gold, synthetic scale corpora or proxy claims.

| Surface | Current disposition / retry condition |
|---|---|
| foreign-corpus self18/ripgrep/flask/redis quality, neural/rerank bakeoff | **Historical / UNREPRODUCIBLE**: Original gold, pinned corpora, competitor patterns and bakeoff evaluator are absent; named ripgrep gold/corpus paths have no Git history. Retry only when: Recover the exact original gold, corpus pins, competitor rules and evaluator. |
| 23k/100k lexical and structural scale, 29-pattern Semgrep scale suite | **Historical / UNREPRODUCIBLE**: Original generator, query battery, corpus snapshots and hand-pattern rules are absent. Retry only when: Recover the original scale generator, complete query/pattern suite and corpus fingerprints. |
| watch CPU/latency/idle-memory speed study | **Historical / UNREPRODUCIBLE**: Behavior tests exist; original timing workload, transcript, machine fingerprint and sampler are missing. Retry only when: Recover the original timing workload, sampler and machine fingerprint; passing behavior tests are not speed evidence. |
| intent confusion and routing-weight grid | **Historical / UNREPRODUCIBLE**: intent-calibrate.py plus the historical multi-corpus gold/corpus inputs are missing. Retry only when: Recover the original classifier labels, corpus/gold pins and intent-calibrate.py. |
| foreign-corpus prevented-read study | **Historical / UNREPRODUCIBLE**: The 20-query ripgrep gold, pinned corpus and original study writer/results JSON are missing. Retry only when: Recover the exact 20-query gold, corpus and writer; do not substitute current output byte totals. |
| 54,732-chunk idx_big 85-query semantic/hybrid run | **Historical / UNREPRODUCIBLE**: idx_big, 85-query battery and original corpus fingerprint are missing. Retry only when: Recover idx_big/corpus and the original 85 queries plus command/config pins. |
| CASS 60-day negative-history exclusion | **Historical / UNREPRODUCIBLE**: CASS code 5: searchable lexical metadata unavailable, archive exceeds automatic-repair budget. Retry only when: Lexical metadata must become searchable; do not run a second rebuild while the CASS-owned detached rebuild runs. |

Semgrep's native one-rule timing does not restore the foreign-quality bakeoff.
Watch behavior checks and Pi reindex timings do not restore a watch speed study.
This is a complete disposition of the existing matrix, **not** a claim that
unavailable historical cells were rerun successfully.

### Regeneration

Use the JSON bundle's exact command templates, gold/configs, corpus file hashes
and release pins. `$CORPUS` must be a materialization of the pinned Git blobs,
not a copy of a later dirty working tree. Avoid `git archive`: this repository's
export-ignore attributes omit benchmark inputs. Build the candidate from its
pinned production inputs, not from an arbitrarily newer checkout.

- CLI: each template query has two warmups and 20 measured fresh processes;
  rotate old/new order on each observation. Indexing uses a new database/iteration.
- Suites: `asgrep --json --index-path <new-index> bench <root> --suite
  default|self --fixture sample|self --iterations 20 --warmed`.
- Quality/output pack: `ASGREP_EVAL_BIN`, `ASGREP_EVAL_ROOT` and
  `ASGREP_EVAL_OUT` select a prebuilt binary, frozen root and retained output.
  `ASGREP_EVAL_SKIP_RELIABILITY=1` is measurement-only and is recorded as such.
  Default working-tree runs still refuse dirty inputs. No temporary index is deleted.
- Warm sessions: `ASGREP_BENCH_ROOT`, `ASGREP_BENCH_MANIFEST`,
  `ASGREP_BENCH_INDEX_PATH`, `ASGREP_BENCH_JSON=1` with
  `node benchmarks/warm_distinct.mjs <binary> 1`; repeat 20 alternating blocks.
- Pi: `ASGREP_BIN`, `ASGREP_CODEMODE_NAPI_PATH`, optional baseline
  `ASGREP_BENCH_EXTENSION_DIR`, `ASGREP_BENCH_KEEP_FIXTURES=1`,
  `ASGREP_BENCH_JSON_PATH` with `node packages/pi/extension/scripts/p100-bench.mjs`.
- fff: `ASGREP_BENCH_JSON=1 node benchmarks/fff_grep_leg.mjs <root> SearchHit 20`.
- Semgrep native reference: `semgrep-core -json -rules <rule-file> -targets
  <targets-file> -j 2`. The bundle includes the exact one-rule definition; targets
  are its 381 `.rs` corpus paths as `CodeTarget`/Rust/SAST. Preserve progress-dot
  framing when retaining raw output; parse the JSON object following those dots.
- IVF: named `adaptive_ivf_recall_at_10_stays_within_quality_error_budget` and
  `adaptive_ivf_tradeoff_at_2048_and_10000_vectors` tests in
  `ast-sgrep-core --test semantic_ivf_roundtrip` (tradeoff requires `--ignored`).
- Mmap: `semantic_ivf_open_probe prepare <path>` then `open <path>`; repeated
  same-process control is `open <page-cached-path> 101`, discarding only the first
  opening. Unique-file preparation/fsync/cache policy is outside the timed region.

Unabridged local command receipts, failed attempts, stdout/stderr, databases and
fixtures are retained under gitignored `benchmarks/results/raw/2.6.0-20261011/`.
Only the curated bundle is a public results artifact. No release/tag/push follows
from this benchmark refresh.

## Historical optimization baseline (2026-10-06, br-mcm)

**Status: measured current baseline and stage attribution, not an optimization
keep.** Frozen native SHA256
`89eebe1889c1b8cb6cee67f509d9f04915f06d87e7efe2612be63b26810663e4`.
Required-remote RCH built only `ast-sgrep-cli --bin asgrep --locked --offline
--profile release-perf --target aarch64-unknown-linux-gnu --jobs 2`, using inner
`env CARGO_HOME=/home/aditya/.cargo` and the existing check target-dir base.
Compiler receipt confirms rustc 1.98.0 / LLVM22.1.8. All180 selected regular
crate/vendor/native manifest/static inputs match dispatcher and worker and stayed
unchanged through measurement; this is not a complete registry/Cargo closure.
Profiling executable includes symbols and is not a shipped-size measurement.

Reuse the preserved frozen-self corpus and release-baseline/self.db, explicit
--no-auto-index, default CLI features/embedding policy, one native process per
query, warm OS cache, CPU5 on shared spark-1672. Seventeen existing gold query
inputs each have20 observations after2 warmups; query order rotates/reverses.
All340 measured stdout/stderr replies match the new frozen current replies.
There is no comparison against an old implementation and no fresh quality score.

| Current query | Fresh-process p50 ms | Empirical p95 ms |
|---|---:|---:|
| defs:dedup_hits | 2.920 | 3.635 |
| literal:PRAGMA synchronous | 3.845 | 5.257 |
| word:SnapshotStamp | 3.838 | 5.082 |
| compact output path interning | 383.172 | 387.790 |
| consistent search snapshot generation | 379.756 | 383.873 |
| reciprocal rank fusion across evidence channels | 381.895 | 402.307 |
| penalize misleading identifier collisions when ranking results | 410.333 | 424.329 |

Separate instrumented phase calls preserve stdout: lexical prefilter sums
2,627,033us over8 conceptual queries, symbol16,628us over8, finish1,879us over17.
These nested diagnostic wall spans must not be summed into CPU percentages.
They identify the current discovery path as the first optimization target, not
proof of which inner operation dominates. Three separate GNU-time probes each
for the slowest3 queries record user/system CPU and peak RSS; complete raw values
are retained, not merged into uninstrumented latency trials. Index size/mtime
remain unchanged. P95 among20 observations is descriptive, not a tail SLA.
No cold-storage, reused-session, Mac/Windows/neural/release certificate.

Evidence root `/Users/aditya/.hermes/artifacts/ast-sgrep-perf-current-20261006/`:
`baseline.py`, `baseline-build.log`, `baseline-freeze.json`,
`baseline-project-inputs.json`, `baseline-fingerprint.json`,
`baseline-samples.jsonl`, `search-golden.json`, `baseline-summary.json`,
`baseline-phase-profile.json`, `baseline-hotspots.json`. Worker mirror:
`/home/aditya/.cache/hermes/scratch/ast-sgrep-perf-current-20261006/`.
Exact runtime producer: `python3 .../baseline.py` on that worker; it refuses to
overwrite old outputs. Native binary is `asgrep-baseline` in the worker mirror.
This checkpoint does not refresh or invalidate earlier paired receipts. CASS
exclusion remains OPEN code5. No production optimization had been applied at
this initial checkpoint; retained changes are recorded below.

### Validated-prefix discovery exclusion: retained (br-mcm)

**Status: first retained optimization against the current baseline above;
worker-local paired evidence, not a universal speedup.** Later cold discovery
terms skip previously admitted files in SQL only when their keys are strictly
before the greatest already-admitted path. Prior successful ordered scans have
fully validated those files in the same fenced snapshot; the boundary file may
have stopped at its first match and remains in the stream. The child's proposed
all-files exclusion was not accepted. Unicode/NUL matching, Rust skip checks,
preferred/secondary ordering, term/reserve/output limits and decoder behavior
remain unchanged; no new schema/cache/dependency. Statement shapes use padded
power-of-two bindings. Only literal.rs differs among180 checked project inputs.

Original native SHA256 is the current baseline above; candidate/restored native:
`893d81563882573ae3dd09e5119737e48fc8f4fa16aca15e8e34aca237bccd90`.
Same rustc1.98.0, release-perf/default features, Linux aarch64 worker and CPU5.
Seventeen frozen current queries have24 alternating pairs after2 warmups,
12 original/candidate and12 reverse. All816 measured replies are byte-identical
in stdout/stderr. These pairs, not the earlier standalone baseline timing, price
this change; no subtraction across separate measurement blocks.

| Conceptual query | Original p50 ms | Candidate p50 ms | Median paired change |
|---|---:|---:|---:|
| index write durability profile | 310.470 | 291.986 | -6.36% |
| compact output path interning | 360.304 | 340.329 | -5.38% |
| consistent search snapshot generation | 355.460 | 316.193 | -11.06% |
| penalize misleading identifier collisions when ranking results | 385.300 | 361.551 | -6.30% |
| derive the next command to run from the best result | 179.516 | 176.312 | -2.83% |
| combine two search channels in a single query | 309.479 | 290.465 | -6.41% |
| reciprocal rank fusion across evidence channels | 358.637 | 328.536 | -8.79% |
| remember query embeddings between searches | 307.181 | 277.804 | -10.12% |

Countermetrics retained: initial short-control p95s rise for BEGIN DEFERRED and
callers:integrity_check; a single separate60-pair confirmation across9 controls
has paired medians -2.14%..+2.78%, with mixed tails. It does not erase the original
p95s or certify all tails. Independent GNU-time CPU/RSS probes (3/arm on3 real
conceptual cases) support lower CPU, essentially unchanged RSS. Stage probes are
diagnostic wall attribution, not CPU percentages. No warm-session/cold-storage/
Mac/Windows/neural/full-quality/release certificate.

Twenty-four decoder/failure observations replay exactly. A new diagnostic fixture
places invalid UTF-8 after the first match in the32nd admitted file: original
and retained fail operationally2. Changing strict '< frontier' to '<= frontier'
suppresses that error and returns success0, detecting the proposed unsafe bound.
Source restored byte-exact; named core identifier_ranking/literal_unit/
literal_warm_cold_parity pass33, with1 existing self-gold ignore. Restored CLI
binary matches the measured candidate hash and replays17 searches,24 decoder
observations and the frontier error exactly. No extra repo test framework.

Evidence under the current checkpoint root: `exclusion-paired.py`,
`exclusion-pairs.jsonl`, `exclusion-paired-summary.json`,
`exclusion-countermetrics.json`, `exclusion-frontier-{error,mutant}.json`,
`exclusion-restored-native-replay.json`, `exclusion-verification.json`, snapshots,
source manifests and complete named RCH logs. Runtime command on the worker:
`python3 /home/aditya/.cache/hermes/scratch/ast-sgrep-perf-current-20261006/exclusion-paired.py`.
The Python SQLite3.45.1 substrate probe retains the same path-index/line-index
plan, but is not a bundled-native EXPLAIN certificate. Its initial missing file:
URI prefix created empty `release-baseline/self.db?mode=ro`; actual DB untouched,
empty file preserved under the no-deletion policy. Countermetric console summary
failed after complete artifact write; that existing artifact was parsed without
rerunning observations. CASS exclusion remains OPEN5.

### Code Mode admission duplicate-walk experiment: BLIND (br-mcm)

The public batch experiment resolved real payloads (1/6/19 hits; sorted-JSON
value lengths904/2952/8482 bytes) but not the proposed per-serve admission cost.
Three cases, single vs8 identical calls,20 observations/arm after2 warmups:
all120 measured batches preserve stable payloads and leave DB size/mtime unchanged.
All three single/repeat IQRs overlap. Median client walls (single/repeat8 ms):
small469.798/480.178, default466.866/459.963, large456.939/450.210. The negative
default/large subtraction is noise, not negative work or an admission speedup.
**No call-level admission bypass applied.** No observed cache-hit counter or
admission CPU attribution. Batch wall_ms begins inside run_batch, excluding
CLI startup/request parsing/final JSON serialization; it is not whole-process
wall. A difference of independent medians is not a rigorous per-serve bound.
Producer `admission-baseline.py` was corrected before execution to keep request
files in worker scratch without deletion. SHA is the retained exclusion binary
above. Evidence: `admission-baseline.{json,jsonl}`, run log, corrected
`out_admission_workload.md`. Reopen only with same-public-path timing resolving
the positive single/repeat control without changing caps/payload semantics.

### Corpus single-component token preparation: retained with controls (br-mcm)

The admission experiment exposed a separate preparation cost: three bounded
own-inferior gdb boundary probes place line_corpus_load at250.369..262.321ms and
trigram_refresh at143.114..149.557ms inside warm_search_path456.203..475.948ms.
These are diagnostic wall boundaries, not CPU percentages or additive profiling
totals. Actual indexed input has1,055,290 ASCII identifiers of length>=3,
361,539 first raw occurrences per file; generated trigram source materially
affects this frozen-self corpus, so do not generalize corpus composition.

One leaf specialization reuses the existing posting for all-lowercase ASCII/
digit tokens: split_ident has one identical component, so its duplicate owned
keys/split/insert work is unnecessary. Uppercase/underscore shapes keep the old
path. No new cache/schema/dependency; warm DF, matching, ordering, caps, errors,
generation and snapshot policy remain unchanged. Only line_corpus.rs differs
among180 selected native project inputs from the previous retained binary.
Original SHA893d81563882573ae3dd09e5119737e48fc8f4fa16aca15e8e34aca237bccd90;
candidate SHAbb3f6e2a7a6e8e3d18b14c25b7d4dfe2b86bcdc21eb9ede9eedd8045018c77d7.
Same required-remote release-perf/rustc1.98.0/Linux aarch64/default features.

| Public Code Mode batch | Original p50 ms | Candidate p50 ms | Median paired change |
|---|---:|---:|---:|
| small,1 call | 470.149 | 394.380 | -16.16% |
| small,8 identical calls | 460.786 | 387.668 | -15.70% |
| default,1 call | 459.730 | 386.140 | -15.82% |
| default,8 identical calls | 458.785 | 385.863 | -15.88% |
| large,1 call | 454.447 | 381.056 | -15.68% |
| large,8 identical calls | 461.046 | 388.391 | -16.24% |
|17 unique frozen queries | 800.496 | 668.107 | -11.37% |
| catalog control (no search preparation) | 1.187 | 1.240 | +4.72% |

Twenty-four alternating pairs/case after2 warmups, same CPU5/shared worker/warm
OS cache; all384 measured batches (2160 call values) match after removing only
top-level wall_ms. Eight query shapes have both order arms reported; real
session output hit counts stay fixed. Separate17 cold queries also preserve816
exact replies plus24 decoder observations and the late frontier error. This is
a preparation win, not another cold-search or per-cache-hit speed claim.

Countermetrics are not hidden: a single60-pair confirmation across10 short
controls has catalog+6.22% (median1.184->1.245ms), defs-dedup+5.02% with opposing
order effects, remaining paired changes+0.75..+3.66%, mixed tails. Cold conceptual
changes -0.16..+1.02%. **No universal non-regression claim.** Separate GNU-time
small/large probes support lower CPU; unique17 candidate CPU is noisy and worse
in2/3 samples. RSS is effectively flat. Three candidate gdb probes put corpus
load at191.811..196.125ms; trigram preload unchanged, no CPU-share claim.

Existing tests originally let a wrong-token-key mutation survive through
residual scans. Expanded the existing posting-contract test while that fault
was present: RED101 proves numeric posting12345 missing versus[0,1]. The
overbroad split-guard mutation independently fails; both restored byte-exact.
Final named core corpus/cascade/parity gates pass24/no ignores; rustfmt/diff
checks pass. No new test file/framework. Frozen release binary was not replaced
by debug mutation builds, and its180 input hashes still match final product code.
Evidence: current artifact root `single-token-{session,cold}-paired.py`, golden/
pairs/summaries, `single-token-countermetrics.py`/JSON, profile/frequency producer
and outputs, all remote build/test/mutation logs, retained source and
`single-token-verification.json`. No cold-storage/neural/sticky-tail/platform/
release certificate; CASS5 exclusion OPEN. Exact runtime producers run with
`python3 /home/aditya/.cache/hermes/scratch/ast-sgrep-perf-current-20261006/<producer>.py`.

### Initial indexing phase checkpoint (br-mcm, no candidate)

Current retained bb3f6e2a native, real frozen-self root, new separate scratch DB,
no embeddings, CPU5/6 affinity and RAYON_NUM_THREADS=2. One diagnostic initial
index succeeds:660 files indexed,9 skipped,0 failed/depth-truncated/removed;
6377 symbols,64776 callers,2165 imports. Existing perf wall spans:
index_walk_parse8029.173ms, sqlite_upsert3480.434ms, index_all11693.723ms.
Hash samples sum6.226ms over660 files; these are nested/parallel wall observations,
not CPU fractions or a throughput benchmark. Preparation/extraction is the next
measured node; no indexing optimization applied. Evidence/runtime argv and full
stdout/stderr: `index-initial-current-profile.json` in the current artifact root.
The new scratch index is `index-initial-current.db`; original frozen index and
source corpus are not modified. No cold-storage/platform/neural certificate.

### Initial-index parser point reuse: retained (br-mcm)

One leaf change in lang/extract.rs reuses tree-sitter start/end row coordinates
instead of counting every source prefix for symbol/call/import/pattern rows.
The child-proposed newline-offset table was not built: TS0.26.10 already counts
LF in its lexer and stores the matching start/end extents. Current callers use
whole-source UTF-8 trees, without edits/included ranges/UTF16 parsing; row+1 has
the old one-based/exclusive-end semantics. Arbitrary-byte structural matching
retains byte_to_line. No cache/table/allocation/schema/dependency, traversal,
depth budget, stats, admission or threading change. Only extract.rs differs
among180 selected current native source inputs.

Original native SHAbb3f6e2a7a6e8e3d18b14c25b7d4dfe2b86bcdc21eb9ede9eedd8045018c77d7;
candidate SHAee836a6d140eed0bfe601e9acfdaf70a71dccb4bc86c971ef3d392cea70ce641.
Same required-remote release-perf/rustc1.98.0/default features/Linux aarch64.
Twenty alternating pristine-DB pairs/case after2 warmups, CPUs5/6,
RAYON_NUM_THREADS=2, warm OS cache/shared worker, no embeddings. All120 measured
initial indexes preserve full stdout/stderr and seven sorted canonical table
projections (files/lines/symbols/callers/imports/pattern_nodes/meta); surrogate
IDs excluded, semantic_chunks empty. Source hashes fixed before/after.

| Initial-index case | Original p50 ms | Candidate p50 ms | Median paired change | Original/candidate p95 ms |
|---|---:|---:|---:|---:|
| complete frozen660-file corpus | 11996.980 | 9665.594 | -19.86% |12193.412 /9944.888 |
|32 ordinary core-source files, no generated staticdata | 565.662 | 490.742 | -17.54% |724.672 /659.258 |
|60 variants of15 language fixtures | 72.883 | 72.325 | +0.61% |78.791 /78.160 |

The complete index keeps6377 symbols/64776 callers/2165 imports and all470584
pattern rows,349491 lines,1986 metadata rows exactly. Ordinary32 remains480/
3794/178/28325; the60-fixture control covers LF, CRLF, Unicode comments with CR
and missing final LF across all15 existing language fixtures. Full-case order
arms -19.81%/-20.37%; ordinary32 -20.56%/-7.77%. Tiny fixture change is noise,
not another win. Existing exact all-language golden suite and parser mutation/
candidate-kind tests pass12/no ignores before/after. Independent endline+2 and
zero-based caller-line faults fail101; source restored byte-exact. No new tests
needed; these existing goldens pin the changed rows.

Independent GNU-time3 probes/arm/case: full user CPU median16.53->12.16s,
ordinary0.61->0.51s, tiny0.02s unchanged; RSS mixed/nearflat, not a RAM reduction
certificate. Separate stage calls place walk_parse6880.542->4607.302ms,
sqlite_upsert3373.125->3426.020ms; diagnostic nested/parallel wall, not CPU
percentages or pricing from separate baseline blocks. Cold17-query24-pair
countermetrics preserve816 replies plus24 decoder observations; two nonempty
native structural queries also match. Cold conceptual paired changes
-0.02%..+0.28%; no cold-search win. BEGIN DEFERRED p95 rises2.919->3.880ms,
other small controls have mixed order/tail effects: no universal tail claim.

Evidence/runtime producers at current artifact root: `node-points-index-paired.py`,
input manifest/goldens/pairs/summary, `node-points-countermetrics.py`/JSON and
cold pairs, exact source/native inputs, named remote build/test/mutation logs,
`node-points-verification.json`. Exact commands are
`python3 /home/aditya/.cache/hermes/scratch/ast-sgrep-perf-current-20261006/node-points-index-paired.py`
and `.../node-points-countermetrics.py`. Long initial producer transport timed
out420s while its owned worker process continued; verified its exact identity,
waited via pidfd and read completed60 pairs/3 summaries. Original shell exit/log
unavailable, not fabricated; each native invocation asserts exit0. Countermetric
producer exits0. Pristine scratch DBs retained, no deletions. No refresh/neural/
cold-storage/Mac/Windows/release certificate; CASS5 exclusion OPEN.

### Pattern-walk local trivia gate: retained (br-mcm)

Actual own-inferior gdb/MI probe of ee836a6d native records24 periodic all-thread
snapshots:72 thread observations,35 waiting,23 in the pattern collector's
ancestor climb,9 SQLite,5 other. Counts are per-thread stack presence, not
recursive-frame totals or CPU shares. Pinned TS0.26.10 ts_node_parent re-descends
from the root on each call; the earlier child kind-vs-format cost dismissal was
wrong and unmeasured. MI producer was corrected before execution (token-command
punctuation, bounded queue-reader, quote-aware frame parsing);4 handshakes done,
actual native exit0 and original rows read back. Final process ticks unavailable
after exit; per-sample ticks are supporting evidence only.

One pattern/index.rs gate now checks the current node's existing trivia-kind
table. This walk starts at the root, so every surviving sub-cap ancestor has
already passed that check. The depth>MAX declaration-exempt arm runs first,
returns and remains byte-identical; no inherited state, traversal fusion, new
cache/table/dependency/schema/limits/row or error policy. General ancestor helpers
and other callers stay unchanged. Only pattern/index.rs differs among180 selected
native inputs. Original SHAee836a6d140eed0bfe601e9acfdaf70a71dccb4bc86c971ef3d392cea70ce641;
candidate SHAd307374d20938c9375f2ab19970604448a4e5488ca65dd5365f34a20bdc68587.
Same required-remote release-perf/rustc1.98.0/default features/Linux aarch64.

| Pristine initial-index case | Original p50 ms | Candidate p50 ms | Median paired change | Original/candidate p95 ms |
|---|---:|---:|---:|---:|
| complete frozen660-file corpus | 8284.453 | 4705.583 | -43.08% |8310.713 /4845.597 |
|32 ordinary core-source files | 380.193 | 211.463 | -44.69% |392.496 /219.490 |
|60 multilingual line variants | 50.805 | 47.491 | -7.56% |55.439 /52.721 |
| depth/trivia boundary control,2files | 344.950 | 38.958 | -88.78% |348.658 /48.303 |

Twenty alternating pairs/case after2 warmups, CPUs5/6/RAYON2/noembed,
warm OS/shared worker. Fullcase runs10+10 medium-call batches to avoid transport
deadline; all160 measured indexes preserve exact stats/stdout/stderr and seven
canonical stored-field projections (same surrogate-ID/empty-semantic policy as
prior producer). Reuses original canonical fingerprint routine, not a second
validator. All470584 full-corpus pattern rows still match. Ordinary/15-language
line variants retain the prior goldens;300-level declaration plus shallow
comment/raw-string control keeps exactly1 truncated file and the original rows.
No multiplication with previous wins priced in different trials.

Initial disabled-prune fault survived the two extraction goldens. Expanded the
existing generator JS/TS fixture while that fault was present: ghostItem inside
template interpolation must not enter cached signatures, while readItem stays
present; RED101 catches the gap. An independent overbroad identifier-prune fault
also fails101. Restored source byte-exact; named extraction/parser/kind/pattern
suites pass349/no ignores; rustfmt/diff pass. No new test file or golden rewrite.

Independent GNU-time3 probes/arm/case: full user CPU median12.18->5.03s,
ordinary0.51->0.18s, tiny0.02s unchanged. **Full RSS median rises485120->500168KiB
(+15048KiB/+3.10%); this is not a memory reduction.** Other RSS controls nearflat;
parallel preparation timing can change high-water without a new cache. Separate
phase walk4603.591->1037.786ms, SQLite3382.313->3331.913ms. Post-change debugger
probe naturally finishes after13 snapshots:39 thread observations,23 waiting,
10SQLite,6other,0pattern ancestor climbs. Diagnostic wall occupancy, not CPU
fractions or a complete absence certificate. SQLite writes are now the measured
next dominant phase. Cold17-query24-pair controls preserve816 replies plus24
decoder and2nonempty structural-query observations; small cold effects/tails
remain mixed and are not another cold-search or universal tail win.

Evidence/current artifact root: corrected `index-stack-probe.py`/MI/JSON and
`index-stack-verification.json`, `local-trivia-index-paired.py` with `full-a`,
`full-b`, `controls` phases, pairs/summary/depth golden, countermetric/post-stack
producers and outputs, native/source inputs and named remote build/test/fault
logs, `local-trivia-verification.json`. Runtime producers use
`python3 /home/aditya/.cache/hermes/scratch/ast-sgrep-perf-current-20261006/<producer>.py`.
Native/source/readback counts verified on worker after mutations. Scratch indexes
retained; no deletion, new dependency, kernel tuning, local heavy compile or
workspace-wide Cargo. No neural/refresh/cold-storage/platform/release certificate;
CASS5 exclusion OPEN.

### SQLite invariant-pattern-binding experiment: rejected (br-mcm)

The bounded write review found no candidate worth implementing, but source
statement counts do not prove a universal write-path ceiling. Parent tested one
distinct safe experiment within the per-row loop: keep identical pattern INSERT
text/execute order; bind all5 parameters in original order for the first row,
then retain only file_id/empty-excerpt bindings through rusqlite0.40.1 reset.
Row-dependent fields are rebound for every row. No batching, rowid, FTS, schema,
metadata, transaction, dependency or default changes.
Original d307374d native above; candidate
`dd84077c00a5f55f60addf79afe1f7d684e51212de7bb9c0ef3d5ee8af17adcc`.
Same required-remote release-perf/CPU5/6/RAYON2/noembed/warmOS/sharedworker.

| Pristine-index case | Original/candidate p50 ms | Median paired change |
|---|---:|---:|
| complete660-file corpus |4714.723 /4707.794 | -0.033% |
| ordinary32 |212.349 /209.805 | -0.518% |
|60 line variants |53.138 /53.039 | -0.491% |
| depth control |37.695 /39.356 | +2.788% |

Twenty alternating pairs/case after2 warmups,160 native indexes with complete
stats/stdout/stderr/seven canonical projections exact. Fullcase signal is noise;
ordinary order arms oppose, depth regresses. **No keep: restore original source
byte-exact**, preserving all preceding optimizations. Named store/transaction
gates pass24 with1 existing timing-quarantine ignore after restoration;
180 selected local/worker source hashes and retained d307374d digest match.
No new tests/resource/tail certification needed for this unadmitted experiment.

Source inventory from actual stored counts:1,941,866 top-level per-row execute
calls,1,397,964 in the four-insert line loop (71.99%). This is neither CPU share
nor all SQLite/FTS internal work. The child report's generic multirow/rowid
impossibility and per-file unresolvability claims are not established ceilings.
Evidence/current artifact root: `out_sqlite_write_next.md`, `binding-index-paired.py`
(`full-a`, `full-b`, `controls`), pairs/summary, candidate/before snapshots,
source/native freezes, remote build/store test logs, `binding-rejection-verification.json`.
Exact runtime producers use the existing worker scratch root and `python3`.
Candidate/artifacts retained externally, no files deleted; CASS5 exclusion OPEN.
Retry only with a changed mechanism and same-host pairs showing a material
whole-index gain with unchanged IDs, fields, transaction/error/empty-input
semantics and no representative regressions. No SQLite/schema redesign inferred.

### Warm corpus borrowed-row extension: rejected (br-mcm)

Bounded source review established a feasible extension of the existing cold
decoder into `LineCorpus::load`, not a speed win. Parent moved the unchanged
projection/column0→3 decoder into an existing lower store leaf, using a Rows
loop whose borrows are consumed before advancing SQLite. Only first-file
metadata is owned; packing, tokens, caps, generation, query/SQL ordering stay
unchanged. Two existing product files, no store→search dependency. Current
d307374d native above is the original for both blocks; no previous cold-row
borrowing result is rebranded as new work.

Fresh own-inferior GDB entry/finish diagnostics on this original:3 observations
`LineCorpus::load`174.293–175.641ms; trigram refresh143.004–153.547ms;
enclosing warm375.790–389.234ms. Wall boundaries, not CPU shares or additive
phase accounting. `corpus-borrow-baseline-attribution.py`/JSON/raw worker logs.

| Public serial Code Mode workload | Original shared-decoder extension paired change | Single-language-read refinement paired change |
|---|---:|---:|
| small1 / small8 | +1.753% / +0.874% | -0.818% / +0.408% |
| default1 / default8 | +0.278% / +0.414% | -1.180% / -0.894% |
| large1 / large8 | +0.383% / +0.349% | -1.349% / -0.484% |
| unique17 | +0.828% | +1.186% |
| catalog control | +2.364% | +0.666% |

Twenty-four alternating measured pairs/workload/variant after2 warmups:
384 pairs/768 native batches, stable complete response fields exact versus
the original within both blocks. Original extension native
`c573ca9c0d96193604aa1f08bcade8c2023d4707183ed994943ff34bf07bf3e8`;
refined `689f81fc11d918c9ad26c979dfb1d6ff9fcfa07698242158b861174b605687c5`.
The one bounded refinement reuses the language ValueRef already read for
NULL admission instead of retrieving that field twice, while preserving typed
fallback/error order. Absolute timings drift across blocks; do not compare
their medians or call a small/order-sensitive delta a universal improvement.

Actual `libmemusage.so` characterization (3 probes/arm, same small1 payload):
malloc calls2,592,753→1,583,890 (−1,008,863/−38.91%);
cumulative heap bytes175,654,228→148,247,433 (−15.60%) for original extension.
Refinement has the same malloc count and148,247,489 cumulative bytes.
Heap peak59,933,508→59,933,509/59,933,517 bytes is unchanged, not RAM saving.
Separate3 GNU-time samples/arm: user CPU median0.32s for all, RSS
184,924→184,920/184,856KiB is effectively flat (not memory certification).
The allocation trace is real churn reduction; **it did not earn a product keep**.
Complete path lacks a representative net win and unique17 regresses in both
blocks. Both productionfiles restored byte-exact, all four prior keeps intact.

Restored named required-remote `line_corpus_unit`6 +
`literal_warm_cold_parity`15 pass/no ignores;180 selected local/worker source
hashes and retained d307374d digest verified. No extra cap/error mutations,
new tests or cold full replay for unadmitted code. Producer scripts, frozen
sources/natives, summaries/pairs/goldens, actual allocator/resource traces,
build/test logs and `corpus-borrow-rejection-verification.json` remain at the
current artifact root. The first trace parser expected colon vs actual pipe;
fixed/resumed saved verified observations without erasing failures/rerunning.
CASS5 history OPEN. Retry only with a changed mechanism and same-host
whole-call/CPU/peak-RSS evidence showing a material gain across representative
cases, preserving complete row decode/errors before cap/grouping and packed
bytes/tokens/generation/retrieval semantics. No warming-policy change inferred.

### Exact vocabulary preload: bounded deferral, no candidate (br-mcm)

Parent checked the bounded review against cached libsqlite3-sys0.38.1
`sqlite3.c:268464–268522,268686–268818`: xBestIndex forwards term equality/
range constraints; the ROW scan increments aDoc per posting while colUsed
already avoids unused cnt position counting. This supports the existing
SELECT term/doc projection, not an impossibility proof for all future SQL.
Direct doc/length predicates do not give this xBestIndex a posting-pruning
constraint. Do not infer every optimizer-derived LIKE/range plan was measured.
Nor does a source comment establish a current point-probe timing certificate.

Own-inferior GDB/MI3 current d307374d public small1 warm calls, CPU5/noembed,
23 stopped snapshots/run, native exit0 and unchanged source-index size/mtime.
69 thread observations:29 corpus-load,22 FTS5 vocabulary backend,1 vocab
column getter/mutex,17 other. All22 backend frames reach fts5VocabNextMethod
through Rows::next/preload_vocab; stack heads show segment iteration, rowid
advance, varint parsing and merge comparisons. The1 getter remains separately
counted, not silently assigned to the backend. **Wall occupancy only**, no CPU
percentage, exact per-row timing split or whole-stage lower bound. Supporting
existing entry/finish143.004–153.547ms refresh wall above; not a speed result.

No low-complexity candidate justified changing this path: key/reserve shaping
does not remove the observed backend posting work. Selective/on-demand loads
need distinct error/unknown/generation/warming proof; no engine, schema,
tokenizer, cache or warming-policy changes authorized. Leave code unchanged.
Retry when a new observed mechanism bypasses repeated physical work while
preserving all typed validation/error precedence, exact term/doc selection,
decoy defense, generation/armed/unavailable state and representative replies,
without broadening those boundaries. No universal irreducibility claim.

Evidence/current artifact root: `out_vocab_warm_next.md` (bounded source-only
report), `vocab-warm-stack.py` (`0`,`1`,`2`), raw MI/JSON and
`vocab-warm-stack-summary.json`. Native runtime diagnostics only, no Rust
changes/builds/tests here. CASS5 exclusion remains OPEN. Separate early-index
diagnostic `remaining-index-stack.py`/MI/JSON/summary identifies the next
eligible language-walk unit; its24-snapshot cap intentionally biases early
preparation and cannot certify SQLite disappearance or CPU shares.

### Terminal identifier recording shortcut: rejected (br-mcm)

Profile-backed three-line candidate in pattern/index.rs: return after pushing
an identifier signature, avoiding redundant declaration/call classification
while the caller still recurses. Mechanically checked11 identifier kinds,
10 call kinds and41 declaration keys (40 table entries + dynamic specials)
have empty intersections. Existing trivia/depth exemptions, node_text failure
fallthrough, signatures/first-dedup winner and excerpts unchanged. One product
file; no defaults, parser, schema, cache or result changes.

Original retained d307374d native; candidate
`cfbdbc8267299161befc7e795bd3aa39dff02546d147bf2d7ad3c318cb36d1cc`.
Required-remote release-perf, sameCPU5/6/RAYON2/noembed/warmOS controls.
20 alternating measured pristine-index pairs/case after2 warmups:
80 pairs/160 indexes, full stdout/stderr/stats/seven stored projections exact.

| Case | Original/candidate p50 ms | Median paired change |
|---|---:|---:|
| full660 |5382.460 /5399.903 | +0.245% |
| ordinary32 |243.039 /244.895 | +0.476% |
| multilingual line variants |54.559 /52.962 | -3.137% |
| depth control |42.289 /41.909 | -0.570% |

**Reject:** no material full/ordinary win; the multilingual improvement is
bounded to that control, ordinary/depth order arms oppose. Restore original
source byte-exact, preserve all four previous keeps. Named restored language
gates pass candidate_kind_table8/extraction_goldens2/pattern337 =347, no
ignores;180 selected local/worker source hashes and retained native verified.
No extra mutations/resource/cold certificates for unadmitted code. The child's
MoonBit test pins declaration/call rows, not the proposed terminal-ident-drop
fault: do not claim a kill that was not run. It also enumerated7 pushsites
while calling them6; source proof/data take precedence over reviewer prose.

Evidence/current artifact root: `out_identifier_terminal_next.md`,
`identifier-terminal-kind-proof.json`, before/candidate snapshots, input/native
freeze, build/lang/restored logs, `identifier-terminal-index-paired.py`
(`full-a`,`full-b`,`controls`), pairs/summary and
`identifier-terminal-rejection-verification.json`. Nothing deleted;
CASS5 history remains OPEN. Retry only with a changed mechanism and same-host
whole-index improvement across full and ordinary cases while preserving the
exact rows/stats/truncation and multilingual controls; do not repeat source-only
comparison counts as a speed claim.

### Signature last-line ownership reuse: rejected (br-mcm)

Two-file prototype replaces per-file HashSet<(signature,line)> with
HashMap<signature,last-line>. Same root-started preorder/byte-ordered children
and complete-source points imply nondecreasing attempt lines, preserving
first-winner admission/order/excerpts/depth policy. Parent checked worker's
pinned tree-sitter0.26.10 children()/next-sibling implementation after the
child lacked its local extraction. No unrestricted edited/included-range tree
or oversized-source parse-failure claim. Real507-file pattern-row ordering has
no backward examples;470,584 rows versus117,257 distinct per-file keys suggest
353,327 accepted key copies/3,905,468 key bytes avoidable, not traced mallocs.

Original retained d307374d; candidate
`8cc57e4b536b39ca805026027104048693b04e2138741c56cf85ea61ea635775`.
20 alternating pristine-index pairs/4cases after2 warmups, sameCPU5/6/RAYON2/
noembed/warmOS:80 pairs/160 indexes with exact stats/stdout/stderr/seven stored
projections.

| Case | Original/candidate p50 ms | Median paired change |
|---|---:|---:|
| full660 |4687.195 /4660.011 | -0.370% |
| ordinary32 |208.419 /206.874 | +0.252% |
| multilingual line variants |49.447 /48.433 | -1.974% |
| depth control |39.279 /39.809 | +1.501% |

Separate3 GNU-time samples/arm/3cases: full user CPU5.02→4.96s, but full
RSS500,092→507,340KiB (+7,248/+1.45%). Ordinary CPU0.18→0.17s/RSS
32,664→32,560KiB; multilingual0.02→0.01s/27,768→27,356KiB. Small/coarse
samples are characterization, not hard RAM/CPU certificates. Separate full
phase wall: walk1,045,730→1,031,330us, SQLite3,337,333→3,294,216us;
unchanged SQLite-code phase variation must not be attributed to this lever.

**Reject:** no material representative speed win and full RSS higher, despite
less duplicate-key ownership. Both productfiles restored byte-exact; prior
four keeps intact.347 named restored language passes/no ignores;180 selected
local/worker inputs and retained native digest verified. No additional
mutation/newtest/cold certificate for unadmitted code; stale RA type errors
during the two-file restore are not final compiler failures.
Evidence/current artifact root: `out_signature_lastline_next.md`, cardinality,
before/candidate snapshots, input/native freeze, lang/build/restored logs,
`signature-lastline-index-paired.py` (`full-a`,`full-b`,`controls`), pairs/
summary, resources producer/raw worker samples/phase logs and
`signature-lastline-rejection-verification.json`. Nothing deleted; CASS5 OPEN.
Retry only with a changed mechanism and same-host material whole-index or
peak-RSS improvement across full/ordinary cases retaining exact rows/stats/
truncation and multilingual controls. Key cardinality is not a net-gain claim.

### Scoped native-call range membership: retained (br-mcm)

Profile first: original d307374d public `pattern:ping($ARG)` scaling on valid
synthetic Rust calls;5 samples/size after1 warmup. Owned-inferior stacks show
11 observations in `push_match_with_captures`'s full-output `.any` loop via
`walk_calls`, alongside36 other-pattern/3 parser/109 other-thread observations.
Stopped-wall occupancy, not CPU shares. The ordinary source/default indexed
lane is not the same workload as this high-cardinality native call walk.

One production file, structural.rs: the Call-only branch starts a local exact
`HashSet<(byte_start,byte_end)>` and threads it through its sole preorder walk.
Both emitters finish the original gates/capture unification before admission;
exact Eq membership replaces the Vec scan, preserving first captured winner,
all output order/ranges/captures/hit construction. Failed captures never poison
the set. Other emitters/general helpers unchanged; no adjacency/monotone/hash-only
assumption, result/cap/schema/cache-policy/parser/default change.
Original d307374d; retained native
`abe63db667c73b967dcce34b1df32c0a1247ee79946752e8785bd91263f7548e`.
180 selected local/worker inputs match; only structural.rs differs from original.

| Public native workload | Original/candidate p50 ms | Median paired change |
|---|---:|---:|
|256 synthetic calls |6.799 /6.830 | -0.499% (opposing order arms) |
|1024 calls |19.633 /19.315 | -1.770% |
|4096 calls |73.321 /68.875 | -5.638% |
|16384 calls |356.096 /287.920 | -19.080% |
| real frozen-tree node_text capture call |733.252 /737.467 | +0.490% |
| real function capture control |863.038 /862.771 | -0.045% |
| real struct capture control |267.138 /266.358 | -0.323% |

24 alternating pairs/case after2 warmups:168 pairs/336 native replies, complete
rendered JSON bytes identical, source indexes unchanged. CPU5/warmOS/noembed,
existing frozen indexes; synthetic calls kept distinct, not fewer-match work.
Limit16 controls rendering, not a claim that all16k matches were returned.
Separate3 GNU-time probes/arm/4cases:16k user CPU0.33→0.26s and peak RSS
52,832→47,364KiB;4096 CPU0.06→0.06s/RSS18,676→18,548KiB.
Real-call0.71→0.72s/RSS63,324→63,448KiB. Keep the real-call slowdown,
small mixed order arms and sample granularity; no universal CPU/RAM/nonregression
or shipped-size/cold-storage/hard-tail/all-matcher complexity certificate.

Restored named required-remote lang gates:pattern337/extraction_goldens2/
candidate_kind_table8 =347 passes/no ignores. Existing
f105_plain_call_rest_slots_bind_like_sg kills independent constant-range-key
faults in both emitters: None arm[1] vs[1,2,3], slots arm[2] vs[2,3], semantic
101 each. Candidate restored byte-exact; source/native readback verified.
No new test required; no claimed drop-duplicate-gate mutant kill for a suite
without a same-range call fixture. Counter replay:17 cold +24 decode cases
exact across both natives (82 invocations); slots real calls at limits1/16/256
and synthetic16384 at limit256 give identical full rendered replies.

Evidence/current artifact root: `out_call_dedup_next.md`, baseline scaling/
stack producers/MI/JSON, source/input/native freezes, build/lang/restored/mutant
logs, `call-dedup-paired.py`, pairs/summary/goldens, resource probes/raw worker
samples, `call-dedup-counter-replay.json`, `call-dedup-verification.json`.
Required-remote commands use locked/offline -p ast-sgrep-lang named tests and
-p ast-sgrep-cli --bin asgrep --profile release-perf on aarch64/jobs2.
Fifth current native keep; prior four and all dirty work preserved. CASS5 OPEN.
Retry broadening only when another measured emitter has the same fresh-output/
exclusive-writer semantics and exact equality/order/capture proof, with matched
native improvements and resource/ordinary controls; do not generalize blindly.

### PHP member-slot range membership: retained (br-mcm)

Next sibling profiled independently on prior retained abe63d native:3 owned
inferior runs/12 snapshots each yielded9 observations in the member walk's
whole-output `.any` loop,11 other-pattern/6 parser/1 parent/69 other-thread
observations. Wall occupancy, not CPU share. The intended independent review
failed HTTP503 before doing work; parent traced fresh/exclusive output, sole
root/self-recursive entry, both emitters and hit_for_node's actual Node spans
inline. No independent-review certificate claimed.

One existing file structural.rs, final scope **member slots emitter only**:
root-local exact range set, insertion after receiver-meta and argument-slot
unification succeeds. Slots option is immutable across a walk, so emitters
never mix. Same first captured winner/order/raw spans/excerpts and every
connector/nullsafe/statement/continuation/refusal gate. None emitter remains
the original push_match; plain-call keep and all other helpers/lanes unchanged.
Original abe63d; final retained native
`7d539269296048faffbc62580bc196f76390f2353893767e2ac05fd54d9dc141`.
180 selected local/worker input hashes match, sole structural.rs delta.

| Public native workload | Original/candidate p50 ms | Median paired change |
|---|---:|---:|
|256 synthetic PHP members |4.940 /4.915 | +0.386% (opposing order arms) |
|1024 members |11.636 /11.473 | -2.538% |
|4096 members |38.063 /34.289 | -9.141% |
|16384 members |199.170 /132.410 | -33.546% |
| retained plain16384-call control |290.637 /289.384 | +0.236% (mixed orders) |
| real Rust call control |740.110 /740.194 | +0.046% |
| real Rust function control |864.943 /865.998 | +0.250% |
| real PHP member absence control |60.088 /60.353 | +0.342% |

24 alternating pairs/case after2 warmups:192 pairs/384 complete rendered JSON
replies exact, CPU5/warmOS/noembed. High-cardinality synthetics are not a
real-member-corpus gain: this frozen PHP fixture defines methods but invokes
none, so its case is explicitly negative. Separate3 GNU-time probes/arm/4cases:
16k user CPU0.17→0.10s, peak RSS63,780→64,372KiB (+592/+0.93%);
4096 CPU0.02→0.02s/RSS22,956→22,956KiB. Disclose added range storage,
small-control noise and no universal CPU/RAM/nonregression/tail certificate.

347 final required-remote named language passes/no ignores. Existing f85a
flat/chain-rest test kills slots constant-key fault [2] vs[2,3,5,8], semantic101.
Initial None-key fault survived focused f85a and all337 pattern tests: scope
was narrowed back to its original emitter, not patched with test ceremony.
Initial both-emitter f611 native/pairs remain historical, NOT final-code timings.
Initial benchmark's nonempty PHP expectation and dangling-arrow positive
expectation were wrong on baseline CLI; preserved failures/relabelled negative
cases, no product tests loosened/skipped. Final replay8 PHP boundary queries
plus17 cold/24 decode cases gives98 native invocations with exact replies;
statement/embedded/rest/nullsafe/repeated-meta/absence faces retained.

Evidence/current artifact root: member scaling/stack scripts and JSON/MI,
input/source/native snapshots, initial member-dedup trial/failure logs,
`member-slots-paired.py`, final pairs/summary/goldens/resources,
slots mutant/None-survivor logs, final lang/build logs, counter replay and
`member-slots-verification.json`. Sixth native keep; prior five/dirty work intact,
no schema/default/cap/dependency/cache-policy changes; CASS5 history OPEN.
Retry broadening only with another profiled exclusive-output emitter's exact
span/capture/refusal proof, meaningful existing discriminator and matched
public gains/ordinary controls/resources; do not generalize from PHP slots.

### Call-kind-first ancestor gate: retained (br-mcm)

Fresh profiles of the sixth retained native locate repeated parent lookups in
`call_match_path`, whose only caller walks every node, including non-calls.
Two owned-inferior real-call runs yield48 snapshots/142 thread observations:
65 contain is_in_comment_or_string,6 parser,15 other-pattern,56 other. These
are stopped-wall observations, not CPU percentages. The PHP member profile
is separate and does not justify changing that lane.

One guard reorder in calls.rs: reject non-call kinds before ancestor lookup.
Both predicates are read-only; every actual call still receives the complete
comment/string veto before the unchanged callee resolver. Traversal, first
winner, captures, ranges, ordering, parsing and all refusal/cap/cache policies
are unchanged. No subtree pruning or new helper/cache/dependency. Optional
call siblings already use this predicate order. Original native is the sixth
keep `7d539269296048faffbc62580bc196f76390f2353893767e2ac05fd54d9dc141`;
candidate `e5608e68d4d2207aadbccc2b4846edf5795227b7edcd57ca2e0ab09bb1094b2a`.
180 selected native inputs match dispatcher/worker, with only calls.rs differing
from original. Same release-perf/rustc1.98.0/Linux aarch64/default features.

| Public native workload | Original/candidate p50 ms | Median paired change |
|---|---:|---:|
|256 synthetic Rust calls |7.190 /4.663 | -35.630% |
|1024 calls |19.572 /9.542 | -51.956% |
|4096 calls |70.050 /28.605 | -59.218% |
|16384 calls |287.949 /108.600 | -62.249% |
| real frozen-tree node_text capture call |739.770 /163.644 | -77.866% |
| real function capture control |864.212 /863.715 | +0.064% |
| real struct capture control |267.375 /267.330 | -0.079% |
| PHP16384 member control |127.320 /127.644 | +0.231% |
| real PHP member absence control |59.999 /60.083 | +0.102% |

32 alternating pairs/case after2 warmups:288 pairs/576 complete rendered replies
exact, CPU5/warmOS/noembed/fresh process/existing indexes; DB size/mtime unchanged.
The real call is one frozen-source query, not an all-query gain. P95s/maxima and
both order arms remain in kind-first-summary.json. Negative countermetrics:
calls256 candidate maximum13.553ms versus original9.500ms despite lower p95;
PHP member p95 rises131.928→133.260ms. No hard-tail/nonregression certificate.

Separate3 GNU-time probes/arm/4cases preserve exact replies. Real-call median
user CPU0.72→0.14s, peak RSS60,944→60,920KiB;16k calls0.27→0.09s,
RSS46,920→46,920KiB. PHP member median CPU0.10s/RSS64,032KiB unchanged,
but one candidate RSS sample69,856KiB exceeds all three originals
(64,020/64,032/64,160KiB); do not infer universal RAM reduction. Post-profile
two runs yield20 snapshots/58 observations,9 ancestor/2 parser/14 other-pattern/
33 other, native exits0; unequal diagnostic sampling is not a CPU-share ratio.

Required-remote lang pattern337/extraction_goldens2/candidate_kind_table8 =347
passes, zero failures/ignores. Existing tests retained without new assertions or
mutations: this pure short-circuit reorder preserves both candidate predicates,
not a new matching contract. Both frozen natives additionally replay17 cold
queries and24 decoder witnesses exactly (82 invocations), including exit classes.
Scoped rustfmt/diff checks pass. All six preceding keeps and dirty work intact.

Evidence root `/Users/aditya/.hermes/artifacts/ast-sgrep-perf-current-20261006/`:
current-match-stack.py/MI/JSON, calls.kind-first-before, kind-first-{paired,
resources,stack,counter-replay}.py, pairs.jsonl, summary/resources/stack JSON,
freeze/input manifests and kind-first-verification.json. Exact worker runtime:
`python3 /home/aditya/.cache/hermes/scratch/ast-sgrep-perf-current-20261006/kind-first-paired.py`.
Named build/test RCH commands and actual exit/count receipts are in verification;
no workspace/local compilation, release/platform/neural certificate or commit.
CASS lookup remains OPEN code5. Retry broader ancestor elision only for a new
profiled caller with a proven inherited-trivia invariant and exact native replies;
this keep does not authorize bypassing ancestor checks on actual calls.

## Chain traversal work budgets: original baseline (br-mcm)

**Status: original checkpoint only; no cap existed at that checkpoint.**
The subsequent retained resource contract is recorded below; no speed admission.
Owner approved explicit work/exhaustion semantics separately from output limits.
Native baseline `chain-original-098f5bc5`: executable SHA256
`098f5bc57095d00fc4e7b43f4d03e244792f686215d19832d45de441e31ec941`.
RCH built `ast-sgrep-cli --bin asgrep --locked --offline --profile release-perf`
for aarch64 Linux with two jobs and existing rustc 1.98.0 / LLVM 22.1.8.
All 178 regular-project Rust/manifest/static-JSON hashes match the worker;
this is not a complete registry/toolchain Cargo-closure receipt.

Synthetic source is indexed by that actual CLI without embeddings. CPU5 on
shared spark-1672 is an X925; no OS tuning or frequency locking. Storage is
OS-warm. Each row has 12 fresh-process, 12 sticky-session and 12 separate
native-RSS observations after two timing warmups. RSS uses GNU time around the
native process, not the contaminated pre-exec Python-coordinator wait4 highwater.
CPU is wait4 child CPU including launch. Sticky lifetime HWM is observed at
reply boundaries, not a per-call peak. Initial tainted-RSS and void exact-defs
many-seed observations are retained in `measurement-corrections.json`.

| Fixture | Fresh p50 ms | Fresh CPU p50 ms | Fresh peak RSS p50 MiB | Sticky p50 ms | CLI discovered/returned | Sticky discovered |
|---|---:|---:|---:|---:|---:|---:|
| empty | 1.748 | 1.675 | 7.459 | 0.316 | 0/0 | 0 |
| ordinary | 2.404 | 2.318 | 7.473 | 0.807 | 13/13 | 40 |
| cycle | 2.126 | 2.036 | 7.473 | 0.414 | 5/5 | 12 |
| outgoing-60 | 3.480 | 3.395 | 7.660 | 1.100 | 61/61 | 61 |
| incoming-60 | 3.125 | 3.053 | 7.648 | 1.104 | 61/61 | 61 |
| outgoing-600 | 12.265 | 12.162 | 10.117 | 8.112 | 601/100 | 601 |
| incoming-600 | 13.117 | 12.861 | 10.305 | 8.317 | 601/100 | 601 |
| outgoing-6000 | 107.002 | 106.733 | 30.961 | 78.825 | 6001/100 | 6001 |
| incoming-6000 | 110.876 | 110.626 | 33.676 | 81.424 | 6001/100 | 6001 |
| duplicates-6000 | 70.187 | 69.876 | 13.635 | 60.036 | 2/2 | 2 |
| imports-600 | 8.839 | 8.736 | 9.830 | 3.363 | 601/100 | 601 |
| imports-6000 | 68.017 | 67.782 | 27.814 | 32.036 | 6001/100 | 6001 |
| deep | 3.760 | 3.654 | 20.035 | 66.244 | 21/21 | 5461 |
| many-seeds | 2.161 | 2.100 | 7.469 | 0.681 | 2/2 | 40 |

CLI depth is 2/top_n 1; sticky uses deeper depths for ordinary/cycle/deep and
20 seeds for many-seeds. Those cells are different workloads, not paired speed
comparisons. All original payload fields, topology/endpoints and repeat replies
are checked; no global quality, cold-storage, platform or release certificate.
Full tails, CV, CPU, HWM and source/index/query hashes are in the receipt.

Perf events are blocked by perf_event_paranoid=4. A bounded owned-child GDB
capture with four one-shot source markers places the duplicate-hub root's
material cost in outgoing-call/definition expansion; debugger wall intervals
are diagnostic, never substituted for native benchmark timings or CPU samples.

Artifacts (not a portable reproducible-in-tree harness): Hermes scratch
`chain_budget_20261005/{baseline-verification.json,baseline-corrected.jsonl,
fixtures-corrected.json,baseline-project-inputs.json,characterize.py}`; worker
`/home/aditya/.cache/hermes/scratch/chain_budget_20261005/`. Exact command:
`python3 /home/aditya/.cache/hermes/scratch/chain_budget_20261005/characterize.py measure --samples 12 --phase baseline-corrected`.
No cap defaults or performance claims are inferred before the deciding paired
comparison; ordinary equivalence and saturated coverage/bias need separate proof.

### First work-cap implementation: under-budget regression, not admitted

**Status: rejected as the retained implementation; cursor refinement below.**
Frozen source-bound `asgrep-candidate` is paired against the original above on
CPU5, 24 alternating OC/CO observations per workload and arm (12 in each order).
There are 14 workloads in fresh, sticky and separate native-RSS paths. Complete
core payloads are exactly equal in 19 unsaturated workload/mode combinations;
nine saturated combinations deliberately omit exploration and report it.
These gains are less work, not behavior-preserving speedups.

| Workload | Fresh original/candidate p50 ms | Median paired change | Sticky original/candidate p50 ms | Median paired change | Discovered original/candidate (fresh; sticky) |
|---|---:|---:|---:|---:|---|
| empty | 1.790/1.790 | +0.96% | 0.231/0.227 | +0.20% | 0/0; 0/0 |
| ordinary | 2.405/2.563 | +3.86% | 0.875/1.117 | +23.96% | 13/13; 40/40 |
| cycle | 2.205/2.331 | +7.44% | 0.506/0.750 | +44.72% | 5/5; 12/12 |
| outgoing-60 | 3.418/4.643 | +36.63% | 1.329/2.559 | +93.17% | 61/61; 61/61 |
| incoming-60 | 3.032/4.280 | +42.10% | 1.302/2.556 | +92.88% | 61/61; 61/61 |
| outgoing-600 | 12.723/25.277 | +98.30% | 8.538/20.525 | +139.78% | 601/601; 601/601 |
| incoming-600 | 12.877/25.556 | +98.40% | 8.458/20.581 | +142.83% | 601/601; 601/601 |
| outgoing-6000 | 107.271/37.816 | -64.74% | 79.973/31.182 | -61.06% | 6001/2731; 6001/2731 |
| incoming-6000 | 110.248/26.292 | -76.20% | 82.576/19.678 | -76.02% | 6001/6001; 6001/6001 |
| duplicates-6000 | 70.919/33.658 | -52.51% | 61.208/27.662 | -54.85% | 2/2; 2/2 |
| imports-600 | 9.222/21.591 | +135.34% | 3.644/15.589 | +326.34% | 601/601; 601/601 |
| imports-6000 | 68.857/16.475 | -76.12% | 32.435/8.437 | -73.88% | 6001/1638; 6001/1638 |
| deep | 3.815/3.837 | +0.96% | 65.220/34.084 | -47.88% | 21/21; 5461/1929 |
| many-seeds | 2.026/2.053 | +2.33% | 0.842/1.663 | +91.58% | 2/2; 40/40 |

Medium complete workloads regress materially; the cheaper saturated cases do
not compensate for that. The first candidate is therefore not admitted. Its
executable identities, full order arms/tails, CPU, native RSS, sticky HWM/startup,
returned-node/edge overlap and all payloads are retained in Hermes scratch
`chain_budget_20261005/{paired-default.jsonl,paired-default-summary.json}` and
worker scratch of the same name. Exact command:
`python3 /home/aditya/.cache/hermes/scratch/chain_budget_20261005/characterize.py paired --samples 24 --phase paired-default`.

Strace shows essentially unchanged syscall counts; extra time is predominantly
user CPU. A Python SQLite3.45.1 same-index LIMIT diagnostic is retained separately
and is not native attribution or a full explanation. Perf events are forbidden
and valgrind is unavailable; no privilege/dependency changes were made. The next
single mechanism to price is bounded cursor admission using the original ordered
SQL shape, not raising budgets, removing ordering or redesigning indexes.

### Retained chain work-budget cursor admission (br-mcm)

**Status: measured dirty-main resource contract; not a speed keep or portable
reproducible-in-tree harness.** Retained `store/sql.rs` uses the original cached
SQL and cuts the cursor before owned decoding. SQLite matching/sorting remains
uncapped. Default work is8192; host overrides are1..=1,000,000, independently of
output limit. Retrieval, seed resolution and native cache warm-up remain outside
the post-seeding budget. Outgoing, incoming, then import admission remains ordered,
not fair. The failed SQL-LIMIT prototype above is preserved, not erased.

Same Linux aarch64 worker, Rust1.98.0 `release-perf`, default features, CPU5,
OS-warm files and bounded threads as above. Original SHA256
`098f5bc57095d00fc4e7b43f4d03e244792f686215d19832d45de441e31ec941`;
retained SHA256 `488a14b87468261664959b0be27c44df55dc8d07e5de040833d3cca79f81c165`.
All178 regular native project-input hashes match current source, not a complete
Cargo dependency-closure receipt. Producer:
`python3 characterize.py paired --binary /home/aditya/.cache/hermes/scratch/chain_budget_20261005/asgrep-cursor --samples 24 --phase paired-cursor`.
Driver, source/index manifest, binaries, full replies, RSS probes and raw samples
are retained under `/home/aditya/.cache/hermes/scratch/chain_budget_20261005/`;
local mirror `/Users/aditya/.hermes/cache/scratch/chain_budget_20261005/`.
Each row has24 original/candidate pairs (12OC,12CO) per fresh/sticky mode,
plus24 independent GNU-time RSS pairs. Sticky times exclude startup; HWM is
process-lifetime, not per-query peak. Perf remained denied. P95 is empirical
nearest-rank among24 observations, not a hard latency bound.

| Workload | Fresh original -> cursor p50 ms | Sticky original -> cursor p50 ms | Fresh original -> cursor p95 ms | Independent RSS original -> cursor p50 MiB | Fresh / sticky partial |
|---|---:|---:|---:|---:|---|
| empty | 2.096 -> 2.165 | 0.238 -> 0.234 | 2.761 -> 2.442 | 7.473 -> 7.467 | no/no |
| ordinary | 2.581 -> 2.571 | 0.858 -> 0.866 | 2.898 -> 2.891 | 7.473 -> 7.473 | no/no |
| cycle | 2.236 -> 2.212 | 0.482 -> 0.476 | 3.489 -> 2.526 | 7.473 -> 7.621 | no/no |
| outgoing-60 | 3.419 -> 3.358 | 1.254 -> 1.265 | 3.654 -> 3.766 | 7.664 -> 7.789 | no/no |
| incoming-60 | 3.464 -> 3.563 | 1.269 -> 1.264 | 3.669 -> 3.718 | 7.648 -> 7.836 | no/no |
| outgoing-600 | 13.004 -> 12.996 | 8.416 -> 8.429 | 13.299 -> 13.434 | 10.109 -> 10.117 | no/no |
| incoming-600 | 13.133 -> 13.214 | 8.603 -> 8.544 | 13.630 -> 13.580 | 10.293 -> 10.305 | no/no |
| outgoing-6000 | 107.347 -> 37.649 | 79.425 -> 31.131 | 108.480 -> 38.936 | 30.961 -> 24.867 | yes/yes |
| incoming-6000 | 110.827 -> 18.717 | 82.253 -> 12.043 | 111.675 -> 19.354 | 33.668 -> 30.363 | yes/yes |
| duplicates-6000 | 71.091 -> 33.391 | 60.588 -> 27.529 | 72.004 -> 33.758 | 13.633 -> 10.840 | yes/yes |
| imports-600 | 8.972 -> 9.031 | 3.564 -> 3.537 | 9.512 -> 9.383 | 9.832 -> 9.836 | no/no |
| imports-6000 | 68.966 -> 15.999 | 32.866 -> 8.384 | 69.508 -> 16.464 | 27.816 -> 23.641 | yes/yes |
| deep | 3.768 -> 3.850 | 66.244 -> 24.403 | 4.060 -> 4.186 | 20.027 -> 20.035 | no/yes |
| many-seeds | 2.342 -> 2.355 | 0.838 -> 0.843 | 2.557 -> 2.618 | 7.477 -> 7.656 | no/no |

All19 nonpartial workload/mode combinations preserve complete original payloads;
nine report incomplete traversal. Cursor and rejected prototype payloads/counters
match in every combination. Saturated reductions represent less explored work,
not equivalent-search acceleration. Tiny ordinary sticky marginal p50 increases
by about0.008ms; its median paired change is+6.4%, with opposing order-arm effects.
RSS does not uniformly fall. No uniform latency/memory non-regression certificate.

`python3 boundary-probes.py` exercises an isolated node at work1/3/4/5/1,000,000:
exact work4 completes without false truncation. A mixed outgoing/incoming/import
fixture at work8 returns only calls; raising work recovers all three relations.
Four fan-out cases at512/8192/100,000 (three exploratory observations each, not
admission percentiles) recover the full original payload at100,000. Lowering work
does not bound native warm-up, seed resolution, SQL CPU or total byte/RAM costs.
Full tails, CPU, paired order arms, CV, startup/HWM, returned-edge/node overlap and
boundary/coverage sweeps are in `paired-cursor-summary.json`,
`boundary-bias-curve.json`, `verification.json` and retained raw logs. Core zero is
source-reviewed, not newly runtime-qualified by these host-positive probes.
No cancellation, full-workspace, foreign-quality or cross-platform certificate.

## Binary-safe literal SQL: correctness ready, latency tradeoff open (br-mcm)

**Status: working-tree correctness repair; not admitted as a speed keep.**
Warmed scans now contain the entire match within one indexed record. Cold SQL
literal/word gates use full-byte `INSTR` rather than NUL-terminated `GLOB`, and
NUL-bearing needles bypass the FTS query parser. Existing indexes already retain
post-NUL text, as a real FTS-backed regex control verifies. Small/large,
filtered/unfiltered and CS/CI regressions pass. Five named core targets report
26 unique passes without skips. Separate warm literal/file-discovery span faults,
SQL/trigram prefix truncation and removal of the needle NUL guard are rejected.
The last three mutations were run against the text-INSTR implementation before
the behavior-preserving byte-mode refinement; the final byte variant passes the
same five complete targets.

Native timing uses 23 frozen ordinary queries, 24 AB/BA pairs per query,
two warmups, CPU 5 on shared spark-1672, an existing frozen self index and OS-warm
storage. Each candidate trial validates every complete stdout reply against its
original; **1104 measured replies** per experiment are equal. This is fresh
process timing, not a warm session, cold storage or a neural/release certificate.
Compiler: rustc 1.98.0, aarch64 Linux, default features, `release-perf`.
186 regular project compiler-input hashes match on the worker; only `literal.rs`
differs between native arms. RCH's optional source-content receipt refused the
`CLAUDE.md` symlink before compilation, so provenance here is the independently
checked regular-project manifest, not a successful full Cargo-closure receipt.

| Native arm | Executable SHA256 |
|---|---|
| Original GLOB | `9149f5197b48f2b23b7c0ea272a2c1a71bd16f7425307927ce03e733a55c4197` |
| Text INSTR, rejected for ordinary-scan cost | `af65b268cf6184f8f0cb3f57ee47c47c59c13ee6dc775ad7cba6085680241335` |
| Byte INSTR, current correctness-ready tree | `9f7b02ea49ab6a2a53d2beedabdfb7d72c469c3f59008df0880a5c07bfaa1db6` |

| Short literal control | Original / text INSTR median ms | Text change | Original / byte INSTR median ms | Byte change |
|---|---:|---:|---:|---:|
| `literal:as` | 27.869 / 31.900 | +14.46% | 27.910 / 28.685 | +2.78% |
| `literal:xy` | 24.242 / 31.001 | +27.88% | 23.963 / 26.410 | +10.21% |
| `literal:ab` | 27.646 / 31.689 | +14.62% | 24.550 / 26.618 | +8.43% |

The paired-median byte costs are +3.34%, +10.82%, +3.50%, respectively;
retain these beside marginal medians because drift changes the `ab` aggregate.
For `xy`, nearest-rank p95 is 24.691 / 27.235 ms. Other short controls have mixed
small tails (for example `literal:BEGIN DEFERRED` p95 3.029 / 3.232 ms); eight
conceptual marginal medians improve 1.56–2.91%, but this is not a claimed general
speedup. With 24 observations, p95 is descriptive, not SLA evidence. No allocator,
RSS or retrieval-quality gain is established.

Evidence, preserved binaries and full raw pairs:
`/Users/aditya/.hermes/artifacts/ast-sgrep-deep-review-20261002/literal-nul-perf/`;
focused proofs: `parent-packed-boundary-verification.json` and
`parent-literal-nul-verification.json` in its parent. Original pair artifacts are
not refreshed. Retry speed admission only when the same frozen ordinary replies
remain equal and the short-query regression is removed in at least 24 alternating
observations per arm, or the owner explicitly accepts the correctness/latency
tradeoff. The owner-approved registered-function experiment below was completed
and rejected; shared connection configuration and dependency features were
restored exactly. No callback or index-format expansion remains.

### Registered memchr substring function: rejected (br-mcm)

The approved experiment enabled rusqlite's functions feature, registered a pure
BLOB predicate through the shared reader/writer/in-memory configuration, and
reused it in the three case-sensitive SQL gates. Semantic registration RED,
27 passing focused cases and a rejected NUL-truncation fault established the
prototype's behavior. It did not establish a latency win.

A rotating/reversing three-arm run used the same 23 frozen queries and index,
CPU 5, two warmups and 24 measured observations per arm. All **1656 measured
replies** matched the original complete stdout. Compared in that same run:

| Query | Original GLOB median ms | Byte INSTR median ms | Registered memchr median ms |
|---|---:|---:|---:|
| `literal:as` | 35.806 | 34.038 | 35.819 |
| `literal:xy` | 30.558 | 33.152 | 33.774 |
| `literal:ab` | 28.769 | 29.605 | 28.600 |

The callback is +5.23%, +1.88%, -3.40% versus byte INSTR on these marginal
medians; it does not remove the miss regression against original GLOB. Shared
host drift made original absolute times larger than earlier runs, so compare
arms within this run, not across the tables. Full timings/CPU samples/tails,
source snapshots and driver are preserved under `literal-udf-perf/` beside
`literal-nul-perf/`. Candidate executable SHA256:
`f1b3addb27e6b94c48dc403a4376a876f86da6e7fe7b9c91ab0ab1cfebb26ca9`.
Only the three approved regular project compiler inputs changed. No allocator,
RSS, warm-session, OS-cold or neural claim is made.

The feature, callback, gate substitutions and function-specific test were
removed with focused patches. All four edited files match their pre-experiment
hashes, and the original five focused targets pass 26 cases again. Prior
packed-record/NUL repairs remain; no file was deleted. Retry only with a changed
single mechanism that preserves these outputs and NUL controls and, in at least
24 same-host alternating observations, removes the original short-miss regression
without introducing a median regression above 5% versus byte INSTR on the other
short controls. Callback correctness alone is not grounds to add the mechanism.

## Cold discovery borrowed SQL rows (br-mcm)

**Status: retained allocation reduction with modest measured cold-query gains,
not universal speed or retrieval-quality certification.** The ordered SQL
candidate universe and all acceptance/ranking predicates are unchanged. Decode
path/language/line/content in the original order, including rejected rows;
borrow valid SQLite text until admission. For invalid text/type, delegate to
`Row::get<String>` to retain column-aware error formatting and precedence.

| Fingerprint | Value |
|---|---|
| Source | dirty `main`, based on `c411e20a`; only `bmh.rs` and `literal.rs` differ between native arms |
| Build | default features, `release-perf`, aarch64 Linux, actual Cargo compiler rustc 1.98.0 |
| Worker | shared `spark-1672`, Linux 6.17.0-1026-nvidia/glibc 2.39; paired calls pinned CPU 5 (ARM part 0xd85); MemTotal 127600752 kB |
| Baseline executable SHA256 | `29142628b0b0ae46b6613d1049bd35c782fb85de81377188bcbf04ba5df1d877` |
| Candidate executable SHA256 | `83ad5a2a8700cca727208d0704a14527e66f7301c53445e93d1e28b3cada5927` |
| Inputs | existing frozen self corpus/index; 186 regular crate/vendor/Cargo input hashes independently matched before build and after restoration |
| Evidence | `/Users/aditya/.hermes/artifacts/ast-sgrep-deep-review-20261002/borrow-discovery-perf/` |

The native baseline's existing lexical-prefilter span accounts for
222,386 of 246,445 µs in one no-embed path-interning profile. This is a single
stage timing, not allocator frame attribution or a pooled percentile.
Separate glibc `libmemusage.so` captures of that same complete no-embed query:

| Instrumented allocator counter | Baseline | Candidate |
|---|---:|---:|
| malloc calls | 3,054,237 | 24,430 |
| Cumulative tracked heap bytes | 86,145,665 | 3,860,091 |
| Peak tracked heap bytes | 857,027 | 857,028 |

This proves reduced allocation churn, **not reduced peak memory or RSS**.
Uninstrumented timing uses complete fresh native processes, 17 actual gold
queries, 24 rotating/reversing observations per arm after two warmups, one
fixed index, CPU 5 and OS-warm storage. Default embedding configuration is
retained for timing (the allocator capture above explicitly disables embedding).
All **816 measured replies** have identical stdout hashes between arms.

| Representative query | Baseline median ms | Candidate median ms | Change |
|---|---:|---:|---:|
| compact output path interning | 208.967 | 201.074 | -3.78% |
| consistent search snapshot generation | 157.129 | 151.479 | -3.60% |
| derive the next command to run from the best result | 57.276 | 54.806 | -4.31% |
| reciprocal rank fusion across evidence channels | 72.484 | 70.278 | -3.04% |
| word:SnapshotStamp | 2.582 | 2.645 | +2.43% |

All eight conceptual-query marginal medians improve 3.04–4.31%; untouched
symbol/literal/caller controls mostly move slightly downward. Countermetrics
remain in `summary.json`: query-vector-reuse p95 increases 163.191→164.851 ms;
word control p95 increases 2.845→2.905 ms; two other short-control tails also
increase. Nearest-rank p95 with 24 observations is descriptive, not tail SLA
certification. No warmed-session, OS-cold storage, neural-model or Mac claim.

The preserved candidate and rebuilt restored executable both replay 17 frozen
real-corpus replies and 24 decoding/control observations exactly (13 operational
errors and 11 valid/SQL-excluded controls). Suppressing borrowed text errors
changed three observations, including language-rejected content and error
precedence; the mutant was removed, both source hashes restored, and the rebuilt
executable hash/replies restored. Existing four named targets report 11 passes:
10 exercised behavior tests, plus the optional ripgrep differential's early-return
pass (competitor **not run**). Existing Unicode warm/cold and short-word tests
cover the unchanged verifier/admission; no post-hoc tautological test was added.

Commands: the artifact's `paired-stream-worker.py` takes positional arms
`followup-current-control followup-borrowed` with `PAIR_CPU=5 PAIR_SAMPLES=24`;
`replay.py <candidate>` validates the recorded binary and frozen argv/replies on
the original worker. Cargo used remote-required RCH with one `-p` package and
`--jobs 2`: build `ast-sgrep-cli --bin asgrep --profile release-perf`; tests
`ast-sgrep-core --test literal_warm_cold_parity --test literal_glob
--test literal_word_limit_window --test literal_diff`; both specify
`--locked --target aarch64-unknown-linux-gnu`. Complete commands/logs are in
`parent-verification.json` and the neighboring `borrow-discovery-*.log` files.
RCH source-receipt mode refused the existing `CLAUDE.md` symlink; manual hash
matching is bounded input evidence, not a claimed RCH receipt. An initial driver
call used the wrong environment-variable interface and ran historical arms;
its log is excluded from current comparisons (local historical receipts remain).
CASS exclusion-history lookup remains blocked; the negative ledger records the
retry condition. This does not resolve the earlier distinct-file tradeoff.

## Pi sanitizer regex fast-path rejection (br-mcm)

**Status: rejected on an earlier frozen dirty-tree snapshot; no retained
sanitizer change.** This is separate from the later retained bounded width scan.
The proposed primitive-string no-control regex shortcut preserved outputs but
regressed representative complete search/edit cards. Node 26.7.0, shared Apple
M5 Max/18 logical CPUs, TypeScript source/source comparison; captured native
input envelopes came from the existing bindingVersion 2.1.0 addon, not a current
or published-package native build. The frozen baseline `present.ts` SHA256 is
`7f642a029514c63d3905b605bd31da33a140ce2647983cdc8455b01c6af41ab3`.

Twenty-four alternating AB/BA pairs per scenario, 14 scenarios, eight warmup
batches per arm. Tiny/default batches contain 512 uncached complete card calls;
large/Unicode/dirty read batches contain 128. Amortized microseconds/call are
not whole-tool or individual-call tail latencies. Percent changes below are
**medians of paired ratios**, not ratios of the two marginal medians.

| Frozen complete-card scenario | Original median µs | Candidate median µs | Median paired change |
|---|---:|---:|---:|
| default read | 38.660 | 23.515 | -38.9% |
| large read | 341.064 | 59.360 | -82.6% |
| default search | 31.500 | 35.870 | +13.7% |
| default edit | 19.411 | 22.319 | +17.6% |
| large search | 90.404 | 102.247 | +12.3% |
| large edit | 50.220 | 59.969 | +18.3% |

Both unchanged-clone and restored-source controls are retained and do not show
those search/edit regressions. Parent independently reconciled every raw
scenario's 24-pair count and candidate arithmetic with the report. The frozen
output file contains 6,205 observations; the child recorded source/artifact-JS
equivalence, 26 existing presentation test passes and rejection of an
ESC-admission mutant. These older equivalence/tests were not rerun as a current
package certificate. Nine before/after hashes in the restoration receipt agree
for that older snapshot; **they are not hashes of today's parent-edited tree**.
Current parent width/surrogate fixes were separately checked and are intact.

Evidence: `/Users/aditya/.hermes/artifacts/ast-sgrep-deep-review-20261002/ui-perf-followup.json`,
`ui-perf/{measure,control,restored-control}-raw.json`, frozen inputs/profiles,
and `parent-ui-followup-reconciliation.json`. The replay scripts/commands are
listed in the child handoff. CASS history lookup remains blocked. Retry only
with a changed single mechanism preserving frozen outputs, at least 24 same-host
complete-card pairs, a default-read gain of at least 10%, and no median paired
regression over 5% on tiny/default/large search/edit or ANSI controls in either
order. No per-arm memory, whole-tool latency or published-package claim.

## Pi bounded display-width scan (br-mcm)

**Status: retained renderer-only optimization with small-card countermetrics;
not whole-tool/live-terminal latency or a release certificate.** Cropping now
uses a shared width walker that stops once its nonnegative cell count exceeds
the requested width. Public `displayWidth` still scans completely with an
infinite stop limit. ANSI recognition, Unicode/tab rules, crop references,
ellipsis handling, style resets and cached-card behavior are unchanged.

Fingerprint: dirty `main`/`c411e20a`, Apple M5 Max arm64/18 logical CPUs,
51,539,607,552 bytes RAM, Darwin 25.6.0, Node **22.23.3**, TypeScript **5.9.3**.
Both frozen original and candidate are source-derived ES2022/ESNext modules
compiled identically, timed in the **same Node process**. Uncached complete
card-model + render path, ANSI theme; no source-versus-distribution timing mix.
Baseline `present.ts` SHA256:
`d33200544cfb79d32cd635ea652ca21b276a53f9f3e8f5d096bf3b16a2ba0028`.
Candidate `present.ts` SHA256:
`b249f1ab0f2f98ca2472a9ec4f7aa3cbf39931fc0397b0e4472659974151bd4d`.
The frozen `card.ts` hashes are identical across arms.

The baseline CPU sample subset containing `profileCards` had 873 samples:
`displayWidth` was rank 2 by self weight (**19.35%**); its ANSI scanner was
rank 1 (**33.48%**). Resampling the candidate leaves `scanDisplayWidth` at
**8.38%** self weight and exposes sanitization as a larger remaining component.
These are sampler shares in the declared nine-fixture mix, not wall-time
fractions or a fixed-work CPU reduction. Import/compiler startup is excluded.
Heap profiles did not provide samples attributable to that caller subtree;
there is **no per-arm allocation or memory improvement claim**. Reported process
max RSS includes compiler loading and both arms, not just product rendering.

Twenty alternating original/candidate batches per case after warmup. Values
below are **p50 batch-derived average microseconds/card**, not individual-frame
or tail-latency percentiles. Raw p95/p99/max and `cpuUsage` are in `summary.json`;
with 20 observations p99 equals the largest batch average.

| Initial complete-card fixture | Baseline µs/card | Candidate µs/card | Median change |
|---|---:|---:|---:|
| tiny read | 4.268 | 4.920 | +15.3% |
| tiny search | 8.902 | 9.329 | +4.8% |
| tiny edit | 8.033 | 7.857 | -2.2% |
| default read | 31.992 | 33.078 | +3.4% |
| default search | 63.002 | 57.979 | -8.0% |
| default edit | 249.963 | 252.845 | +1.2% |
| large read | 737.815 | 424.973 | -42.4% |
| large search | 1,140.415 | 839.990 | -26.3% |
| large edit | 2,805.010 | 1,672.531 | -40.4% |

**Countermetrics retained:** one predeclared larger-batch tiny/default
confirmation, without source changes or replacing initial observations, gave
read/search/edit tiny changes **+7.3% / +2.3% / +0.6%** and default changes
**-1.9% / -12.0% / -6.2%**. Initial loops per batch were 1,000/200/10 for
read/search tiny/default/large; edit used one quarter (minimum one). Confirmation
multiplied tiny batches by ten and default batches by two. No third tuning round,
size selector, global CPU/governor change or cached-render gain is claimed.

Frozen output vectors for all nine card fixtures and 360 crop helper cases
(ANSI, malformed UTF-16, Unicode, fractional/zero/negative/nonfinite budgets,
wide/styled ellipses) match exactly. All 28 focused tests passed; changing the
strict overflow comparison to `>=` failed eight existing tests. After byte-exact
restoration, the targeted source compiled strictly, only its JS was emitted,
and declarations stayed unchanged. The installed Pi TUI 1.0.0 oracle validated
2,926 rows across 70 source/dist card models with exact source/dist equality;
this did not launch a real terminal or certify all Pi versions.

Evidence: `/Users/aditya/.hermes/artifacts/ast-sgrep-deep-review-20261002/card-width-perf/`
(frozen modules, `baseline.json`, `comparison.json`, `confirmation.json`, CPU/heap
profiles, hotspot tables, `summary.json`). External replay script:
`/Users/aditya/.hermes/cache/scratch/ast-sgrep-card-width-perf.mjs`;
commands/mutation/build logs are linked by `parent-pi-bounded-width-verification.json`
at the artifact root. CASS exclusion-history search remains blocked and recorded
in the negative ledger; no whole-tool, package-size or retrieval-quality claim.

## Code Mode read-only warming deferral (br-mcm)

**Status: retained targeted read-startup optimization, not a universal speedup,
reproducible-in-tree harness or release certificate.** Session preparation now
has named Fresh/Read/Search phases, separate from root/language/index identity.
The first read anchors the existing deferred SQLite snapshot with the same
constant-size generation query that previously began warming. Search/chain
still attempt the original `warm_search_path` once per cached open, including
its existing best-effort error policy; no ranking/DF preparation is removed.
Repeated reads do not construct line corpus, trigram, symbol or embedding caches.

Build and host match the preceding indexed-range experiment: dirty `main` based
on `c411e20a`, rustc 1.98.0, default features, `release-perf`, shared Linux aarch64
`spark-1672`. Baseline SHA256 (retained indexed-range candidate):
`6302973b9f53af0376deb4f91ae215cf9f13cb89888198942130d840291e6489`.
Initial prototype SHA256:
`252b709b9448c0e0924fb1f33575e4505081985750a3c24c08a94ed59539c036`.
Retained/refined SHA256:
`29142628b0b0ae46b6613d1049bd35c782fb85de81377188bcbf04ba5df1d877`.

Real `asgrep codemode-batch --requests FILE`, fresh process/warm OS cache,
lexical-only indexes, three warmups and 20 alternating observations per arm/case.
**Retained comparison is explicitly CPU-0 pinned**, not a replacement for the
initial unpinned prototype results. Existing indexed synthetic and frozen
product-source fixtures are reused. Batch32 includes startup and first preparation.

| Complete-call fixture, retained CPU-0 comparison | Baseline p50 ms | Candidate p50 ms |
|---|---:|---:|
| 20-line index / first read | 2.758 | 2.721 |
| Actual `io.rs` in frozen two-file index / first read | 8.740 | 3.042 |
| 100,000-line index / first front read | 47.449 | 3.453 |
| 100,000-line index / first tail read | 47.503 | 3.050 |
| 100,000-line index / serial batch32 reads | 49.775 | 4.887 |
| 100,000-line index / first literal find | 48.124 | 48.588 |
| 100,000-line index / read then literal and word finds | 48.572 | 48.473 |
| Actual source / defs-only | 8.949 | 9.125 |
| Actual source / read, defs, chain | 10.801 | 11.293 |
| 100,000-line index / chain-only | 48.915 | 49.196 |

**Countermetrics retained:** tiny first-find rose 3.330 → 3.577 ms (+7.4%),
defs-only rose 2.0%, and the actual mixed sequence rose 4.6%. The initial
unpinned prototype tiny-find was 1.920 → 3.701 ms and large-find 23.434 →
30.876 ms. Both original prototype comparisons remain in the raw evidence;
no claim that deferral speeds retrieval or every workload. One refinement moves
snapshot anchoring specifically to read preparation so retrieval-first opens
avoid an unnecessary generation-query round trip; named state replaces tuple
access without mixing preparation state into cache identity.

Independent `libmemusage.so` profiles for the 100,000-line/front read recorded
malloc calls **303,293 → 2,651** and tracked heap peak **2,356,228 → 408,698 bytes**.
Separate single-read `/usr/bin/time -v` RSS peaks were **19,336 → 7,844 KiB**.
Instrumentation is separate from timing; no CPU sample-share or neural-model,
cold-storage, persistent-server or cross-platform performance claim.

Evidence root: `/Users/aditya/.hermes/artifacts/ast-sgrep-deep-review-20261002/read-warming-perf/`
(`baseline.json`, both original prototype comparisons, `refined-cpu0.json`,
allocator profiles, `summary.json`, frozen snapshot replies). External script:
`/Users/aditya/.hermes/cache/scratch/ast-sgrep-read-warming-perf.py`; worker fixtures
and preserved executables: `/home/aditya/.cache/hermes/scratch/asgrep-read-warming-profile-20261003/`.
All **480 retained measured responses**, and all **1,440 including prototypes**,
matched frozen originals after removing only `wall_ms`. All **67 targeted tests**
passed on restored code, including root/language reuse, writer freshness,
read bounds, cache admission and batch/chain paths. A real sticky-serve test
preserved snapshot identity through a failed read followed by an unstamped
peer SQLite write; removing only the generation anchor failed semantically.
Source restoration was byte-exact, the native executable was rebuilt, and the
same serve transcript passed again. Full commands/logs are recorded in
`parent-read-warming-verification.json` at the artifact root. CASS history
exclusion and Beads/Mail bookkeeping remain separately blocked, not certified.

## Code Mode indexed SQL-range experiment (br-mcm)

**Status: retained targeted range-read optimization, with explicit small-input
countermetrics; worker-local evidence, not reproducible-in-tree or a release
certificate.** The indexed path previously fetched the entire file for every
window. `IndexStore::source_lines_in_range` now selects the inclusive range and
excludes only the global empty EOF cursor. A single CTE/LEFT JOIN keeps range
selection and EOF discovery on one SQLite snapshot; `None` (no indexed rows)
and `Some([])` (indexed empty/past-EOF range) remain distinct. Existing full-text
reconstruction/edit behavior and the common Unicode/budget selector are unchanged.

Build: same dirty `main`/`c411e20a`, default features, rustc 1.98.0,
`release-perf`, Linux aarch64/shared `spark-1672` as the preceding disk-window
experiment. Baseline executable (the retained disk-window candidate) SHA256:
`e3544d3dbb2de7d8de1c81044d56f2f1b4f9188a130122fc411eac623fac850b`.
Indexed-range candidate SHA256:
`6302973b9f53af0376deb4f91ae215cf9f13cb89888198942130d840291e6489`.

Real `asgrep codemode-batch --requests FILE` runs: three-line window, lexical
index, warm OS cache, three warmups, 20 alternating observations per arm/case.
Generated fixtures contain indexed `//x\n` lines. The real-source fixture is a
frozen two-file index of the current `io.rs` and `queries.rs`. Batch32 reuses one
session for 32 reads **but still includes process startup and first warming**;
it is not an isolated per-read or persistent-server benchmark.

| Fixture / complete call | Baseline p50 ms | Candidate p50 ms |
|---|---:|---:|
| 100,000 lines / one front read | 37.553 | 23.469 |
| 100,000 lines / one tail read | 37.359 | 29.104 |
| 10,000 lines / serial batch32 front | 39.549 | 5.029 |
| 100,000 lines / serial batch32 front | 374.622 | 24.692 |
| Actual `io.rs`, 525 lines / serial batch32 | 10.776 | 10.573 |
| Actual `queries.rs`, 1,041 lines / serial batch32 | 18.402 | 10.396 |

**Countermetrics retained:** unpinned one-read 20-line front/tail controls were
2.689 → 2.852 ms and 1.722 → 1.745 ms; 10,000-line front was 5.258 → 7.321 ms.
One separately labeled CPU-0 confirmation gave that 10,000-line front
11.134 → 8.721 ms, but tiny/tail still rose 3.472 → 3.663 ms. The 20-line
batch32 rose 2.592 → 2.855 ms; actual `io.rs` one-read rose 4.994 → 5.360 ms,
while actual `queries.rs` one-read was 5.731 → 5.779 ms. No universal latency
improvement is claimed; original observations are not replaced by confirmations.

For the **100,000-line/front** indexed fixture, `libmemusage.so` recorded
malloc calls 403,163 → 303,293 and tracked heap peak 6,803,371 → 2,356,258 bytes.
Separate single-read `/usr/bin/time -v` RSS peaks were 24,968 → 19,336 KiB.
Instrumented profiles are not latency samples. Remaining initialization work
includes `CodeModeSession::with_searcher` eagerly calling `warm_search_path`,
which loads the line corpus before a read; that connected cost is unchanged.

Raw evidence: `/Users/aditya/.hermes/artifacts/ast-sgrep-deep-review-20261002/indexed-window-perf/`
(`baseline.json`, both comparison files, `batch32-comparison.json`,
`actual-source-comparison.json`, allocator stderr, summaries and query plan).
External script: `/Users/aditya/.hermes/cache/scratch/ast-sgrep-indexed-window-perf.py`;
worker fixtures: `/home/aditya/.cache/hermes/scratch/asgrep-indexed-window-profile-20261003/`.
All **760 measured batch responses** matched frozen originals after removing
only `wall_ms`; 24 additional boundary calls matched for blank/empty lines,
CRLF, Unicode, wide-line caps, stale indexed data, unindexed fallback and EOF.
All 47 selected Code Mode tests passed before/after byte-exact EOF-mutant
restoration; the existing `read_large_json_bounds_do_not_wrap` rejected loss of
cursor exclusion both before and after the refactor. Full commands/logs are in
`parent-indexed-window-verification.json` at the artifact root. No new dependency,
schema/index migration, cross-platform certification or search-quality claim.
CASS exclusion-history search remains blocked, explicitly recorded in the ledger.

## Code Mode borrowed line-window experiment (br-mcm)

**Status: retained targeted optimization; worker-local measured evidence,
not reproducible-in-tree or release/platform certification.** The disk fallback
previously copied every source line into an owned numbered vector before
selecting a window. It now passes borrowed numbered lines to the same window
selector used by indexed reads. Caps, Unicode handling, ordering, line/ref
coordinates, truncation and all non-duration envelope fields are unchanged.

Build: dirty `main` based on `c411e20a`, default features, `release-perf`, Linux
aarch64 on shared `spark-1672` (Cortex-X925/A725). Cargo's `.rustc_info.json`
confirms **rustc 1.98.0**; the separately captured SSH-shell compiler was a
different nightly and is not build provenance. Baseline executable SHA256:
`7c44263851fb59d9524090aa815bb24448954771ac15a15edc6713ea6559cc87`.
Candidate executable SHA256:
`e3544d3dbb2de7d8de1c81044d56f2f1b4f9188a130122fc411eac623fac850b`.

Scenario: real `asgrep codemode-batch --requests FILE`, one three-line read of
an unindexed `data.txt` containing `x\n` repeated N times. The index contains
one Rust seed; setup/indexing is outside timing. Fresh process, warm OS cache,
three warmups and **20 alternating observations per arm/case**. RSS is one
separate `/usr/bin/time -v` peak observation per arm/case, in KiB.

| Source lines / window | Baseline p50 ms | Candidate p50 ms | Median change | Baseline → candidate peak RSS KiB |
|---|---:|---:|---:|---:|
| 20 / front | 2.697 | 3.578 | +32.7% | 7,872 → 7,844 |
| 20 / tail | 3.480 | 3.298 | -5.2% | 7,872 → 7,844 |
| 100,000 / front | 10.843 | 3.374 | -68.9% | 14,084 → 7,984 |
| 100,000 / tail | 10.872 | 3.638 | -66.5% | 14,084 → 8,100 |
| 1,000,000 / front | 68.828 | 3.478 | -94.9% | 72,004 → 9,732 |
| 1,000,000 / tail | 64.053 | 8.241 | -87.1% | 72,004 → 9,732 |

**Countermetric retained:** the unpinned tiny/front control regressed. One
bounded confirmation pinned both arms to CPU 0, without changing global host
settings, gave tiny/front **2.548 → 2.724 ms (+6.9%)** and tiny/tail
**2.337 → 2.565 ms (+9.7%)**; large/front **78.816 → 4.583 ms** and large/tail
**80.136 → 14.828 ms**. These are separately labeled diagnostic measurements,
not replacements for the original unpinned observations or a universal speed
claim. Raw p95/p99/max are in `summary.json`; with 20 samples, p99 is the sample
maximum and has no tail-certification meaning.

Independent `libmemusage.so` allocation profiles for the million-line/front
case recorded malloc calls **1,003,126 → 3,125** and tracked heap peak
**36,863,463 → 2,309,244 bytes**. Instrumented profiles are not latency trials.
CPU-stack sampling was blocked by `perf_event_paranoid=4`; no kernel settings
were changed and no CPU sample-share attribution is claimed.

Evidence root: `/Users/aditya/.hermes/artifacts/ast-sgrep-deep-review-20261002/window-perf/`
(`baseline.json`, `comparison.json`, `comparison-cpu0.json`, allocation stderr,
`summary.json`). The external replay script is
`/Users/aditya/.hermes/cache/scratch/ast-sgrep-window-perf.py`; preserved baseline
and fixtures are on the worker at
`/home/aditya/.cache/hermes/scratch/asgrep-window-profile-20261003/`.
The candidate's **480 measured baseline/candidate observations** matched frozen
payloads after removing only `wall_ms`. All 47 selected Code Mode tests passed
before and after byte-exact source restoration following an end-bound mutant;
the existing Unicode/blank-line assertion rejected that mutant. This does not
certify search ranking, cold storage, persistent-session latency or Pi UI.
The CASS exclusion-history query remains blocked and is recorded in the
performance ledger; it was not silently marked complete.

## 2026-10-02 distinct-file admission candidate

**Status: measured local candidate, not a release certificate or a universal
speedup.** Both executables use the corrected independent Recall@1/5/20
semantics. Thus the quality delta below is retrieval, not relabeling a metric.
Gold judgments, ranking weights and keep thresholds were unchanged.

**Historical executable scope:** the latency/quality values below describe only
those hash-identified preserved executables. Subsequent Unicode-correctness work
changed cold case-insensitive discovery to a full ordered row scan. In particular,
the reported latency delta below is **not a measurement of today's tree**.
`search-perf-followup.json` in the deep-review artifact root records this distinction;
its borrowed-row proposal was subsequently measured in `Cold discovery borrowed
SQL rows` above. That allocation optimization is not a reversal of the original
tradeoff decision or a measurement of those older executables.

| Fingerprint | Value |
|---|---|
| Source | dirty `main` based on `c411e20a`; source edits are uncommitted |
| Frozen corpus | 677 selected tracked text files; manifest SHA256 `44d2ce59e3e216dc0fa6f69ecf2b886b015308f5f1d982756f15929e339e7a7d` (snapshot count, not indexed-file count) |
| Baseline executable | SHA256 `1bb45784e9dfe1256b27a8c484a864da00f9119797bc59c1ae837f509ed14372` |
| Candidate executable | SHA256 `79b770469c5c36a4f50f760be69ec04e15bb676e0e6eaa0b2c1f5c5a7063b856` |
| Build/host | default features, Cargo `release` (not `release-ship`), Linux aarch64, shared `spark-1672`, Cortex-X925/A725 |
| Local evidence root | `/Users/aditya/.hermes/artifacts/ast-sgrep-score-pass-20261002/` |

| Metric (`self`, 17 queries; hybrid and no-embed agree) | Before | Candidate |
|---|---:|---:|
| MRR | 0.765 | 0.861 |
| nDCG@k | 0.756 | 0.870 |
| Recall@1 | 0.676 | 0.735 |
| Recall@5 | 0.765 | 0.853 |
| Recall@20 | 0.765 | 0.941 |

The conceptual subset (8 queries) moves from MRR 0.500 to 0.705 and Recall@k
0.500 to 0.875. All 6 symbol and 3 literal queries are unchanged. Path interning,
snapshot consistency and follow-up derivation enter the page; rank fusion still
misses. Fixed `native_semantic` (3) and `invent_path` (6) gold observations are
unchanged in both arms. Graph precision remains 4 correct edges over 4 predictions;
unobserved resolution tiers remain null. These small fixtures are not external
held-out quality certification. Optional neural configurations were not tested.

**Countermetric:** 24 alternating fresh-process observations per query/arm on
one frozen index, after 2 warmups, give a mean of the 8 conceptual per-query
medians of **8.855 → 10.272 ms (+16.0%)**. This includes process startup and is
not a pooled percentile. Shared-host variation affects unchanged control queries
too. The in-process bench blocks were `quarantine_cv`; they establish no speed
keep. Correct file admission does more candidate work, so this is a correctness/
recall candidate with a measured latency cost, pending the owner's tradeoff
decision—not an optimization win. Warmed-session code was not changed.

Unstripped executable sizes are 40,676,048 → 40,673,392 bytes; the 2,656-byte
layout difference is not a meaningful footprint claim or a shipped Mac size.
The permitted Mac executable was an empty placeholder, so Mac startup/RSS was
not measured. Baseline Linux startup (20 observations, 3 warmups): help median
1.238 ms, version median 1.240 ms; help peak RSS 4,772 KiB (one GNU time sample).

Reproduce using the preserved executable, `corpus.tar.gz`, and `measure.py` in
the evidence root on Linux ARM64. The driver records each argv in
`release-{baseline,candidate}/commands.json`; `paired.py` records the exact
same-index alternating protocol in `paired.json`. The fundamental quality command
is `$BIN --json --index-path "$INDEX" eval --gold "$CORPUS/benchmarks/gold/self.json" "$CORPUS" --ab no-embed`.
Run from a fresh output directory. Artifacts are local, not a portable in-tree
canonical fingerprint. Earlier historical rows remain historical.

## 2026-10-02 executable footprint follow-up

**Status: measured Linux ARM64 footprint improvement, not a runtime speed keep.**
The native-artifact workflow and installed DSR recipe select ordinary `release`,
not the optional `release-ship` profile. Ordinary release now strips non-runtime
symbols; optimization level, LTO and codegen-unit settings remain unchanged.
`release-perf` explicitly retains its symbols. Stripped releases sacrifice symbol
names for postmortem debugging; use the profiling build when those are needed.

Same frozen corpus, index and uncommitted source lineage as the preceding row.
Evidence is under its local evidence root, subdirectory `executable-followup/`.
The previous candidate is the control (SHA256 `79b770469c5c36a4f50f760be69ec04e15bb676e0e6eaa0b2c1f5c5a7063b856`).
Final Cargo-built stripped executable SHA256:
`ba57167b643de28a77cd7e6aeb353fd31595162d3733077c6c19aed0f7da58dc`.

| Metric | Ordinary release before | Ordinary release, symbol-stripped |
|---|---:|---:|
| Executable bytes | 40,673,392 | 37,605,440 |
| Deterministic gzip-9 bytes, executable only | 7,479,315 | 6,907,490 |
| Conceptual fresh-process latency, mean of 8 query medians, CPU 5 | 9.8567 ms | 9.8603 ms |
| Symbol/literal controls, mean of 9 query medians, CPU 5 | 2.7846 ms | 2.8385 ms |
| Help startup median, CPU 5 | 1.5063 ms | 1.4822 ms |
| Version startup median, CPU 5 | 1.1299 ms | 1.1526 ms |
| Conceptual search peak RSS median, unpinned | 36,804 KiB | 36,676 KiB |

The executable shrinks **3,067,952 bytes (7.54%)**. Its gzip payload shrinks
**571,825 bytes (7.65%)**; this is not the complete npm package size. RSS is five
GNU-time observations on `compact output path interning`, not a memory-reduction
claim. Search latency uses 60 rotating/reversing observations per arm/query after
two warmups on CPU 5 (shared worker, not exclusive); OS caches are warm. Startup
uses 40 alternating observations after two warmups. This is not cold-storage
latency or warmed in-process latency. CPU usage and indexing cost were not changed
or certified by this pass.

**Counterevidence retained:** the initial unpinned Cargo comparison measured
11.4552 → 12.5551 ms for the conceptual mean of query medians. CPU affinity was
then used to control placement on this heterogeneous worker, not to replace or
erase that observation. Even pinned, `literal:BEGIN DEFERRED` measured +7.52%;
the controls' aggregate was +1.94%. No universal latency-neutrality claim follows.
The post-link stripping probe preserved all allocated section hashes, but the
actual Cargo rebuild changed linker layout; its behavior was separately checked.
All 17 self-query stdout payloads were byte-identical across the control,
post-link stripped probe and final Cargo artifact in the pinned run. The final
release executable also passed all 18 existing `numerical_eval` tests, including
the failure-first repeated-line admission regression. No new test was necessary
for the profile setting; the artifact itself was exercised.

Two alternatives were **not kept**:
- Streaming a bounded path-keyed file map instead of SQLite's global line sort
  preserved all 17 stdout payloads but measured 11.8682 → 12.3930 ms conceptual
  mean-of-medians (24 observations per arm/query). Reverted; original admission
  code remains. Executable SHA256 `f01accab39eab2a098629a0662dee02b2b2fd280d66fa93c35fc87e7e2cc79d4`.
- Existing `release-ship` (thin LTO, one codegen unit, stripped) produced
  36,753,256 bytes but measured 12.6576 → 13.8756 ms against ordinary release in
  its unpinned block. Not selected for the release runners. Executable SHA256
  `2d216e8b4c3c990a29a6ce7777868f7f352abdae43b270a90a6cf31435f0ac71`.

Build command: `RCH_REQUIRE_REMOTE=1 RCH_VISIBILITY=summary rch exec -- env
CARGO_TARGET_DIR="${RCH_TARGET_BASE:-${TMPDIR:-/tmp}}/rch_target_astsgrep_test"
cargo build --locked --release -p ast-sgrep-cli --bin asgrep
--no-default-features --target aarch64-unknown-linux-gnu --jobs 2`.
This was a targeted RCH profile diagnostic, not a DSR release. DSR's
`build ast-sgrep --target linux/arm64 --allow-dirty --dry-run` passed; no release
was run or published. `release-lean.log` records the actual build/pool path;
`lean-artifact-tests.log` records the existing test harness run against that
preserved release executable. `paired-stream.py`, `footprint.py`, raw JSON/logs
and four preserved trial executables are in the evidence directory. The scripts
record worker-local paths; adapt them explicitly when replaying elsewhere.
No Mac/Windows artifact or cross-platform release certification is claimed.
The earlier distinct-file recall-versus-latency decision is still unresolved.

## 2026-09-19 ranking pass (self corpus)

**Status: `reproducible-in-tree`.** Reproduce:
`asgrep --json --no-embed eval --gold benchmarks/gold/self.json .`
(add `--no-embed` removal for the default embed-on config.)

| Provenance | value |
|------------|-------|
| date | 2026-09-19 |
| commit | `845fbf0a`; base = `9bfa6aa0` release build |
| machine | Apple M5 Max (arm64), macOS 26.6.2 |
| corpus | self, 637 indexed files, 17 gold queries |
| harness | `asgrep eval --gold benchmarks/gold/self.json` (in tree) |
| caveat | absolute values move with the indexed lexicon state; associations are learned at index time, so compare within one index, not across indexes |

Paired run, same index, pre-pass binary vs current:

| Metric | no-embed prev | **no-embed now** | embed-on prev | **embed-on now** |
|--------|--------------:|-----------------:|--------------:|-----------------:|
| MRR | 0.827 | **1.000** | 0.727 | 0.850 |
| nDCG | 0.869 | **0.984** | 0.774 | 0.852 |
| recall@1 | 0.706 | **0.882** | 0.618 | 0.735 |
| recall@5 | 0.882 | **1.000** | 0.824 | 0.882 |
| recall@20 | 1.000 | 1.000 | 0.941 | 0.941 |

What changed:

- **Adjudication now sees the retrieval concept set.** The post-fusion critic's
  concept affinity read only the static hand-written concept groups, while the
  retrieval passes expanded the query through the repository (PPMI) vocabulary.
  A symbol covering a *learned* association (`compact -> budget`) therefore
  counted as single-concept. With the union, `CompactBudget`
  (`crates/ast-sgrep-plugins/src/lib.rs`) covers compact + budget and reaches
  rank 1 for `compact output path interning` (was: pool rank 33, page rank 7).
  Unit-tested with a mutation check in `search/critic.rs`.
- **Literal/word lanes route through the critic** (previously hybrid only), and
  `has_code_evidence` counts code-file line hits, so prose/data/test penalties
  actually apply to line-hit shortlists. `is_prose_path` covers data/markup
  extensions and fixture trees.
- **Comment mentions rank below code uses** for non-conceptual intents
  (`COMMENT_MENTION_PENALTY`, code evidence required).
- **`word:<Identifier>` / `literal:<Identifier>` also run the def lane**, so the
  declaration outranks incidental mentions.
- **Removed `demote_thieves_below_conjunction_combine`** (plus
  `steals_conjunction_query` and its unit test). That clamp hardcoded
  `conjunction.rs` / `fusion.rs` / `eval.rs` / `field_weight.rs` and the symbol
  `combine` to force one ordering. Measured: with the clamp deleted the numbers
  above are identical, because the learned-vocabulary rule now carries the same
  ordering by mechanism instead of by file name.

Open, measured: the **default embed-on path stays 0.15 MRR behind** no-embed
(0.850 vs 1.000) and one query keeps a relevant item outside the top 20
(`recall@20 = 0.941`). Embedding-channel crowding is the next candidate, not a
no-embed regression: the embed channel was not touched by this pass.

## Canonical fingerprint rows

**Status: `canonical` + `UNREPRODUCIBLE`.** Cite these ids. The gold + eval
harness that produced them is not in this tree.

One versioned fingerprint per (corpus × metric × config). Other publications
must cite these rows; they must not introduce a second “canonical” value.

| fingerprint id | corpus | config | metric | value | status |
|----------------|--------|--------|--------|------:|--------|
| `self-hybrid-d3eab74` | self @ d3eab74 (18 gold) | default hybrid / `--no-embed` same | MRR | **0.712** | UNREPRODUCIBLE (gold harness absent) |
| `self-hybrid-d3eab74` | self @ d3eab74 (18 gold) | default hybrid | Recall@k | **0.889** | UNREPRODUCIBLE |
| `self-hybrid-d3eab74` | self @ d3eab74 (18 gold) | default hybrid | nDCG@k | **0.751** | UNREPRODUCIBLE |
| `rg-hybrid-default-d3eab74` | ripgrep 14.1.1 (14 gold) | default hybrid (hashed/local embed path as shipped) | MRR | **0.290** | UNREPRODUCIBLE — **canonical** for default hybrid |
| `rg-neural-rerank-d3eab74` | ripgrep 14.1.1 (14 gold) | `neural-embed` + cross-encoder rerank (`ASGREP_NEURAL_EMBED=1 ASGREP_RERANK=1`, see losses.md) | MRR | **0.605** | UNREPRODUCIBLE — different config; **not** interchangeable with 0.290 |
| `self-hist-pre-29129bd` | self (historical unlabeled gold) | historical hybrid | MRR ≈ 0.75 / Recall ≈ 0.94 | — | **SUPERSEDED** by `self-hybrid-d3eab74` (also formerly cited as 0.746); do not quote as current |

**Deprecations:** Do not cite 0.290 and 0.605 as competing “the” ripgrep MRR.
They are two fingerprint rows. Do not cite dual ~0.75 / 0.746 self-corpus
figures alongside 0.712 as current.

## Candidate evaluation pack (no canonical run yet)

**Status: `historical` (negative ledger).** `./benchmarks/run_eval.sh` exists
and refuses dirty worktrees. It has **not** produced a canonical fingerprint.

The tree now contains a gold fixture and a harness:

```bash
./benchmarks/run_eval.sh
```

The harness refuses dirty worktrees and writes generated output under
`benchmarks/results/raw/`, which remains untracked. A previous dirty run was
withdrawn because its corpus referenced experimental generation-layout code
that is no longer part of this change. **No value from that run is canonical or
reproducible, and no performance, token-reduction, or retrieval-quality claim
may cite it.** This note remains as the required negative ledger.

The live `self` corpus changes with the worktree and is therefore weak evidence
for retrieval-quality deltas. Use a frozen or foreign corpus before claiming a
quality change. The repository PPMI lexicon now widens conceptual candidate-file
discovery and semantic scoring. Returned lexical evidence plus final lexical
and structural scoring still use the original query, but the expanded terms do
increase candidate work. The frozen A/B below measures that change. It is not
evidence of broader retrieval quality.

## Native repository-vocabulary A/B (fixed fixture @ 19a7eb0)

**Status: `reproducible-in-tree` (A/B delta rows, not a canonical quality
fingerprint).** The three human-judged queries each have two relevant results:
one literal training context and one held-out target symbol whose source omits
the query term. This isolates whether a repository-learned association can
close the lexical gap without model weights, downloads, network access, or an
external process.

| fingerprint id | config | MRR | nDCG | Recall@1 | Recall@5 | Recall@20 |
|----------------|--------|----:|-----:|---------:|---------:|----------:|
| `native-vocab-ab-19a7eb0` | default hybrid, repository vocabulary on | 1.000 | 0.877 | 0.500 | 1.000 | 1.000 |
| `native-vocab-ab-19a7eb0-control` | `--ab no-repository-vocabulary` control | 1.000 | 0.613 | 0.500 | 0.500 | 0.500 |

Candidate minus control: +0.264 nDCG and +0.500 Recall@5/20; MRR and
Recall@1 are unchanged because the literal training context ranks first in
both arms. All three held-out target symbols enter the top five only in the
candidate arm. This small purpose-built fixture proves the mechanism, not
general semantic-search quality.

Reproduce at implementation commit `19a7eb0` (hashed-256 in-process backend,
three queries, `benchmarks/fixtures/native_semantic`):

```bash
ASGREP_BENCH_RATCHET=0 cargo run -q -p ast-sgrep-cli --bin asgrep -- \
  --json eval --gold benchmarks/gold/native_semantic.json \
  benchmarks/fixtures/native_semantic --ab no-repository-vocabulary
```

## Invent-path conceptual gold (`invent_path`)

**Status: `reproducible-in-tree` measurement spine.** Offline hashed concept
groups (`hashed-256-xof-cg5`) — no neural download, no daemon. Six zero-overlap
/ paraphrase queries over `benchmarks/fixtures/invent_path`. Record rows here
after each invent-path change; do not quote empty placeholders as certified.

```bash
ASGREP_BENCH_RATCHET=0 cargo run -q -p ast-sgrep-cli --bin asgrep -- \
  --json eval --gold benchmarks/gold/invent_path.json \
  benchmarks/fixtures/invent_path
```

| fingerprint id | config | MRR | nDCG | Recall@1 | Recall@5 | notes |
|----------------|--------|----:|-----:|---------:|---------:|-------|
| `invent-path-cg5-8d74cfdd` | default hybrid, hashed-256-xof-cg5, repo vocab on | 1.000 | 1.000 | 1.000 | 1.000 | 6/6 rank-1 on invent_path gold; small purpose-built fixture, not a release certificate |

## 7d5x.4 A/B — multi-field rescoring vs concat vector (self @ d18725e)

**Status: `reproducible-in-tree` (A/B delta rows, not canonical quality
fingerprints).** These rows exist to answer one question: does the
intent-weighted per-field embed rescore (7d5x.3) change fused retrieval
outcomes on the self gold? They must not be quoted as "the" self-corpus MRR;
the corpus, gold (17 queries), and commit differ from `self-hybrid-d3eab74`.

| fingerprint id | comparison | delta MRR | delta nDCG | delta Recall@20 | verdict |
|----------------|------------|----------:|-----------:|----------------:|---------|
| `self-ab-concat-7d5x4-d18725e` | hybrid (multi-field) vs `--ab concat-embed` | 0.000 | 0.000 | 0.000 | **null result**: zero delta on all 17 queries |
| `self-ab-noembed-7d5x4-d18725e` | hybrid vs `--ab no-embed` | +0.008 | +0.020 | +0.059 | embed channel contributes (1 conceptual query found only with embed) |

Absolute aggregates for the hybrid arm of both runs: MRR 0.491, nDCG 0.534,
Recall@20 0.735 over 17 gold queries (12 prior + 5 conceptual added for this
eval). Do not cite these as canonical; they are the A-arm context for the
deltas above.

Both arms run the identical fused pipeline including the deterministic
post-fusion critic; the only variable is embed-channel scoring (per-field
mix vs concatenated vector). Hashed-256 in both arms; no ONNX involved.

Interpretation, kept small: on this gold, per-field rescoring reorders
nothing that fusion had not already decided — lexical/def/graph channels
dominate the final top-k, and the query set contains no paraphrase the
concat vector misses but the field mix catches. The embed channel as a
whole earns its seat (`conceptual-query-vector-reuse` is found at rank 7
only with embed on). The ONNX gate (`ast-sgrep-onnx-only-if-eval-hole`)
stays closed: no measured hole here justifies a model download, and a
judged paraphrase eval on a foreign corpus remains the missing evidence.

Reproduce (clean worktree at `d18725e`, macOS arm64, hashed-256 backend,
rustc per `Cargo.lock` toolchain):

```bash
cargo build --release -p ast-sgrep-cli
target/release/asgrep eval --gold benchmarks/gold/self.json . --json
target/release/asgrep eval --gold benchmarks/gold/self.json . --json --ab concat-embed
target/release/asgrep eval --gold benchmarks/gold/self.json . --json --ab no-embed
```

or run `./benchmarks/run_eval.sh`, which now emits
`self-ab-concat-embed.json` beside the existing artifacts. The mechanism
itself is falsifiable in-tree: `tests/core/concat_embed_ab.rs` proves the
concat arm strips per-field scores while the default arm attaches them.

Found by this run and fixed before measuring: `defs:` crashed on any span
containing a blank line (`substr` of an empty BLOB is NULL);
`ast-sgrep-5vur`, fixed in `d18725e` with a regression test. The first eval
attempt died on it, which is exactly what the eval pack is for.

The candidate pack now emits graph-edge precision by resolution tier from a
fixed, completely labeled fixture, including explicit nulls for unobserved
tiers. No graph precision value is canonical until a reviewed clean-worktree
run is promoted here with its commit fingerprint. Still unmeasured here:
foreign-corpus quality and token efficiency, confidence calibration,
multi-field rescoring on a foreign or judged paraphrase corpus (the self-gold
A/B above is a null result, not coverage), and agent-in-the-loop
token/tool-call outcomes.

## Provenance

**Status: `historical`.** Pins for the UNREPRODUCIBLE quality/speed rows below.

| field | value |
|-------|-------|
| date | 2026-07-10 |
| commit | `d3eab74c7f3725bae4b1fab24ea94fe3b58d3601` (d3eab74) |
| machine | Apple M5 Max, 18 cores (arm64), 48 GiB, macOS 26.5, APFS SSD |
| build | `cargo build --profile release-perf -p ast-sgrep-cli` |
| rustc | 1.96.0 |
| python | 3.14.6 |
| competitors | ripgrep 15.1.0, ast-grep 0.44.1, semgrep 1.168.0 |
| timing | hyperfine 1.20.0 |

The original run used a `corpora.lock` file that is not shipped in this tree. Its pinned tag and SHA values are preserved here:

| corpus | ref | SHA |
|--------|-----|-----|
| self (this repo) | d3eab74 | `d3eab74c7f3725bae4b1fab24ea94fe3b58d3601` |
| ripgrep | 14.1.1 | `4649aa9700619f94cf9c66876e9549d83420e16c` |
| flask | 3.0.3 | `c12a5d874c5a014495eb2db8a73f40037bc813ac` |
| tokio | tokio-1.38.0 | `14c17fc09656a30230177b600bacceb9db33e942` |
| express | 4.19.2 | `04bc62787be974874bc1467b23606c36bc9779ba` |

## Retrieval quality — self corpus (18 gold queries)

**Status: `canonical` citation of `self-hybrid-d3eab74` + `UNREPRODUCIBLE`.**
**Reproduction status:** unavailable from this tree. The recorded run used an 18-query eval gold file and retrieval harness that are not checked in; the current `tests/fixtures/ranking/cases.json` is a different schema and corpus. `asgrep eval --gold <gold.json> <root> --ab <mode>` is the supported evaluator shape, but without the original gold file it cannot reproduce these rows.

| tool | MRR | Recall@k | nDCG@k |
|------|----:|---------:|-------:|
| asgrep hybrid | **0.712** | **0.889** | **0.751** |
| asgrep --no-embed | 0.712 | 0.889 | 0.751 |
| asgrep semantic-only | 0.294 | 0.611 | 0.364 |
| ripgrep (file order) | 0.061 | 0.167 | 0.086 |

Note: 0.712 is lower than the previously published ~0.75 / 0.746
(`self-hist-pre-29129bd`, **superseded**); the drop landed
with the reviewed correctness fixes in `29129bd` (ranking changed for one
query). The historical run recorded a `retrieval_gold.rs` gate (MRR >= 0.70), but that harness is not present in this tree. This table supersedes the old figure and remains a published record, not a currently reproducible result.

## Retrieval quality — foreign-corpus bake-off (k=10)

**Status: `canonical` for `rg-hybrid-default-d3eab74` / `rg-neural-rerank-d3eab74` + `UNREPRODUCIBLE`.**
**Reproduction status:** unavailable from this tree. The foreign corpora can be recovered from the pinned SHAs above, but their gold labels and the cross-tool bake-off harness are not checked in. Running `cd benchmarks` alone performs no evaluation.

### ripgrep 14.1.1 (Rust, 14 queries)

**Canonical default-hybrid fingerprint:** `rg-hybrid-default-d3eab74` → **MRR 0.290**.
The neural+rerank figure **0.605** is fingerprint `rg-neural-rerank-d3eab74` only
(see [`losses.md`](losses.md)); it is not the default hybrid row.

| tool | MRR | Recall@k | nDCG@k | wall ms |
|------|----:|---------:|-------:|--------:|
| asgrep hybrid | 0.290 | 0.464 | 0.330 | 28 |
| asgrep --no-embed | 0.290 | 0.464 | 0.330 | 19 |
| ripgrep (file order) | 0.000 | 0.000 | 0.000 | 11 |
| ast-grep structural | 0.143 | 0.214 | 0.162 | 31 |
| **semgrep (reference to beat)** | **0.536** | 0.571 | 0.545 | 1235 |

### flask 3.0.3 (Python, 15 queries)

| tool | MRR | Recall@k | nDCG@k | wall ms |
|------|----:|---------:|-------:|--------:|
| asgrep hybrid | 0.161 | 0.533 | 0.246 | 15 |
| asgrep --no-embed | 0.161 | 0.533 | 0.246 | 13 |
| ripgrep (file order) | 0.162 | 0.600 | 0.259 | 10 |
| ast-grep structural | 1.000 | 1.000 | 1.000 | 16 |
| semgrep | 0.033 | 0.067 | 0.042 | 1254 |

The semgrep 0.536 MRR on the ripgrep corpus is the standing reference for the
`ast-sgrep-6hk` gate (foreign-corpus MRR >= 0.60, beating semgrep honestly).

## Speed — cold index and hybrid NL query latency

**Status: `historical` + `UNREPRODUCIBLE` for the table below.** The hyperfine
shape is documented; original `<root>` corpora and `corpora.lock` are not in
this tree, so these numbers are not a live keep. In-tree latency gates are
`asgrep bench` + `.bench-history` (`reproducible-in-tree`).

Illustrative hyperfine shape (does **not** regenerate the published table
without the missing corpora):

```bash
cargo build --profile release-perf -p ast-sgrep-cli
hyperfine --warmup 1 --min-runs 3 --prepare 'rm -rf /tmp/bl.db' \
  './target/release-perf/asgrep --index-path /tmp/bl.db index <root>'
hyperfine --warmup 3 --min-runs 20 \
  "./target/release-perf/asgrep --index-path /tmp/bl.db --json '<query>' <root>"
```

Queries: self = "where is hybrid ranking fused"; ripgrep = "where does
ripgrep apply gitignore rules"; flask = "where does flask dispatch HTTP
requests".

| corpus | cold index mean | NL query p50 | NL query p95 |
|--------|----------------:|-------------:|-------------:|
| self | 416 ms | 13.4 ms | 14.8 ms |
| ripgrep 14.1.1 | 3.91 s | 29.5 ms | 35.1 ms |
| flask 3.0.3 | 335 ms | 13.3 ms | 14.4 ms |

Cold-index figures include hashed-embedding generation (the default `index`
path). They are larger than the older `run-scale.sh` table in
an older methodology note, which indexed with different roots and machine state;
this table is the pinned reference going forward.

## Watch mode -- per-save incremental index work

**Status: `historical` + `UNREPRODUCIBLE`.** `watch-bench.py` is **not** in this
tree. Do not treat the commands as a live harness. Numbers below are a
published record only.

| config | median | p95 |
|--------|-------:|----:|
| hashed embed (default) | 0.837 ms | 1.062 ms |
| --no-embed | 0.438 ms | 1.156 ms |

Measured 2026-07-10 at the ast-sgrep-48p commit. Sidecar rebuilds (tantivy,
semantic IVF) are deferred out of the save path and flushed after a quiet
period. SIGKILL mid-burst leaves a recoverable index (WAL + per-file
transactions): `PRAGMA integrity_check` ok, queries succeed.

## Noise bounds (second run)

Every suite was run twice back-to-back on the same build:

- **asgrep MRR / Recall@k / nDCG@k: identical to three decimals across runs**
  (ranking is deterministic). Any diff > 0.001 on an unchanged corpus and
  commit is a regression, not noise.
- **semgrep MRR: identical across runs** (0.536 / 0.033).
- **ripgrep and ast-grep file-order rows jitter** because both walk files with
  parallel, nondeterministic traversal (self ripgrep MRR 0.061 vs 0.047;
  ripgrep-corpus ast-grep MRR 0.143 vs 0.179). Treat those two rows as
  order-of-magnitude only.
- **Wall-clock timings: informational, +-30% run-to-run** on a busy machine;
  hyperfine p50 figures are stable to ~10%.

## Fusion / stage budget map (e2hc.31)

**Status: `historical` (index, no new fingerprint).** Sub-1ms median is the
in-process `CORE_PARTS` gate (`tests/core/sub1ms.rs`), not CLI vs competitors.
Scorecard: [`fusion-scorecard.md`](fusion-scorecard.md). CLI p95 vs ripgrep /
ast-grep: [`speed.md`](speed.md). Quality ids above unchanged.

## Rules

1. No number may be quoted without a section status tag plus a row in this
   file (or another results doc that cites a fingerprint id here).
2. Rebaselining a `canonical` quality row requires the gold + eval harness
   restored in-tree. Until then the fingerprint stays `UNREPRODUCIBLE`.
3. `eval-bakeoff.py` / `results.json` stamping is **absent** from this tree.
   Do not hand-edit historical scores. Do not claim a new MRR win in a close
   reason.

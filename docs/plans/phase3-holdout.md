# Phase 3 Holdout — Spec

Status: **v1 built, placeholder baseline recorded 2026-10-17** (MRR@10
.531, Hit@100 .950). Originally armed to gate Phase 3 weight candidates;
with Phase 3 cancelled by direction (2026-10-17, no training runs ever),
this holdout now gates any quality-affecting ENGINE candidate before a
frozen-suite run. The gate itself is unchanged.

## Principle

No model ships with ast-sgrep. The shipped program is the engine: native
tokenizer, normalization, chunking, field construction, pooling, field
mixing, fusion, ranking. The token table is learned data, trained offline on
Spark, hash-pinned, loaded with `include_bytes!`. A teacher used offline is
not part of the program and must not be vendored.

The holdout therefore judges **engine behavior**, not table memorization.
The rejected Qwen recipe proved a same-function docstring holdout
(.523 → .571) anti-predicts the suite task (.449 vs .533). This holdout
mirrors the suite task instead: a natural-language behavioral description
ranks functions across a multi-file repo.

## Schema (identical to frozen RepoQA)

Each query: `id, scope, language, query, path, start_line, end_line,
repoqa_start_line, repoqa_end_line, name` plus `provenance` (which
usage evidence the query was written from: test path + symbol, call site,
or issue URL). Same schema means the existing arm driver
(`benchmarks/sgrep_suite/run_asgrep_arm.py`) and scorer
(`run_suite.score_repoqa`) run unmodified against a holdout bench dir —
only the directory contents differ.

## Corpus rules

- ~12 repos, 2 per frozen language (python, cpp, java, typescript, rust,
  go), each pinned by commit SHA with date recorded.
- Permissive licenses only (MIT / Apache-2.0 / BSD). License file verified
  at pin time; copyleft excluded.
- Disjoint from the frozen 60 RepoQA scopes (checked by scope string).
- Full source at the pinned commit, tests included — the whole repo is the
  distractor pool, exactly like frozen.

## Query rules

- Each query is a behavioral description (purpose / input / output /
  procedure, RepoQA style) written from **usage evidence**: a test that
  exercises the target, a call site, or an issue report. Record which.
- Never from the target's own docstring or comments.
- Identifier-stripped: no exact target symbol, file-unique literal, or
  distinctive constant from the gold span.
- Leakage gates (mechanical, in `build_holdout.py --check`):
  1. No 6-gram of the query appears in the target file's comments /
     docstrings.
  2. Query ids and gold spans are disjoint from all frozen query ids,
     descriptions (by SHA-256), and gold spans.
  3. Target's own docstring sentences are excluded even paraphrased —
     enforced by human authorship from tests, spot-checked.

## Metrics and acceptance

- Same as frozen: MRR@10 primary, Hit@1/@10/@100, per-language split.
- A weight candidate must beat the placeholder table on the holdout
  **and** show no frozen regression (RepoQA .533 / CoSQA .240) with no
  per-language collapse before it earns a frozen-suite run.
- Placeholder baseline on the holdout is recorded in
  `benchmarks/results/baselines.md` at construction time, so future
  deltas have a reference.

## Non-goals

- The holdout is the training gate, never training signal: no pair
  construction from it, no early-stopping on it beyond the accept/reject
  comparison.
- Frozen-suite queries and gold spans are banned from every training split,
  unchanged.

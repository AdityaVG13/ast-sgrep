# Phase 3 holdout v1 — placeholder baseline (2026-10-17)

The retry gate from the two rejected Phase 3 tables demanded a held-out
set of repository-level queries whose text is not the target's own
docstring, over multi-file repos disjoint from the frozen suite. This
holdout is that set, and this doc is the placeholder bar every future
candidate must beat *before* any frozen-suite run.

## Corpus

12 permissive-license repos, pinned by commit, disjoint from the frozen
RepoQA scopes and CoSQA snippets: fmt, spdlog, anyhow, httpx, requests,
cobra, viper, commander, clap, zod, mockito, caffeine. Full pins and
SHA-256 in `benchmarks/phase3_holdout/bench.json`.

## Queries

120 hand-written usage→behavior queries (10 fmt, 10 spdlog, 11 anyhow,
10 httpx, 10 requests, 10 cobra, 10 viper, 10 commander, 10 clap,
10 zod, 10 mockito, 9 caffeine), each with frozen-schema gold
(path + start/end line) and a provenance pointer to the exercising
test. Machine gates enforced at build: symbol ban, 6-gram comment
overlap ban, whole-file span ban, symbol-count cap, frozen-overlap ban.
Authored source: `benchmarks/phase3_holdout/queries-authored.json`;
bench copy: `<bench>/holdout-queries.json` (assembled by `build_holdout.py
assemble`). Driver: `benchmarks/phase3_holdout/run_holdout_arm.py`
(same RepoQA scorer as the frozen suite: file rank + line coverage).

## Baseline (placeholder `static:potion-code-16M-v2:256`)

Binary: `target/release/asgrep` built at HEAD `56492b0e` plus the
uncommitted Phase-2/2D worktree (SHA-256 `a8d45440…`; not the 2D
canonical binary — table bytes identical). 120/120 complete, zero
errors/timeouts. Full rows: `<bench>/../holdout-baseline/`.

| slice | n | MRR@10 | Hit@1 / @10 / @100 |
|---|---|---:|---|
| all | 120 | **.531** | .400 / .825 / .950 |
| cpp | 20 | .219 | .050 / .700 / .950 |
| go | 20 | .547 | .400 / .950 / 1.000 |
| java | 19 | .461 | .316 / .684 / .842 |
| python | 20 | .810 | .700 / 1.000 / 1.000 |
| rust | 21 | .461 | .381 / .714 / .905 |
| typescript | 20 | .688 | .550 / .900 / 1.000 |

## Reading

The bar is mid-range by design (.531 vs frozen RepoQA .533 — coincidence,
not calibration). Weakest slice is cpp (.219, mostly spdlog header-only
targets); strongest is python (.810). A candidate that only lifts python
while dropping cpp further fails the "no per-language collapse" clause.
Per the plan, a candidate needs: holdout win **plus** no frozen
regression (.533/.240) — the holdout opens the suite run, it does not
replace it.

## br-dx0 lever-1 result (2026-10-17)

Genre-boilerplate strip (`query.rs`: drop ~45 boilerplate terms for
queries over 15 tokens). Holdout embed-on **.531 -> .561**, no-embed
**.273 -> .308**. Frozen paired arms, same worktree, 600/500 sets:
control RepoQA .698 / CoSQA .317, lever-1 RepoQA **.719** / CoSQA .317
(bit-identical), no per-language collapse — full row with binary SHAs in
`benchmarks/results/baselines.md` (br-dx0 section). Ship gate passed:
holdout win plus no frozen regression.

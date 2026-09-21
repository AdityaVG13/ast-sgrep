# Performance negative results

Ideas that were measured and rejected (or Open pointers until measured).
Project law: `AGENTS.md` (negative-evidence discipline). Every rejection names
its evidence and a retry-condition predicate in plain measurable language
(see `README.md`). Predicates below are written to be directly testable.

Ledger created 2026-09-20. It did not exist before; the seed below is
reconstructed from verifiable repo evidence only (git log, `benchmarks/`).

## Rejected

- **literal-expansion-skip REJECTED 2026-09-21 (skill-loop pass 1).**
  Skipping concept expansion for Literal intent saves ~3.2 ms wall on S1
  (10.0 to 6.8 ms, hyperfine n=10, noisy host, M5 Max) with byte-identical
  hits, but zeroes the golden-pinned `query_expansions` field (5
  prefix-keyword rows to 0), breaking golden S1. Retrieval provably ignores
  expansions for non-Conceptual intents, but the JSON field is output
  contract. Retry predicate (form 4, contract-change): re-open only after a
  product decision removes or redefines `query_expansions` for literal
  queries and golden S1 is re-blessed to the new contract by an explicit
  owner.
<!-- pass 2, 2026-09-21, asgrep @6b0db169 release-perf sha256:467489cb3145f709 -->
- **sqlite-prepare-cache REJECTED 2026-09-21 (skill-loop pass 2).** Routing the repeated per-query probes (`search_data_versions` ~6 parses/one-shot, `PRAGMA data_version` 3–5×/one-shot) through rusqlite `prepare_cached` showed no measurable one-shot win: order-swapped interleaved A/B vs pristine-HEAD control (n=16 each, M5 Max, same block) gave pooled means 9.85ms control vs 9.95ms cached, within ±0.5ms host σ. Isomorphism held (4/4 goldens byte-identical). New attribution: of S1's 47% sqlite3RunParser CPU, ~45% nests under `configure_connection_inner`/`execute_batch` (first-touch schema load) and only ~2% under genuine query prepares, bounding the cacheable fraction below resolvability. Reverted, no code change (experiment patch `/tmp/pass2.patch`, host-local). Retry predicate: re-open only with (a) a same-block order-swapped interleaved A/B (n≥50, control CV <2% on a quiet host) showing ≥3% one-shot mean win, or (b) profile evidence that per-query prepares exceed 10% of one-shot CPU after connection-setup costs change.

## Prior wins (kept; do not re-litigate without new evidence)

- `c6f45d92` structural-stage row budgets + lazy excerpts + match-index
  selection: p50 -31%, p90 -42% (`benchmarks/results/speed.md`, 2026-09-19).
- `613d6bfd` unique-hybrid Code Mode path, skip unused stamps.
- `bd20dd2d` cascade-file pattern seek, skip conceptual structure.
- `817a7d9c` mmap hybrid, nprobe cap 8, skip short-token LIKE.
- `657a77c6` rank IVF from mmap, top-N column fetch.
- `bb31f097` stamp memo, empty-embed guard, batched fetch, SQL GLOB reverify.
- `251446fe` lazy structural excerpts (attach after fusion).
- `0a08adcf` vocab bulk-preload, IN-list buckets, resident read cache.

## Measured 2026-09-20 (bake-off; see `benchmarks/results/speed.md`)

- **One-shot retrieval overhead FIXED same-day** (was: warm
  `literal:SearchHit` p95 171 ms). Flamegraph (samply, 3/3 captures,
  symbolicated via atos) named the hotspot: ~95% of one-shot wall under
  `literal_pass` -> `IndexStore::line_corpus`: full `lines` table
  materialization plus per-line token indexing on every cold query. Fix:
  corpus is cached-only (`line_corpus_if_cached`); cold queries take the
  trigram/SQL path, warmed sessions keep the memchr scan. Result: one-shot
  literal p95 **7.9 ms** (beats rg 13.0 ms 1.65×, ties warm tgrep 7.5 ms);
  serve path steady at p50 ~1.8 ms. Pinned by
  `tests/core/literal_warm_cold_parity.rs` (C1 no-implicit-load, C2
  literal-lane full-vector parity, C3 hybrid file-set parity) and two
  score-only golden hunks (+0.26% on one fused hit: cold prefilter now
  emits matching-line evidence instead of corpus file-stub
  representatives). Retry predicate: re-open only if one-shot literal
  regresses past 1.3× vs pinned rg on the same workdir protocol across 3
  interleaved rounds.
- **NL semantic 2.34 s VOIDED as load contamination** (bake-off row).
  Re-measured same query/corpus/index: fixed binary 17.8/18.5 ms (n=12,
  tight 16.9 to 19.0) and HEAD-built control 10 to 20 ms (5/5 fast). The
  bake-off session ran under another agent's build storm. No hotspot, no
  flamegraph owed. Retry predicate: re-open only if NL semantic exceeds
  100 ms p95 on the workdir protocol in a quiet system across 2 rounds.
- **Warm stub→matching-line serve cost: inconclusive, accepted.**
  `warm_distinct` pooled p50 1.97 ms (after, 4 rounds) vs 1.82 ms
  (before, 4 rounds, incl. historical) with overlapping ranges; mechanism
  estimate ~0.01 ms/query (bounded memchr over admitted files). Within
  run noise; no tuning (would be fitting noise). Quality A/B null:
  self gold per-query identical, invent-path 1.0 both. Retry predicate:
  re-open only with a per-query interleaved A/B showing >10% serve
  regression attributable to representative location.
- **Resolved 2026-09-20: trigram df-trust tests vs read-only store.**
  Root cause was `PRAGMA query_only`, not `SQLITE_OPEN_READ_ONLY`: the flag
  alone fail-closes main writes (verified: main DDL fails, temp DDL works)
  while query_only also blocked temp DDL, silently disabling the whole df
  lever (Match shortcut, cascade drop_common/foothold/rarity) since
  `ea7ca709`, with c2/c2b red-flagging it. Fix: drop query_only, keep the
  flag, and session-gate the shortcut (`TrigramDfCache::arm` from
  `warm_search_path` only, sticky-session open is its sole caller), because
  the ungated vocab preload costs ~93ms on the 9ms one-shot path (measured
  102.3ms mean). Gated A/B: one-shot p95 10.9 vs base 11.7 (same block,
  n=15, no tax), serve p50 2.2→1.0ms (~2.2×). c1/c2/c2b/c3 now arm
  explicitly; new cold-gating test asserts no vocab on cold search.
- **Hillclimb 2026-09-20, kept (2).** E-open-1: open-gate `status()`
  (six COUNT(*)s + meta reads) replaced by LIMIT-1 `has_indexed_files`
  probe at all four gate sites: one-shot mean 9.9→8.7ms same-block
  (−12%). E-like-1: dropped redundant `lower()` in `or_like_filter`
  (stock LIKE is already ASCII-CI; `x LIKE p` ≡ `lower(x) LIKE
  lower(p)`, no ICU, `case_sensitive_like` unset): caller-scan shape
  1.96→0.79ms (2.5x), hybrid SnapshotStamp interleaved −25% cold/−22%
  warmed, serve p50 −12% / p99 −70%. Methods fixed along the way: bench
  `--warmed` second block (separate `warmed:` labels) and response-cache
  OFF in bench (repeats measured hash hits: 0.35ms vs 3.45ms true).
- **Hillclimb 2026-09-20, killed (6).** Cap-cut 100→16 fetches: pool
  feeds the critic (823 lane), quality-load-bearing; re-open only with
  per-lane pool caps plus golden adjudication showing no rank change.
  Exact+prefix skip-substring: recall probe shows 21/240 identifiers need
  substr-only rows and exact+prefix rarely saturates sql_limit, so the
  fallback would run anyway; re-open only with a saturating fast path
  preserving the 9% recall. Fat LTO: untried, I/O-bound (sqlite3_step
  ~80%) so expected <1%; re-open only with a same-block A/B ≥5%.
  Startup diet: no lever (no runtime/thread-pool init on the path;
  sqlite TEXT page-in structural); re-open only with xctrace/dtrace
  startup visibility naming a >10% phase. Needle memo: killed by probe.
  Hybrid runs 4 distinct (needle, mode) pairs, nothing to dedup.
  PGO/mimalloc: deferred as infra-heavy; re-open only with ≥10%
  same-block A/B.
- **Resolved 2026-09-20: `asgrep bench` exits 2 on quarantine_cv.** UX
  decision: quarantine warns by default (exit 0, measurements printed,
  run recorded but never blessed, since baseline writes already refuse
  quarantines) so bench works as a measurement tool on noisy hosts; a
  measured regression still fails; `ASGREP_BENCH_STRICT=1` restores
  fail-hard for future CI. The 926% CV was load contamination (n=100,
  not a cold-first artifact), so the all-iteration statistic is untouched.
  Locked by `bench.rs` policy + `keep_gate.rs` verdict unit tests (the
  machine_contracts comment claiming such tests now holds) and e2e runs
  (soft: exit 0 + warning; strict: exit 2 JSON error envelope).
- **Cold index is 17.4 s / 231 MiB** (646 files), ~9 s + 107 MiB of it
  vectors. Lexical/AST base alone is 7.9 s / 133 MiB, still 3× the
  August wall on a smaller file count (corpus differs; not controlled).
- **Pattern path won 4× vs ast-grep** (14.6 vs 60.1 ms). Do not "fix" the
  structural path while the fusion path is the outlier.

## Open (measure before claiming)

- **Small-corpus warm lexical gap.** 2026-08-28: asgrep p95 19.0 ms vs
  ripgrep 11.1 ms (1.7×) on the self tree (`head-to-head.md`). No fix
  attempted yet. Retry predicate: re-measure with the 2026-09-19 candidate
  binary on the same corpus protocol; only open a fix bead if the gap
  reproduces at >=1.3× across 3 interleaved rounds.
- **Small-corpus structural gap.** 2026-08-28: asgrep p95 129 ms vs ast-grep
  26.5 ms (4.9×, latency-only). Retry predicate: same as above; a fix must
  also hold `machine_contracts` + `search_correctness_epics` green and keep
  match-set parity on the hand-pattern suite.
- **Competitor bake-off methodology.** `tgrep` 1.0.9 done (warm server,
  speed.md 2026-09-20 rows). `fff` done: identified as `fff-mcp` 0.10.6,
  timed via in-tree `benchmarks/fff_grep_leg.mjs` (warm p95 0.5 ms,
  ranked 20/54, latency-only). `zgrep` dispositioned OUT: it greps gzip
  members, a decompression surface, not a source-search competitor; retry
  predicate: only if asgrep ever ships compressed-corpus search. semgrep
  stays a deep-analysis reference, not a retrieval racer.

## History-mining note (2026-09-20)

`cass` available; full 22-term battery run before the one-shot literal fix
(60d, lexical, limit 5 to 50): ast-sgrep-workspace hits were audit prose
mentioning the failure terms (`UNREPRODUCIBLE` tagging, `jell`-deferral
docs, capability inventories citing `compact-drops-provenance` /
`MCP-no-fusion`) plus one npm-script snippet: no prior perf-fix
rejection, revert, or within-noise signal against this change. `git log`
perf commits above are the interim record.

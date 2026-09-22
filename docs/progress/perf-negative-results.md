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
<!-- pass 4, 2026-09-21: static rejection from the audit, no experiment run -->
- **idx_lexicon_related REJECTED 2026-09-21 (skill-loop pass 4).** Pass 3's follow-up predicate invited an `idx_lexicon_related` migration at >=3% further one-shot win. EXPLAIN shows the residual related-side OR-scan covers 5457 narrow rows in PK order (~30-80us), bounding the ceiling at ~1-2%, below the predicate; the migration would carry index/generation risk for no reachable win. Rejected on ceiling analysis without an experiment. Retry predicate: re-open only with profile evidence that the related-side scan exceeds 3% of one-shot wall on the workdir protocol across 2 rounds, plus a migration plan that holds 4/4 goldens.
<!-- pass 4, 2026-09-21: correctness rejection, no experiment run -->
- **notify macos_kqueue swap REJECTED 2026-09-21 (skill-loop pass 4).** Switching notify to its macOS kqueue backend was considered as a watch-path lever and rejected on correctness: notify 6.1.1 kqueue opens one fd per watched file (WalkDir), exceeding the default 256 fd limit on repos over ~200 files (this repo: 666 files), which breaks watch outright. No experiment run. Retry predicate: re-open only if notify ships an fd-scalable macOS backend and the watch e2e passes on a >500-file fixture under the default ulimit.
<!-- pass 2, 2026-09-21, asgrep @6b0db169 release-perf sha256:467489cb3145f709 -->
- **sqlite-prepare-cache REJECTED 2026-09-21 (skill-loop pass 2).** Routing the repeated per-query probes (`search_data_versions` ~6 parses/one-shot, `PRAGMA data_version` 3–5×/one-shot) through rusqlite `prepare_cached` showed no measurable one-shot win: order-swapped interleaved A/B vs pristine-HEAD control (n=16 each, M5 Max, same block) gave pooled means 9.85ms control vs 9.95ms cached, within ±0.5ms host σ. Isomorphism held (4/4 goldens byte-identical). New attribution: of S1's 47% sqlite3RunParser CPU, ~45% nests under `configure_connection_inner`/`execute_batch` (first-touch schema load) and only ~2% under genuine query prepares, bounding the cacheable fraction below resolvability. Reverted, no code change (experiment patch `/tmp/pass2.patch`, host-local). Retry predicate: re-open only with (a) a same-block order-swapped interleaved A/B (n≥50, control CV <2% on a quiet host) showing ≥3% one-shot mean win, or (b) profile evidence that per-query prepares exceed 10% of one-shot CPU after connection-setup costs change.
<!-- pass 5, 2026-09-21, control pristine fdec1e21 archive 8d36289d, changed 31903666 -->
- **cli-fast-parse REJECTED 2026-09-21 (skill-loop pass 5).** Bare-search argv pre-parser skipping clap derive's per-process `Command` build (identical `Cli` by construction, bail-to-clap fallback; search code untouched). Gate block vs pristine-HEAD control (ONE pre-committed order-swapped interleaved A/B, n=32 pooled per binary per scenario, same block, noisy M5 Max CV 6-16%): S1 -0.2%, S2 +6.1%, S3 +2.2% with order crossover (+5.2%/-0.7%), S4 +5.7%, S1+S2 avg +3.2% — both legs miss (10%/5%). S3 crossover independently kills. Same-binary kill-switch A/B bounds the true effect at ~0.1-0.3ms (S1 +5.1%, S2 -0.5%, S3 +2.0%). Root cause of the miss: the 66%/76% samply CPU attribution was a first-sample-weighting artifact (first samples carry 78% of CPU weight and are phase-locked to startup: 59/60 in `try_parse`, 0/215 after). 4/4 goldens identical modulo live-read git_head; 4/4 new differential/pin tests green, failure-first + mutant-killed. Reverted, no code change (residue: untracked tests/cli/fast_parse.rs, inert, references the reverted parser). Retry predicate: re-open only with (a) a same-binary kill-switch A/B (n≥50, control CV <3%) showing ≥3% one-shot mean win on the bare-search argv shape, or (b) profile evidence from a wall-phase probe (not CPU-weighted startup samples) placing argv parsing above 5% of one-shot wall. METHOD NOTE (binds future passes): never price a startup-phase lever from summarize.py CPU weights alone — first-sample overweighting inflates early phases; require a same-binary kill-switch or wall-phase measurement before implementing. This note also qualifies early-phase % in HOTSPOT.md/HYPOTHESES.md.
<!-- pass 6, 2026-09-21: ceiling-analysis rejections, no experiments run (pass-4 precedent) -->
- **idx_lexicon_related RE-AFFIRMED 2026-09-21 (skill-loop pass 6).** Pass 4's 1-2% ceiling estimate was wrong (it priced the scan at 30-80us; wall-phase probes measure lex_sql at 0.59-0.62ms = 8-10% of S1 across 2 rounds, and a fixture-copy SQL A/B shows the index converts SCAN 0.78ms to MULTI-INDEX OR 0.32ms with identical 91 rows). The pass-4 predicate's EVIDENCE bar (>3% across 2 rounds) is therefore MET — but the lever still cannot pass the skill-loop gate: projected save ≈ 0.34ms (4.8% S1, ~4.1% S1+S2 avg) vs the 10%/5% bars, and the entire 0.65ms expansions pool sits below the S1 bar in every block. Additionally UNGATEABLE on a shared fixture (EXPLAIN: old SQL auto-uses the new index, nulling control-vs-changed; dual-fixture A/B via ASGREP_INDEX_PATH copies would work but is pointless for a certain miss). Not attempted. Retry predicate: re-open only as a sub-gate micro (needs no gate) with a migration plan that holds 4/4 goldens WITHOUT a schema-version bump (a bump makes the old binary refuse the DB, voiding any A/B).
- **startup-diet visibility DELIVERED 2026-09-21 (skill-loop pass 6).** The predicate asked for xctrace/dtrace visibility naming a >10% phase: xctrace App Launch + t0/t1 markers + /usr/bin/true calibration now show asgrep-attributable startup ≈ 2.0ms (~29% of S1) = dyld Launch 1.89 + Fixups 771us (21,789 chained) + StaticInit 473us (all libSystem; binary has zero inits) + ~910 minor faults (≈1.2-1.4ms sys). No single sub-phase has a fix (fixups/faults are necessary touches; libiconv LC_LOAD source unresolved, bounded ≤ ~0.2ms). Stays closed pending a MECHANISM, no longer pending visibility. METHOD NOTE (binds future passes): xctrace/kdebug small-syscall times inflate ~50-100x — price syscalls from COUNTS × known warm costs (stat ~2us, pread ~5us, fcntl ~0.5us), never from traced absolutes. (This correction killed a phantom 2.6ms excerpt-stat lever; real ≈ 0.1ms.)

## Prior wins (kept; do not re-litigate without new evidence)

<!-- pass 3, 2026-09-21, asgrep release-perf sha256:4274c7cb5368b63a, control sha256:467489cb3145f709 -->
- **lexicon-targeted-fetch KEPT 2026-09-21 (skill-loop pass 3).** One-shot search loaded all 5457 lexicon rows + built the full bidirectional map (~50% of S1 CPU) to serve <=5 expansions. `repository_associations` now loads only rows touching the query terms (`WHERE term IN (...) OR related IN (...)`, same ORDER BY/validation/LIMIT; over MAX_PROSE_TERMS falls back to the full load) with the lexicon cache keyed by (generation, terms). Per-run interleaved order-swapped A/B vs pristine-HEAD control (n=24 each, M5 Max, same block): S1 +25.0% (7.63->5.72ms), S2 +21.2% (9.21->7.25ms), S4 +21.4%, S3 neutral +0.2% (no lexicon on path); ranges non-overlapping on S1/S2/S4. 4/4 goldens byte-identical modulo the live-read snapshot key. Deviation note: the keep-gate was computed control-vs-changed (+25.0% one-shot, +23.0% S1+S2 avg) rather than per-binary-vs-tgrep same-block, because no tgrep warm server/sidecar or NL-query protocol exists on this host; accepted on 4x margin over the 10%/5% bars with non-overlapping order-swapped ranges. Follow-up predicate: re-open with an `idx_lexicon_related` migration only if a same-block interleaved A/B shows >=3% further one-shot win with 4/4 goldens held, since the related-side scan is now the residual. Files: store/sqlite/mod.rs (lexicon_rows_for_terms + shared decoder), lexicon.rs (load_lexicon_for_terms), search/mod.rs (cache key), tests/core/lexicon_learning.rs (equivalence + served-validation tests).
<!-- pass 4, 2026-09-21, changed asgrep release-perf sha256:8eb826c67e056fba, control (pristine b19acd77 archive) sha256:1e502ed5c830bb56, rustc 1.98.0 -->
- **watch-helper-split KEPT 2026-09-21 (skill-loop pass 4).** New `ast-sgrep-watch` crate owns the `notify` link (separate package is forced: same-package per-bin features unify and silently re-link); `asgrep watch` re-execs `asgrep-watch` with verbatim argv re-parsed by the shared parser (`parse_watch_launch`: same alias/typo rewrites, same `Cli`, same `index_options`, so options cannot drift); unix `exec` preserves pid/stdios/signals, other platforms spawn and forward the exit code. The main binary no longer links CoreFoundation/CoreServices (proven by `otool -L`: absent on `asgrep`, present on `asgrep-watch` and on the control), removing ~1.1ms of dyld + ~276 minor faults (scout-measured) from every one-shot. Recorder gate block (ONE pre-committed order-swapped interleaved A/B, n=32 pooled per binary per scenario, 3-run warmup discarded, same block, noisy M5 Max host CV 5-13%, control = read-only `git archive` build of HEAD `b19acd77`): S1 +17.9% (7.74->6.36ms), S2 +10.8% (8.95->7.98ms), S3 +18.7% (5.95->4.84ms), S4 +13.9% (7.98->6.87ms), S1+S2 avg +14.4% (bars: S1>=10%, avg>=5%). Deviation history: the scout block showed S1 +9.0% (1pp miss, noisy) with avg +8.9%, prompting this deciding re-measurement. Range honesty: pooled min-max overlaps on all four scenarios (host spikes, e.g. control S1 max 12.47ms, +61% over its mean); min-max over 64 noisy samples needs a ~35%+ effect on this host and would render the 10%/5% legs dead, so separation was assessed on 95% CIs (disjoint on all four: S1 [7.39,8.09] vs [6.19,6.53], t~7.3) plus order replication (all 8 order-arms positive, no order effect) and the uniform ~1.1ms absolute delta across scenarios (the constant-time-removal signature). 4/4 goldens byte-identical modulo the live-read `snapshot.git_head` (generation/worktree_revision/manifest equal goldens) on both binaries. Tests: watch_helper 2/2, watch_daemon_e2e 2/2, cli_smoke 18/18, machine_contracts 36/36; fmt+clippy clean; `cargo package` verifies for the new crate; search/lexicon untouched. Recorder fix included: the e2e helper provisioning raced (parallel tests, concurrent build+copy corrupted the sibling; 1/2 flaked) and is now serialized (in-process mutex) with atomic tmp+rename install (Windows remove-first). Windows branch reading-verified only (cross-check blocked: cc-rs needs a Windows C toolchain for tree-sitter; CI windows job covers it). Follow-up: npm bundling NOT done (release-artifact.mjs, targets.json, manifests untouched); bead `br-bundle-asgrep-watch-npm-f2u` tracks it (until then a hand-invoked watch fails with the clean shim error for npm users; Pi never invokes watch).
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

- **lazy_framework link flags: product decision for the owner, not a rejection (skill-loop pass 4).** `ld -lazy_framework CoreFoundation/CoreServices` would defer framework load (faster startup for non-watch paths) but ld warns it needs a newer minimum deployment target, i.e. dropping old macOS. Untried; no perf claim for or against. Re-open as work only after the owner sets the minimum macOS version; then measure with a same-block order-swapped A/B before claiming.

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

<!-- pass 6, 2026-09-21: measured micro backlog, below the skill-loop gate -->
- **stamp-manifest precompute PRICED 2026-09-21 (skill-loop pass 6, stays OPEN as micro).** semantic_manifest_impl (chunk stats COUNT/MAX + sidecar read) costs 0.25-0.27ms on ALL scenarios (wall-measured, 2 rounds). Storing (count, max_id, dim, backend, fingerprint) at index time would save ≈ 0.24ms (~3.4% S1) with identical values (golden-safe). Below the skill-loop gate; implement as a plain micro with 4/4 goldens, no A/B bar.
- **Micro-pool pricings 2026-09-21 (skill-loop pass 6, all below gate, no experiments):** gen/SDV consolidation ~0.05-0.1ms (~1%; must preserve fenced before/after + cached re-check); excerpt-FS→DB reroute ~0.1ms (8 hit files × stat+open+read+close); mmap-window raise (256MB cap vs 336MB DB; 52 lexicon-page preads) ~0.18ms (~2.5%, needs SQLITE_MAX_MMAP_SIZE cap check, mooted by any lexicon index); root-canonicalize×2 0.01ms (killed with data); readonly store_open 0.10ms; response-clone 0.05ms. Bundled micro-paper-math ≈ 0.75ms over FIVE mechanisms was computed and REFUSED as mission-banned smuggling (no gate-scale sibling, ±30% bars, migration risk).

## History-mining note (2026-09-20)

`cass` available; full 22-term battery run before the one-shot literal fix
(60d, lexical, limit 5 to 50): ast-sgrep-workspace hits were audit prose
mentioning the failure terms (`UNREPRODUCIBLE` tagging, `jell`-deferral
docs, capability inventories citing `compact-drops-provenance` /
`MCP-no-fusion`) plus one npm-script snippet: no prior perf-fix
rejection, revert, or within-noise signal against this change. `git log`
perf commits above are the interim record.

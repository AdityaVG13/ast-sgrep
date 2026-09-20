# Speed benchmarks

Companion to [`baselines.md`](baselines.md) (retrieval quality). This file
owns **wall-clock CLI latency**. Any speed number quoted elsewhere must
trace back to a dated row here or carry its own reproduce command.

Status tags: [`benchmarks/README.md`](../README.md).

## 2026-09-20 static trigram prior: cold one-shot Match (self corpus, working tree)

**Status: `reproducible-in-tree`.** Cold one-shot search answers from a
baked trigram rarity table (`benchmarks/trigram_bake.py --min-count 100`
over the local cargo registry: 5,167 crates, 9.9 GB, 138,909 trigrams
kept of 824,147) instead of degrading to the full-phrase MATCH: the 1–2
relatively rarest needle trigrams drive the probe, a 400-row scan budget
falls back to the phrase on locally-flooding picks. Zero per-process
cost (binary search, no I/O); binary grows +1.1 MB (37.5 → 38.6 MB).
Working tree atop `c28bce4a`, uncommitted — re-run after commit to
re-pin. Binaries: `/tmp/asgrep-static` (this tree) vs `/tmp/asgrep-hillF`
(pre-static hillclimb) on the same index.

| Provenance | value |
|------------|-------|
| date | 2026-09-20 |
| commit | working tree atop `c28bce4a` (static prior, uncommitted) |
| machine | Apple M5 Max, 18 cores (arm64), macOS 26.6.2, APFS SSD |
| corpus | tracked files → rsync workdir, **648 files**; shared `/tmp/asgrep-speed.db` |
| build | `cargo build --profile release-perf -p ast-sgrep-cli --bin asgrep` |
| tools | ripgrep 15.1.0, hyperfine 1.20.0 |

p95 is nearest-rank on hyperfine's raw samples: `idx = floor((n - 1) * 95 / 100)`.

| Surface | n | p50 | p95 | note |
|---------|--:|----:|----:|------|
| one-shot `literal:SearchHit`, static | 15 | 7.2 ms | 7.8 ms | tie vs hillF; beats rg 1.59× mean |
| one-shot `literal:SearchHit`, hillF | 15 | 7.2 ms | 7.7 ms | same block |
| one-shot `rg -n SearchHit` | 15 | 11.6 ms | 12.4 ms | same block |
| one-shot absent `literal:zzquux_no_such_token`, static | 15 | 7.3 ms | 9.2 ms | **1.20×** mean vs hillF (empty postings prove absence) |
| one-shot absent, hillF | 15 | 9.1 ms | 11.3 ms | same block |
| one-shot `literal:SnapshotStamp`, static | 15 | 7.6 ms | 8.1 ms | 1.06× mean vs hillF |
| one-shot `literal:SnapshotStamp`, hillF | 15 | 8.1 ms | 8.7 ms | same block |
| one-shot flood `literal:value`, static | 15 | 8.2 ms | 8.9 ms | 1.01× hillF (noise; wasted probe + fallback unmeasurable) |
| one-shot flood `literal:value`, hillF | 15 | 7.9 ms | 9.5 ms | same block |

Notes:

- The lever moves total time only where MATCH evaluation dominates:
  absent/long needles (phrase intersects every trigram posting list;
  one rare trigram short-circuits). Few-hit needles tie — startup +
  open/gate + cold pages are identical.
- No-regress: 27/27 CLI outputs byte-identical static vs hillF (24
  literal needles incl. short/non-ASCII/flood/absent + pattern +
  semantic + hybrid); 7/7 `trigram_shortcut`, 4/4
  `literal_warm_cold_parity`, 5/5 `trigram_df` unit, 5/5 bench
  `default` identity ok. Bench `self` suite fails identically on both
  binaries (pre-existing fixture/golden mismatch on this tree state —
  out of scope, recorded here, not chased).
- Static picks on this corpus: `searchhit → chh AND hhi`,
  `snapshotstamp → ots AND tam`, `zzquux_no_such_token → zzq AND uux`.

## 2026-09-20 hillclimb: open gate + LIKE lanes (self corpus, working tree)

**Status: `reproducible-in-tree`.** Two kept profile-guided wins atop
`c28bce4a`, uncommitted — re-run after commit to re-pin. E-open-1: the
open gate ran full `status()` (six COUNT(*)s) for a boolean; now a
LIMIT-1 probe. E-like-1: redundant `lower()` dropped from LIKE lanes
(behavior-identical under stock SQLite). Bench gained `--warmed` (cold
+ warmed blocks, separate history labels) and measures with the
response cache off (repeats previously timed hash hits).

| Provenance | value |
|------------|-------|
| date | 2026-09-20 |
| commit | working tree atop `c28bce4a` (hillclimb, uncommitted) |
| machine | Apple M5 Max, 18 cores (arm64), macOS 26.6.2, APFS SSD |
| corpus | one-shot/hybrid: tracked files → rsync workdir, **648 files**; serve: repo `.asgrep/index.db` |
| build | `cargo build --profile release-perf -p ast-sgrep-cli --bin asgrep` |
| tools | ripgrep 15.1.0, hyperfine 1.20.0 |

p95 is nearest-rank on hyperfine's raw samples: `idx = floor((n - 1) * 95 / 100)`.

| Surface | n | p50 | p95 | note |
|---------|--:|----:|----:|------|
| one-shot `literal:SearchHit`, pre-hill | 15 | 10.8 ms | 13.7 ms | same block as hill + rg |
| one-shot `literal:SearchHit`, hill | 15 | 9.1 ms | **10.1 ms** | E-open-1; beats rg 1.39× mean |
| one-shot `rg -n SearchHit` | 15 | 12.9 ms | 13.5 ms | — |
| hybrid `SnapshotStamp` warmed, pre-hill | 10×2 | — | — | interleaved avg 0.84/0.97 ms |
| hybrid `SnapshotStamp` warmed, hill | 10×2 | — | — | interleaved avg 0.67/0.73 ms (**−22%**, E-like-1) |
| serve distinct-query p50, hill | 240×2 | 0.64/0.61 ms | (p99) 7.0/2.3 ms | `warm_distinct.mjs`; pre-hill 0.70/0.70 adjacent |

Notes:

- One-shot decomposition (same-session probes): ~3.8 ms startup
  (`--version`; rg 2.4), ~2.1 ms open+gate pre-fix, ~4.4 ms first-search
  (FTS5 + ~100 cold row fetches), ~0.3 ms render. Search steady-state is
  0.24 ms; first-search is cold-page dominated.
- Killed with data (see perf ledger): 100→16 fetch cap (pool feeds the
  critic), exact+prefix skip-substring (9% need substr-only recall),
  needle memo (4 distinct needles), fat LTO (I/O-bound), startup diet
  (no lever found), PGO/mimalloc (deferred as infra-heavy).

## 2026-09-20 df shortcut resurrection, session-gated (self corpus, working tree)

**Status: `reproducible-in-tree`.** Dropping `PRAGMA query_only` (keeping
`SQLITE_OPEN_READ_ONLY`) re-enables the ephemeral fts5vocab df cache that
`ea7ca709` silently disabled, but the ~93ms vocab preload is lethal on the
9ms one-shot path (ungated binary: 102.3 ms mean) — so the shortcut arms
only via `warm_search_path` (sticky/serve sessions). Binaries built from a
working tree atop `dc35e5c7` with that change uncommitted — re-run after
commit to re-pin.

| Provenance | value |
|------------|-------|
| date | 2026-09-20 |
| commit | working tree atop `dc35e5c7` (df session-gating + bench quarantine policy, uncommitted) |
| machine | Apple M5 Max, 18 cores (arm64), macOS 26.6.2, APFS SSD |
| corpus | one-shot: tracked files → rsync workdir, **648 files**; serve: repo `.asgrep/index.db` (322 MiB) |
| build | `cargo build --profile release-perf -p ast-sgrep-cli --bin asgrep` |
| tools | ripgrep 15.1.0, hyperfine 1.20.0 |

p95 is nearest-rank on hyperfine's raw samples: `idx = floor((n - 1) * 95 / 100)`.

| Surface | n | p50 | p95 | note |
|---------|--:|----:|----:|------|
| one-shot `literal:SearchHit`, base | 15 | 10.6 ms | 11.7 ms | same hyperfine block as gated + rg |
| one-shot `literal:SearchHit`, gated | 15 | 10.4 ms | **10.9 ms** | no one-shot tax (cold search never creates the vocab) |
| one-shot `rg -n SearchHit` | 15 | 13.8 ms | 17.2 ms | gated beats rg ~1.3× p50 |
| serve distinct-query p50, base | 240×2 | 2.45/2.22 ms | (p99) 9.8/5.3 ms | `warm_distinct.mjs`, 2 rounds |
| serve distinct-query p50, gated | 240×2 | 1.07/1.00 ms | (p99) 10.7/3.3 ms | **~2.2×**; back-to-back ungated 0.99 = gated 1.01 (identical steady state) |

Notes:

- The ungated intermediate (query_only dropped, no arming) timed 102.3 ms
  mean one-shot: vocab ensure+preload on every cold process. The gated
  binary matches base one-shot while keeping the full warmed win —
  session-gating is load-bearing, not cosmetic.
- An earlier ungated serve block read p50 0.63 ms; a back-to-back rerun
  reads 0.99 (ungated) vs 1.01 (gated). The 0.63 was a quiet-machine
  outlier; ~1.0 ms is the in-session warmed figure. Gated serve now sits
  ~2× above fff's 0.5 ms warm-grep cell (different match-set semantics).
- `asgrep bench` quarantine now warns (exit 0, measurements printed)
  instead of failing the run; `ASGREP_BENCH_STRICT=1` restores fail-hard.
  E2E: 3-iteration run quarantines at cv 173% → exit 0 + stderr warning;
  strict → exit 2 JSON error envelope.

## 2026-09-20 one-shot literal fix (self corpus, working tree)

**Status: `reproducible-in-tree`.** Same workdir protocol as the bake-off row
below, after the one-shot literal fix (no implicit RAM-corpus load; trigram
path for cold queries). Binary built from a working tree atop `ccbccf19`
with that fix uncommitted — re-run after commit to re-pin.

| Provenance | value |
|------------|-------|
| date | 2026-09-20 |
| commit | working tree atop `ccbccf19` (one-shot literal fix, uncommitted) |
| machine | Apple M5 Max, 18 cores (arm64), macOS 26.6.2, APFS SSD |
| corpus | tracked files → rsync workdir: **646 files** indexed at timing time (649 after the tgrep sidecar landed; scan blocks ran before it) |
| build | `cargo build --profile release-perf -p ast-sgrep-cli --bin asgrep` |
| rustc | 1.98.0 |
| tools | ripgrep 15.1.0, BSD grep 2.6.0, ast-grep 0.45.3, tgrep 1.0.9 (warm server, `--shell=none` client timing), fff-mcp 0.10.6 (warm server, `benchmarks/fff_grep_leg.mjs`), hyperfine 1.20.0 |
| index | schema 16, hashed semantic embedder; 6,296 symbols; `index.db` **231 MiB** |

p95 is nearest-rank on hyperfine's raw samples: `idx = floor((n - 1) * 95 / 100)`.

| Surface | n | p50 | p95 | comparator p95 | note |
|---------|--:|----:|----:|-------------:|------|
| warm `literal:SearchHit` | 15 | 7.6 ms | **7.9 ms** | rg 13.0 ms, grep 50.2 ms, tgrep 7.5 ms | asgrep beats rg 1.65×; near warm-tgrep p95 (tgrep leads p50 4.7 vs 7.6) |
| fff warm `grep SearchHit` | 15 | 0.4 ms | **0.5 ms** | — | fff-mcp 0.10.6 warm server; ranked 20/54 shown vs ~300 literal lines — latency-only, not match-set |
| warm `pattern:SearchHit` | 12 | 9.7 ms | **10.2 ms** | ast-grep 63.3 ms | asgrep wins 6×; path untouched by the fix |
| warm `semantic 'credential renewal'` | 12 | 17.8 ms | **18.5 ms** | — | range 16.9–19.0 ms; the bake-off 2.34 s did not reproduce (see note) |
| cold index (`asgrep index .`) | 8 | 14.1 s | **22.3 s** | — | range 13.9–23.5 s; outliers from system load, path untouched |
| serve distinct-query p50/p99 | 240 | 1.7–1.9 ms | (p99) 4.4–6.3 ms | — | `warm_distinct.mjs`, 2 rounds; warm corpus intact |

Notes:

- The 171 ms → 7.9 ms literal drop is the fix, not corpus drift: same
  corpus class as the bake-off row (646 vs 649 indexed files), same query.
  Flamegraph (3/3 captures) put ~95% of one-shot wall under `literal_pass`
  → `LineCorpus::load`; cold queries now take the trigram path.
- Over-cap selection changed with the path: `SearchHit` has 298 matching
  lines and the lane caps at 100 candidates, so the shown 16 are the
  path-ordered head of the first-100 FTS postings rather than the global
  path-ordered head. All shown hits verified true matches; under-cap
  queries are hit-identical across paths (`literal_warm_cold_parity`).
- tgrep timed with `--shell=none` after the scan blocks (its `.tgrep/`
  sidecar postdates them); one 8.1 ms outlier in 15 runs. asgrep is a
  cold one-shot process here vs tgrep's warm server.
- rg showed a statistical-outliers warning, but separation is clean: rg
  min 11.1 ms > asgrep max 8.3 ms across the interleaved block.
- fff leg: `node benchmarks/fff_grep_leg.mjs <corpus> SearchHit 15`.
  Spawn-to-first-result 68 ms (scan + content index + first query); warm
  calls p50 0.4 / p95 0.5 ms. fff surfaces 54 ranked matches where
  exhaustive tools find ~300 lines — it owns sub-ms ranked file-finding,
  not exhaustive grep. Closing the serve-path gap (1.8 ms vs 0.4 ms) is
  the next warm-latency bead; it needs its own flamegraph first.
- The bake-off row's 2.34 s NL-semantic cell is voided as load
  contamination, not a code regression: the same query on the same
  workdir index times 17.8/18.5 ms (n=12, tight) on the fixed binary and
  10–20 ms on a HEAD-built control binary (5/5 fast after a cold-cache
  first run). The bake-off session ran under another agent's build storm
  (the same storm that voided a literal series); no flamegraph is owed
  for a hotspot that does not reproduce. Same-corpus control for the
  literal fix: HEAD binary 170–180 ms (CPU-bound) vs fixed 7.6 ms.

## 2026-09-19 warm distinct-query latency (self corpus)

**Status: `reproducible-in-tree`.**
Reproduce: `node benchmarks/warm_distinct.mjs <asgrep-binary> 3 [--no-embed]`
-- 240 distinct identifiers sampled from `git ls-files crates`, driven through
`asgrep codemode-serve` NDJSON (the warm path the Pi package uses: one process,
warm Searcher, no per-call spawn), interleaved A/B in one machine session
against the same index. Warm-up calls are excluded.

| Provenance | value |
|------------|-------|
| date | 2026-09-19 |
| commit | `c6f45d92`; base binary = `9bfa6aa0` release build |
| machine | Apple M5 Max (arm64), macOS 26.6.2, APFS SSD |
| build | `cargo build --release -p ast-sgrep-cli` (both binaries) |
| index | schema 16, **637 files**, 197,487 lines, 6,211 symbols, 11,940 semantic chunks, IVF present |
| battery | 240 distinct identifiers (`fn|struct|enum|trait` names, >=7 chars) |
| rows | median of 5 (no-embed) / 3 (embed) interleaved rounds, each n=240 |

| Variant | p10 | p50 | p90 | p99 | mean | max |
|---------|----:|----:|----:|----:|-----:|----:|
| no-embed base | 1.49 | 2.82 | 4.86 | 7.02 | 2.93 | 10.0 |
| no-embed **candidate** | **1.26** | **1.95** | **2.83** | **5.12** | **2.02** | 9.3 |
| embed base | 1.79 | 3.28 | 5.51 | 8.72 | 3.42 | 11.0 |
| embed **candidate** | **1.47** | **2.21** | **3.14** | **5.62** | **2.32** | 10.4 |

Deltas: **p50 -31% / p90 -42%** (no-embed), **p50 -33% / p90 -43%** (embed).
Every interleaved round separated (base p50 2.71-3.04 vs candidate 1.93-2.05).

What changed (structural stage, all inside the hybrid search):

- `pattern_nodes_matching_for_files` returned **every** matching node inside the
  100-file cascade (374 rows / 1.17 ms measured for `hits`). It now takes the
  same row budget as the symbol/caller/anchor passes (`retained_limit().max(32).min(500)`
  = 32 at limit 8) with an explicit `ORDER BY n.signature, n.id`, so the kept
  prefix is the order today's plan already yields first.
- `structural_index_pass` fetched one indexed excerpt per pattern node before
  fusion (`fill_pattern_excerpt`, p50 244 us / p90 691 us of the search). Known
  survivors get the identical string from the lazy attach in finish
  (`attach_indexed_excerpts_if_empty`), which is the only caller path.
- `WarmedSymbolTable::matches_for_files` cloned every substring match before
  sorting and truncating; it now selects indices and materializes only the rows
  that survive the budget (same order, same sort keys, same quotas).

Quality parity (same session, same index): `asgrep eval --gold
benchmarks/gold/self.json` unchanged -- MRR **0.798**, nDCG **0.847**, recall@1
**0.647**, recall@5 **0.882**, recall@20 **1.0** (17 queries). CLI golden lanes
byte-identical: `machine_contracts` 35/35, `cli_smoke` 18/18; core
`search_correctness_epics` 12/12.

Also fixed on the way: `search_correctness_epics::lang_aliases_match_indexed_source_extensions`
was red on `HEAD` (its snippet table did not know `cu`/`dart`/`mbt`, added after
those languages joined `SOURCE_EXTENSIONS`).

## 2026-09-20 (self corpus, HEAD `3e05d604`, competitor bake-off)

**Status: `reproducible-in-tree`.** CLI process times via `hyperfine` on a
copy of `git ls-files`, same protocol as 2026-08-28 plus `grep`, `tgrep`,
and a semgrep reference set. Hit counts were verified non-empty for every
timed query (16/16 on the capped retrieval cells). In-process
`asgrep bench` times are a different surface and are not mixed in.

| Provenance | value |
|------------|-------|
| date | 2026-09-20 |
| commit | `3e05d604` |
| machine | Apple M5 Max, 18 cores (arm64), macOS 26.6.2, APFS SSD |
| corpus | tracked files → rsync workdir: **698 files** (index saw **649** after skip rules; includes a 10 MiB `.tgrep/` sidecar built mid-run — all tools scanned the same bytes) |
| build | `cargo build --profile release-perf -p ast-sgrep-cli --bin asgrep` |
| rustc | 1.98.0 |
| tools | ripgrep 15.1.0, ast-grep 0.45.3, semgrep 1.176.0, tgrep 1.0.9 (warm server, `--no-watch`), BSD grep 2.6.0, hyperfine 1.20.0 |
| index | schema 16, hashed semantic embedder; 6,294 symbols, 12,138 semantic chunks, 63,094 callers; `index.db` **231 MiB** (+107 MiB vectors: `--no-embed` build is 133 MiB / 7.9 s) |

p95 is nearest-rank on hyperfine's raw samples: `idx = floor((n - 1) * 95 / 100)`.

| Surface | n | p50 | p95 | comparator p95 | note |
|---------|--:|----:|----:|-------------:|------|
| cold index (`asgrep index .`) | 8 | 17.0 s | **17.4 s** | tgrep ~0.1 s / 10 MiB (single sample) | different index content (AST/graph/vectors vs trigrams) |
| warm `literal:SearchHit` | 15 | 156 ms | **171 ms** | tgrep 6.1 ms, rg 13.8 ms, grep 107 ms | indexed retrieval slower than scan here; see note |
| warm `pattern:SearchHit` | 12 | 9.6 ms | **14.6 ms** | ast-grep 60.1 ms | asgrep wins; narrow structural path |
| warm `semantic 'credential renewal'` | 12 | 2.25 s | **2.34 s** | — | NL semantic one-shot is seconds-scale; needs a flamegraph |
| serve distinct-query p50/p99 | 240 | 2.4 ms | (p99) 5.4 ms | — | `warm_distinct.mjs`, 2 rounds; the Pi product path is healthy |
| semgrep 2-rule reference | 3 | ~10 s | — | — | `fn $F / struct $S` rules, 2,729 results; deep-analysis class, reference only |

Notes, read before quoting:

- **Literal vs August (19.0 ms → 171 ms) is not yet a controlled
  regression.** Corpus differs (398 → 649 indexed files, schema 14 → 16,
  embedder changes). The in-run comparison stands on its own: on this
  corpus, one-shot indexed literal costs ~150 ms of searcher CPU
  (`status` opens the same index in 9 ms; `--excerpt-lines 0` and
  `--limit 1` do not move it; `--no-embed` index does not move it).
  Prime suspect is per-invocation fusion/cascade work added since August;
  the serve path (same engine, warm Searcher) answers the same class in
  ~2.5 ms, so this is one-shot-invocation overhead, not engine speed.
- **Pattern flipped the other way** (129 ms → 14.6 ms vs ast-grep 60 ms):
  the narrow structural path won while the fusion path lost. Both
  directions need the same flamegraph before any fix claim.
- **Two earlier series were discarded, not averaged in.** (1) A literal
  series ran during another agent's build storm (asgrep 199 ms, tight
  σ — sustained load, verified by rerun at 4.7 ms on a then-unnoticed
  fixture index, which voided it differently). (2) `asgrep bench
  tests/fixtures/sample --index-path $INDEX` rewrites the target db to
  the 8-file fixture, silently voiding every later cell until noticed
  via a 45-row `symbols` table. Methodology rule adopted: verify
  hit counts and `files`/`symbols` table counts alongside every timing
  block, and never point `bench` at a measurement db.
- tgrep was measured as designed (warm `serve`, client query); its 0.1 s
  index covers trigram postings only. semgrep's seconds include Python
  startup + full analysis; it answers a different (deeper) question.

### Reproduce

```bash
cargo build --profile release-perf -p ast-sgrep-cli --bin asgrep
WORKDIR=/tmp/asgrep-speed-corpus
INDEX=/tmp/asgrep-speed.db
rm -rf "$WORKDIR" && mkdir -p "$WORKDIR"
git ls-files -z | rsync -a --files-from=- --from0 . "$WORKDIR"
cd "$WORKDIR"

hyperfine --warmup 1 --runs 8 \
  --prepare "rm -f $INDEX $INDEX-wal $INDEX-shm" \
  --export-json /tmp/asgrep-cold-index.json \
  "$OLDPWD/target/release-perf/asgrep --json --index-path $INDEX index ."

"$OLDPWD/target/release-perf/asgrep" --json --index-path "$INDEX" index .
ASG="$OLDPWD/target/release-perf/asgrep --no-auto-index --index-path $INDEX"
hyperfine --warmup 3 --runs 15 --export-json /tmp/asgrep-literal.json \
  "$ASG 'literal:SearchHit' ." "rg -n SearchHit ." "grep -rn SearchHit . --exclude-dir=.git" "tgrep search SearchHit ."
hyperfine --warmup 3 --runs 12 --export-json /tmp/asgrep-pattern.json \
  "$ASG 'pattern:SearchHit' ." "ast-grep --lang rust --pattern SearchHit ."
hyperfine --warmup 3 --runs 12 --export-json /tmp/asgrep-semantic.json \
  "$ASG semantic 'credential renewal' ."
node benchmarks/warm_distinct.mjs ./target/release-perf/asgrep 2
```

`tgrep search` requires a warm server on the corpus (`tgrep index .`,
then `tgrep serve --no-watch .` in the background). Verify hits and
`select count(*) from files, symbols` after every block (see note).

## 2026-08-28 (self corpus, HEAD `2285ce29`)

**Status: `reproducible-in-tree`.** CLI process times via `hyperfine` on a
copy of `git ls-files`. In-process `asgrep bench` times are a different
surface (Searcher only, no process start) and are not mixed into this table.

| Provenance | value |
|------------|-------|
| date | 2026-08-28 |
| commit | `2285ce29` |
| machine | Apple M5 Max, 18 cores (arm64), 48 GiB, macOS 26.5, APFS SSD |
| corpus | tracked files → rsync workdir: **445 files**, 4.6 MiB source (index saw **398** files after skip rules) |
| build | `cargo build --profile release-perf -p ast-sgrep-cli` |
| rustc | 1.98.0 |
| tools | ripgrep 15.1.0, ast-grep 0.45.2, hyperfine 1.20.0 |
| index | schema 14, hashed semantic-v2 dim 256; IVF sidecar not built at this size; 3,461 symbols, 6,302 chunks; `index.db` **104 MiB** |

p95 is nearest-rank on hyperfine's raw samples: `idx = floor((n - 1) * 95 / 100)`.

| Surface | n | p50 | p95 | comparator p95 | note |
|---------|--:|----:|----:|-------------:|------|
| cold index (`asgrep index .`) | 8 | 4.53 s | **4.58 s** | — | hashed semantic-v2; IVF sidecar not built |
| warm `literal:SearchHit` | 15 | 18.7 ms | **19.0 ms** | rg 11.1 ms | ripgrep wins on this small tree |
| warm `pattern:SearchHit` | 12 | 118 ms | **129 ms** | ast-grep 26.5 ms | ast-grep wins; latency-only, not match-set |
| warm `semantic 'credential renewal'` | 12 | 19.1 ms | **20.3 ms** | — | indexed semantic channel |
| warm NL `how does hybrid search work` | 12 | 18.5 ms | **18.9 ms** | — | unprefixed hybrid |

This tree is smaller than the 2026-08-05 1,107-file snapshot (campaign docs
and fuzz/conformance trees are gone). Do not treat the two dates as a
same-corpus speedup or regression.

### Reproduce

```bash
cargo build --profile release-perf -p ast-sgrep-cli
WORKDIR=/tmp/asgrep-speed-corpus
INDEX=/tmp/asgrep-speed.db
rm -rf "$WORKDIR" && mkdir -p "$WORKDIR"
git ls-files -z | rsync -a --files-from=- --from0 . "$WORKDIR"
cd "$WORKDIR"

hyperfine --warmup 1 --runs 8 \
  --prepare "rm -f $INDEX $INDEX-wal $INDEX-shm" \
  --export-json /tmp/asgrep-cold-index.json \
  "$OLDPWD/target/release-perf/asgrep --json --index-path $INDEX index ."

"$OLDPWD/target/release-perf/asgrep" --json --index-path "$INDEX" index .
ASG="$OLDPWD/target/release-perf/asgrep --no-auto-index --index-path $INDEX"
hyperfine --warmup 3 --runs 15 --export-json /tmp/asgrep-literal.json \
  "$ASG 'literal:SearchHit' ." "rg -n SearchHit ."
hyperfine --warmup 3 --runs 12 --export-json /tmp/asgrep-pattern.json \
  "$ASG 'pattern:SearchHit' ." "ast-grep --lang rust --pattern SearchHit ."
hyperfine --warmup 3 --runs 12 --export-json /tmp/asgrep-semantic.json \
  "$ASG semantic 'credential renewal' ."
```

The in-tree identity suites (`asgrep bench . --suite self` and
`asgrep bench tests/fixtures/sample --suite default`) check hit identity.
They are not this CLI-vs-scan table. High first-query CV is expected there
because later iterations are a warm Searcher.

## 2026-08-05 archive (1,107-file 1.4.0 tree)

**Status: `historical`.** Same machine class. Corpus and binary differ from
the 2026-08-28 row. Kept so the old README numbers have a home.

| Surface | release/1.4.0 p95 | comparator p95 |
|---------|------------------:|-------------:|
| cold index | 2.26 s | — |
| warm literal | 19.5 ms | rg 15.7 ms |
| warm semantic NL | 19.6 ms | — |
| structural pattern (quality path) | 33.1 ms | ast-grep 24.2 ms |
| structural pattern (pre-fix path) | 987 ms | ast-grep 26.3 ms |
| index size | 27 MiB | — |

The “31× structural” claim was quality-path vs pre-fix path on that tree,
not asgrep vs ast-grep. Do not quote it as a current competitor win.

## Older generated dumps

**Status: `historical` + `UNREPRODUCIBLE`.** 23k/100k GATE aggregates and
foreign-corpus `speed-report.py` tables are summarized in
[`head-to-head.md`](head-to-head.md). The generating scripts are not in
this tree. Do not regenerate those rows from this checkout.

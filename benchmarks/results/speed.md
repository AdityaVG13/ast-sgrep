# Speed benchmarks

Companion to [`baselines.md`](baselines.md) (retrieval quality). This file
owns **wall-clock CLI latency**. Any speed number quoted elsewhere must
trace back to a dated row here or carry its own reproduce command.

Status tags: [`benchmarks/README.md`](../README.md).

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

# Speed benchmarks

Companion to [`baselines.md`](baselines.md) (retrieval quality). This file
owns **wall-clock CLI latency**. Any speed number quoted elsewhere must
trace back to a dated row here or carry its own reproduce command.

Status tags: [`benchmarks/README.md`](../README.md).

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

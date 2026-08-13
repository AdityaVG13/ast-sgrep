# Residual leaf shares post-T1-R (hoy3.1)

MEASURE only. No product source change. Do not paste pre-T1 Amdahl S into this row.

## Provenance

| Field | Value |
|---|---|
| Run id | `20260813T212430Z` |
| Git SHA | `8038346` (`feat/golden-assert-testkit`) |
| Binary | `target/release-perf/asgrep` (Mach-O arm64) |
| Profile | `release-perf` + `RUSTFLAGS=-C force-frame-pointers=yes` |
| Host | Darwin arm64, macOS 26.5 (`samply` meta.oscpu) |
| Isolation | local Darwin (samply cannot attach to the Linux RCH artifact) |
| Corpus root | `/Users/aditya/AI/ast-sgrep-wt-nz7i` |
| Files indexed | **403** (55 skipped) |
| Semantic chunks | **5564** |
| ANN / IVF | **on** (`semantic_ivf_present: true`, threshold 2000) |
| Wall | **4.22 s** real / 4.98 s user (`/usr/bin/time -l`) |
| RSS peak | 216 MiB |
| Raw profile | `tests/artifacts/perf/20260813T212430Z/samply.json` + `samply.syms.json` (gitignored) |

This is **not** the historical C4 residual mean 1.934 s (different SHA, file count, and host run). Do not overwrite C4.

## Method

- `samply record --unstable-presymbolicate --save-only` at 1000 Hz on a **cold** `--index-path` DB.
- **Exclusive** innermost-frame self-time, weighted by `threadCPUDelta` (µs). Inclusive IVF would double-count kmeans callers; exclusive is the reopen metric.
- Leaf classifier (first match): tree-sitter/`ts_*`/`ast_sgrep_lang` → extract_embed; `semantic_ann`/`kmeans`/`simsimd`/`build_from_flat` → ivf_build; `blake3`/`compress_xof`/`hash_content` → blake3_hash; `sqlite3*`/`rusqlite`/`upsert_file`/`IndexStore` → sqlite_upsert; else other.

## Share table (exclusive CPU)

| Leaf | Share | reopen_gate (≥5% **and** T3/UPSERT-class) | Notes |
|---|---:|---|---|
| extract_embed | **48.68%** | **false** | tree-sitter walk (`ts_node_child_iterator_next` 20.6% of all exclusive). Not T3/UPSERT. |
| other | **24.62%** | **false** | Mix / unresolved RVAs / CLI glue. Not a named lever. |
| blake3_hash | **9.77%** | **false** | `compress_xof` 9.0%. C20: do **not** drop content hash. |
| sqlite_upsert | **9.37%** | **true** | `sqlite3Fts5HashWrite` + `sqlite3VdbeExec` + `IndexStore` drop. Human review before any UPSERT product bead. Score≥2 still required. |
| ivf_build | **7.56%** | **true** | Almost all `simsimd_dot_f32_neon` (7.10%). `build_from_flat` exclusive is **0.21%**. Human review before T3. |

Checksum 100.00% (method error band ±5% on classification of `other` / unresolved).

## Top exclusive frames (informational)

| Share of all exclusive | Frame |
|---:|---|
| 20.61% | `ts_node_child_iterator_next` |
| 11.26% | `node_lines` |
| 8.98% | `compress_xof` (blake3) |
| 8.79% | `ts_node_child_with_descendant` |
| 7.10% | `simsimd_dot_f32_neon` |

## C6 / C12 note (claim-table upgrade path)

- **C6** pre-T1 `build_from_flat` ~34–35% is still **stale [E]** for exclusive `build_from_flat` (0.21% here). IVF residual that remains is **simsimd kmeans dots** (7.56% class), not the old build_from_flat leaf.
- **C12** residual-as-mix still holds for the IVF/upsert/blake3 trio (none is a majority of wall). Extract/parse is a majority of **this** cold-index exclusive CPU; that is parse, not an IVF T3 lever.
- Active T3/UPSERT product queue is **not** empty by the 5% rule (ivf_build and sqlite_upsert). Do not open product beads from this packet without a Score≥2 opportunity matrix and human review.

## Reproduce

```bash
export RUSTFLAGS="-C force-frame-pointers=yes"
cargo build --profile release-perf -p ast-sgrep-cli
rm -f /tmp/asgrep-hoy3-s2-cold.db /tmp/asgrep-hoy3-s2-cold.db-wal /tmp/asgrep-hoy3-s2-cold.db-shm
samply record --unstable-presymbolicate --save-only -o samply.json -- \
  ./target/release-perf/asgrep --json --index-path /tmp/asgrep-hoy3-s2-cold.db index .
```

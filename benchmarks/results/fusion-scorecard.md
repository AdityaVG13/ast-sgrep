# Fusion / stage budget scorecard (e2hc.31)

Completion-debt for closed `ast-sgrep-e2hc.5`. **Does not invent latencies.**
Arena fusion product work is `e2hc.30` (independent).

## Surfaces (do not mix)

| Surface | Budget intent | Competitor comparison | Status |
|---|---|---|---|
| A. In-process pipeline parts | median **< 1.0 ms** per named part | none (library internals) | Gate: `tests/core/sub1ms.rs` (release only) |
| B. CLI fixture keep-gate | **15 ms** average smoke ceiling | not keep | `reproducible-in-tree` ([README](../README.md)) |
| C. CLI self-corpus vs tools | no sub-1ms claim | ripgrep / ast-grep / semgrep | [speed.md](speed.md) / [head-to-head.md](head-to-head.md) |
| D. Historical 23k/100k GATE | multi-ms aggregates | same tools | `historical` + `UNREPRODUCIBLE` |

Sub-1ms is **A**, not C. Publishing C rows as "sub-1ms vs competitors" is a miss.

## A. Stage budget (in-process)

Parts from `ast_sgrep_core::pipeline_parts::CORE_PARTS`, warm
`tests/fixtures/sample`, local embed, no network:

| Stage | What is timed |
|---|---|
| `query_parse_intent` | parse + intent classify |
| `literal_retrieval` | literal pass |
| `lexical_fts` | FTS/trigram lexical pass |
| `symbol_graph` | defs/callers/anchor graph pass |
| `hybrid_fusion_rank` | hybrid retrieve + `fusion::apply_weighted_rrf` |
| `semantic_embed` | embed pass |
| `result_format` | hit line format |
| `index_update_one_file` | incremental one-file upsert |

Statistic in the harness: per-part **median / mean / min / max / p95** ms
(`PartTiming`). The **assert** is median `< 1.0` (`BUDGET_MS`). Debug builds
skip the budget assert. p99 is **not** computed (no equivalent in the report).

**No host medians are copied here.** Copying a laptop run would mint an undated
canonical row. Reproduce:

```bash
cargo test -p ast-sgrep-core --test sub1ms --release -- --nocapture
# optional: ASGREP_PARTS_OUT=/tmp/sub1ms_report.json
```

Per-stage walls vs ripgrep/ast-grep/semgrep: **`UNREPRODUCIBLE`** (no harness
splits competitor process time onto these eight names).

`ASGREP_PERF_PROFILE` search emits a single `search_query` span, not the eight
parts. Index stages are a different packet ([stage-timers-post-T1R.md](../../docs/validation/stage-timers-post-T1R.md)).

## C. Fused CLI vs pinned competitors (not sub-1ms)

Cite [speed.md](speed.md) 2026-08-05 `reproducible-in-tree` (self, 1,107 files,
release/1.4.0 p95). Quality fingerprints stay in [baselines.md](baselines.md).

| Query class | asgrep | Competitor | Who wins |
|---|---:|---:|---|
| Warm lexical | 19.5 ms p95 | ripgrep **15.7 ms** p95 | **ripgrep** (parity band, 1.24×) |
| Structural pattern | 33.1 ms p95 | ast-grep **24.2 ms** p95 | **ast-grep** (0.73×) |
| Cold index | 2,257 ms p95 | (no scan-tool analogue) | n/a |

Historical 23k/100k "asgrep faster" aggregates in [head-to-head.md](head-to-head.md)
remain `UNREPRODUCIBLE` (dumps not in-tree). Structural `parity_clean` there is
**latency-only**, not match-set (`DISC-pattern-native-subset`).

## Honest negatives

| Loss | Provenance |
|---|---|
| ripgrep faster on small self lexical | speed.md 2026-08-05 |
| ast-grep faster on small self `pattern:` | speed.md 2026-08-05 |
| Older 82–917-file corpora: rg wins 2/3 lexical | speed.md (historical) |
| `rg_std_printer`, `rg_json_output`, `rg_overrides` rank losses vs semgrep hand-patterns | [losses.md](losses.md); MRR config `rg-neural-rerank-d3eab74` **0.605** vs default hybrid **0.290** (`rg-hybrid-default-d3eab74`) |
| Shared miss `rg_search_core` | losses.md |
| Sample IVF-off (~tens of ms class) is not self ANN-on | C18; not a fusion score |

## What e2hc.5 still does not claim

- Fused CLI hybrid under 1 ms vs competitors (C is multi-ms).
- p50/p99 per competitor stage (no harness).
- New MRR/latency numbers in this packet.

Historical `ast-sgrep-e2hc.5` stays closed. This file is the honesty scorecard.

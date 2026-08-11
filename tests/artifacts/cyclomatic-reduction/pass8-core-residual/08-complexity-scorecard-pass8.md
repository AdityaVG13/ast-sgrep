# 08 — Complexity scorecard (partial: Pass 8 core residual)

Run: `2026-08-10T235424Z-baseline`  
Wave only — not campaign complete. Analyzer: lizard via `measure_complexity.py`.

## Touched-scope bill

| Scope | ΣCC before | ΣCC after | Δ | Functions before → after |
|---|---:|---:|---:|---|
| `crates/ast-sgrep-core/src/search/passes/literal.rs` | 37 | **37** | **0** | 5 → 6 |
| `crates/ast-sgrep-core/src/semantic_ivf.rs` | 137 | **137** | **0** | 39 → 40 |
| **Combined touched files** | **174** | **174** | **0** | 44 → 46 |

Displacement check: **pass** — ΣCC flat; shared collapse in literal funds new helper base; IVF write extract is linear displacement (parent −8 / helper +8).

`crates/ast-sgrep-core` package remeasure: total_cc **2886 → 2886** (hotspots >10: 42 → 42; max still `read_header` 25).

## Per-function CC (wave targets)

| Function | File | Before | After | Δ |
|---|---|---:|---:|---:|
| `literal_sql` | `literal.rs` | 16 | **15** | −1 |
| `literal_trigram` | `literal.rs` | 12 | **11** | −1 |
| `content_matches_literal` (new) | same | — | 2 | +2 |
| `save_semantic_ivf_with_publication` | `semantic_ivf.rs` | 25 | **17** | −8 |
| `write_ivf_temporary` (new) | same | — | 8 | +8 |

## Metric-gaming auditor (self)

- No public API split for score.
- No helper fan-out that raised ΣCC (those attempts **refused**).
- Essential IVF format / ranking / URL allowlist **not** scattered.
- RESULT: **METRIC_GAMING_RESULT: pass**

## Ceiling status (wave targets)

| Function | After CC | vs hard 10 |
|---|---:|---|
| `literal_sql` | 15 | residual |
| `literal_trigram` | 11 | residual |
| `save_semantic_ivf_with_publication` | 17 | residual (validation Keep) |
| `write_ivf_temporary` | 8 | under preferred |
| `content_matches_literal` | 2 | under |

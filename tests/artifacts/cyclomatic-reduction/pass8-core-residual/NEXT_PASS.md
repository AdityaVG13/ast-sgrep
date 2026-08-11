# NEXT_PASS.md

Run ID: `2026-08-10T235424Z-baseline`
Completed: **Pass 8 — Module residual (core crates)**
Next: **Pass 9 — Surface crates residual** (cli / mcp / lang) **or** extension residual if reordered

## Pass 8 outcome (this session)

| Function | CC before → after | Technique | Helpers |
|---|---|---|---|
| `literal_sql` | 16 → **15** | extract_method (shared collapse) | `content_matches_literal` (2) |
| `literal_trigram` | 12 → **11** | same shared helper | (shared) |
| `save_semantic_ivf_with_publication` | 25 → **17** | extract_method | `write_ivf_temporary` (8) |

Touched-file ΣCC: **174 → 174 (0)**. Core package total_cc **2886 → 2886**.

Refused pure extracts that raised ΣCC: `apply_watch_path_update`, walk accept helpers, regex multi-helper fan-out / `join_regex_workers`.

Parity: targeted `ast-sgrep-core` tests green (parity, semantic_ivf_roundtrip, search_correctness_epics, e2e_smoke, regex_budget, code_prose_fields). Artifacts: `06-transformed-code/pass8-*`, `07-parity-report-pass8.md`, `08-complexity-scorecard-pass8.md`. Mirror: `tests/artifacts/cyclomatic-reduction/pass8-core-residual/`.

## Explicit Keep / Refuse notes

- `read_header`, `read_clusters_bounded`, `apply_weighted_rrf`, `embed_pass_lazy_ivf`, `refresh_lines_only`, `embed_url_is_allowed` — **Ashby Keep** (format / ranking / security).
- IVF publication **validation** residual on `save_semantic_ivf_with_publication` (17) — Keep domain gates.
- Pure extract without ΣCC funding — **Refuse** (pass 7/8 precedent).

## Residual checks (core) after pass 8 — named ≥3

| Function | CC | Resolve | Why no productive cut this wave |
|---|---:|---|---|
| `update_paths` | 15 | Defer | `apply_watch_path_update` pure extract bill +2; index/remove arms essential |
| `index_content_at` | 13 | Defer/Keep residual | Pass 4 already extracted structure-skip; spine is ordering contract |
| `collect_index_candidates` | 13 | Defer | Walk accept extract raised ΣCC +3..+5 |
| `literal_sql` | 15 | Defer residual | Word/lang/query_map residual after shared collapse |
| `regex_pass` | 18 | Defer | Multi-helper extract +1..+4 without collapse |
| `read_header` | 25 | Keep | Essential IVF format parser |
| `refresh_lines_only` | 19 | Keep | Index corruption risk (prior card) |
| `verify_candidate_generation` | 13 | Keep-leaning | Activation safety gates (jpbq) |

## Do next (Pass 9)

1. Load via `.cyclomatic-reduction/LATEST` → `2026-08-10T235424Z-baseline`.
2. **Do not re-baseline** unless scope changes.
3. **Pass 9 surface crates:** `ast-sgrep-cli`, `ast-sgrep-mcp`, `ast-sgrep-lang` residual hotspots (classify ranks 31–91 as needed); only bill-neutral/negative cuts.
4. Optional funded core returns: only if a **shared collapse** or dead-branch proof can fund `update_paths` / `regex_pass` extracts.
5. Extension residual (`parseEnvelope` 17 Keep, `summarizeCodemode`, launcher resolve\*) remains a separate surface wave if not yet scheduled.

## Mode reminder

Campaign multipass repo-sweep. Prefer real ΣCC cuts / shared collapses over vanity extracts.

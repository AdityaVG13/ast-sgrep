# Work packet D3 — Core search / store residual

| Field | Value |
|---|---|
| id | D3 |
| priority | P2 |
| status | open / **Defer** |
| risk | **high** (search correctness / IVF / store) |
| product_area | `crates/ast-sgrep-core` |
| nearby_Keep | read_header 25, read_clusters_bounded 22, apply_weighted_rrf 21 — **do not touch for score** |

## Goal

Bill-negative shared collapses in search passes and store paths. Never dump walk/regex complexity into base-cost helpers.

## Exact targets (high residual, non-Keep-first)

| Function | CC | File | Label |
|---|---:|---|---|
| `embed_pass_lazy_ivf` | 20 | `search/passes/embed.rs` | Defer |
| `refresh_lines_only` | 19 | `store/sqlite.rs` | Defer |
| `regex_pass` | 18 | `search/passes/regex.rs` | Pass-8 extract **Refuse** |
| `measure_index_update` | 18 | `pipeline_parts.rs` | Harness — low product value |
| `load_semantic_context` | 16 | `search/passes/embed.rs` | Defer |
| `update_paths` | 15 | `index.rs` | Pass-8 extract Refuse; pass-3 guards already applied |
| `literal_sql` | 15 | `literal.rs` | word_mode residual after pass-5 lookup |
| `literal_trigram` | 11 | `literal.rs` | Defer thin |
| sqlite upsert/init cluster | 14–12 | `store/sqlite.rs` | Defer carefully |

Paths relative to `crates/ast-sgrep-core/src/`.

## Essential Keep nearby (forbidden for score cuts)

| Function | CC | Why |
|---|---:|---|
| `read_header` | 25 | IVF format fidelity |
| `read_clusters_bounded` | 22 | ANN structure |
| `apply_weighted_rrf` | 21 | Ranking variety |
| IVF validation residual | 17 | Publication correctness |

## History

| Pass | Action | Bill |
|---|---|---|
| 3 | `update_paths` guards | −3 class on touched |
| 4 | `index_all`, `delete_file_lines`, … | touched −4 |
| 5 | `literal_sql` lookup table | large touched − |
| 8 | `content_matches_literal` shared collapse (bill-neutral file); IVF `write_ivf_temporary` linear displacement | pure walk/regex/update_paths extracts **refused** (+Σ class) |
| 10 | re-measure | core remains hotspot-dense (42 of 83) |

## Classification

| Target | Class | Notes |
|---|---|---|
| regex_pass | accidental residual + essential budget | Multi-helper fan-out raised ΣCC — Refuse pattern known |
| embed_pass_lazy_ivf | extractable gates | Only shared collapse with load_semantic_context if duplicate |
| refresh_lines_only | store I/O accidental + SQL domain | High regression risk |
| literal_sql residual | accidental word_mode | Table only if pure map |

## Allowed techniques

1. Duplicate-gate **shared collapse** across embed helpers (`embed_pass_lazy_ivf` ↔ `load_semantic_context`).
2. Literal residual only if word_mode decision can be **tabled** without behavior change.
3. Store path only if identical transaction gates collapse **and** store tests green.

## Forbidden

- Multi-helper regex fan-out (+4 class refuse in pass 8).
- `accept_walk_file` style extracts that raise ΣCC.
- Changing IVF on-disk format branches for "simpler" parsers.
- Touching Keep ranking/ANN/header functions for score.
- Public index/search API signature changes.

## Procedure

1. Pre-measure touched files:
   ```bash
   python …/measure_complexity.py \
     crates/ast-sgrep-core/src/search \
     crates/ast-sgrep-core/src/store \
     crates/ast-sgrep-core/src/literal.rs \
     --threshold 10
   ```
   Record `total_cc`.
2. Hunt **duplicate** predicates only; if none → Keep residual / stop.
3. One shared collapse; re-measure; accept only `total_cc` ≤ pre-edit.
4. Prefer smallest surface (embed helpers) over sqlite first.

## Verify (acceptance)

```bash
cargo check -p ast-sgrep-core

cargo test -p ast-sgrep-core \
  --test parity --test e2e_smoke --test regex_budget \
  --test semantic_ivf_roundtrip --test search_correctness_epics \
  --test code_prose_fields
# expect all green (same as pass 11 matrix rows 9–10)
```

If touching literal:

- Must keep `iva9_5_literal_lang_filter_not_starved_by_path_limit` green.

If touching IVF/store:

- semantic_ivf_roundtrip + e2e `index_all_preserves_semantic_ivf…` green.
- No panic on corrupt frames (mapped_reader tests).

## Resolve default

Prefer **Keep** on format/ranking/store transaction shape.  
Only **Cut** when measure proves −ΣCC and suite above is green.

## Stop / escalate

Any ΣCC increase → immediate revert.  
Search ranking or IVF format change needs human auth — out of skill default.

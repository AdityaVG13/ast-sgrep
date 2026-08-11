# 06 — Transformed code (Pass 8: core residual)

Run: `2026-08-10T235424Z-baseline`  
Wave: Pass 8 module residual — core crates (`ast-sgrep-core` focus)  
Technique: **extract_method** with shared decision collapse (not vanity fan-out)

## Files changed

| Path | Change |
|---|---|
| `crates/ast-sgrep-core/src/search/passes/literal.rs` | Extract shared `content_matches_literal` used by `literal_sql` + `literal_trigram` |
| `crates/ast-sgrep-core/src/semantic_ivf.rs` | Extract `write_ivf_temporary` from `save_semantic_ivf_with_publication` (pass-4 leftover) |

Public signatures unchanged.

## Per-function transforms

### `literal_sql` / `literal_trigram` — extract_method (shared collapse)

- **Before:** duplicated `if let Some(needle_lower) { … } else { … }` case-fold gate in both loops.
- **After:** single `content_matches_literal(content, needle, needle_lower, word_mode)`.
- SQL residual still owns: LIKE/GLOB pattern escape, lang-param `query_map`, word_mode postfilter gate, `matches_lang` (normalize_id vs SQL equality — not dead).
- Trigram residual still owns: FTS prepare, limit break, context map.

### `save_semantic_ivf_with_publication` — extract_method

- **Before:** validation + temp write + atomic replace + cleanup in one function (CC 25).
- **After:** parent keeps domain validation + cleanup-on-err; `write_ivf_temporary` owns create-new / header / index bytes / padding / vectors / fsync / replace.
- `read_header` left untouched (**Ashby Keep** — format parser).

## Refused / reverted this wave (ΣCC bill)

Tried pure extracts that raised touched-file ΣCC without decision elimination:

| Attempt | Parent effect | Touched ΣCC | Resolve |
|---|---|---|---|
| `apply_watch_path_update` from `update_paths` | 15→8 | +2 | **Refuse** this wave (base-cost dump) |
| walk `accept_walk_file` / `walk_dir_allowed` | collect 13→5 | +3..+5 | **Refuse** |
| multi-helper regex (`compile`/`candidates`/`context`/`join`) | 18→10 | +4 | **Refuse** |
| single `join_regex_workers` | 18→15 | +1 | **Refuse** |

Pass 7 precedent: pure extract without decision elimination rejected until Σ funded.

## New private helpers (not public API)

| Helper | File | Role |
|---|---|---|
| `content_matches_literal` | `literal.rs` | Shared case-fold + word/substring gate |
| `write_ivf_temporary` | `semantic_ivf.rs` | IVF temp write + atomic replace body |

## Measure JSON

- `pass8-literal-before.json` / `pass8-literal-after.json`
- `pass8-semantic-ivf-before.json` / `pass8-semantic-ivf-after.json`
- `pass8-core-crate-before.json` / `pass8-core-crate-after.json`

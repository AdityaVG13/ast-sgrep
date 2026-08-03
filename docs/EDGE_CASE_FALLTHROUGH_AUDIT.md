# Edge-case fall-through audit (ast-sgrep-tius)

Audit date: 2026-08-03  
Branch: `fix/fusion-normalization-e2hc-14`  
Scope: retrieval/query paths that can early-return empty for edge inputs across
defs / callers / imports / chain / literal / word / pattern / semantic.

## Method

Static search for `Ok(vec![])` / `Ok(Vec::new())` / empty short-circuits in
`crates/ast-sgrep-core/src/{search,pattern,chain,semantic_ann,query,rank}` plus
manual reproduction notes. Cross-checked against closed beads `powt`, `c2j5`,
`hsv7`, and iva9.7 fail-closed pattern policy.

## Candidate sites

| # | Site | Mode(s) | Edge case | Reproduction rationale | Disposition |
|---|------|---------|-----------|------------------------|-------------|
| 1 | `search/passes/literal.rs` `literal_pass` | literal / word / hybrid-quoted | empty `target` | `ParsedQuery` with `target: None` or `Some("")` returns `Ok([])` without error | **Expected** empty match-none for empty needle |
| 2 | `search/passes/regex.rs` | regex | empty pattern / invalid regex | empty target → `Ok([])`; invalid regex errors (not silent) | **OK** — invalid fails; empty is match-none |
| 3 | `search/passes/lexical.rs` | hybrid / lexical | `parsed.terms.is_empty()` | tokenize-only stopwords / empty input → `Ok([])` | **Expected**; empty query has no FTS terms |
| 4 | `search/passes/symbol.rs` `symbol_pass` / callers | defs/callers/hybrid | empty terms | early `Ok([])` when `parsed.terms.is_empty()` | **Expected** |
| 5 | `search/passes/embed.rs` | semantic / hybrid+embed | empty terms or `!use_embed` | returns `Ok(Some([]))` / `Ok([])` | **Expected** when embed disabled or no terms |
| 6 | `search/passes/embed.rs` lazy IVF | semantic | ANN under-filled | returns `None` to fall through to flat (iva9.6) | **Fixed earlier** — not silent empty |
| 7 | `pattern.rs` exotic `$` shapes | pattern | `ASGREP_DISABLE_AST_GREP=1` or missing binary | fail-closed `Err` (iva9.7) | **Fixed earlier** |
| 8 | `pattern.rs` classifiable native empty | pattern | no AST matches | authoritative match-none | **OK** |
| 9 | `chain.rs` `expand_chain` | chain | empty/whitespace query | returns empty `ChainResponse` | **Expected** |
| 10 | `chain.rs` `hit_symbol` | chain | hit without symbol/callee/caller/line symbol | previously invented `first_symbol_in_file` | **Fixed (ql1u)** — now returns `None` (skip seed) instead of inventing |
| 11 | `semantic_ann.rs` `score_members` vs flat | semantic | sims near `MIN_SIMILARITY` | IVF used `sim > MIN` while flat used `exceeds_threshold` → inconsistent empties/hits | **Fixed (firi)** |
| 12 | `query.rs` `mode_query` | literal/regex | mixed-case terms | lowercased terms → case-sensitive match miss → empty | **Fixed (eh5a)** |
| 13 | `query.rs` parse `literal:`/`regex:`/`word:` | those modes | `raw` stripped prefix | agent/bench identity skew (not empty, but silent mismatch) | **Fixed (54if)** |
| 14 | Hybrid quoted `"…"` | hybrid→Literal intent | never ran `literal_pass` | lexical/symbol may miss exact multi-word string → empty vs `literal:` | **Fixed (50hx)** |
| 15 | `rank.rs` / symbol scoring | defs/callers | single-char / ASCII normalize | single-char exact retained (e2hc.14); Unicode substring gated by char count | **OK** — documented; not silent empty for exact |
| 16 | `finish_response` file_filter | all | invalid glob | previously skipped filter (unfiltered); now `Err` (iva9.2) | **Fixed earlier** |
| 17 | Tantivy sidecar empty | lexical | auto-created empty DB | previously “ready” → empty success; now not ready + FTS fallback (hkdi/s7jw.2) | **Fixed earlier** |
| 18 | GLOB/LIKE metacharacters in literal | literal | needles with `*?[%` | previously wildcarded away matches (`c2j5`) | **Fixed earlier** |
| 19 | NaN/inf similarity | semantic | non-finite sims | `top_k_*` filters non-finite; `exceeds_threshold` rejects non-finite min/sim | **OK** |
| 20 | Imports resolve miss | imports/chain | unresolved module | empty resolve → no import hits/edges | **Expected** when module missing; language-aware resolve covered by `5wkz` |

## Silent-empty bugs fixed on this branch

1. **50hx** — Hybrid Literal (quoted) now runs `literal_pass` (serial + parallel).
2. **eh5a** — `mode_query` preserves case for literal/regex terms.
3. **firi** — IVF member scoring uses the same `Some(MIN_SIMILARITY)` / `exceeds_threshold` predicate as flat.
4. **ql1u** — Chain seeds no longer invent `first_symbol_in_file`.

## Remaining intentional empties (not bugs)

- Empty / whitespace-only queries across modes.
- Embed disabled or no semantic chunks indexed.
- Pattern match-none for classifiable shapes with zero AST hits.
- Unresolved imports when the module path does not exist on disk.

## Follow-ups (non-blocking)

- Optional: surface a structured `match_none` reason code in machine JSON for agent UX (distinct from errors).
- Optional: env-tunable `PARALLEL_PASS_FILE_THRESHOLD` / `TANTIVY_AUTO_THRESHOLD` for smaller differential fixtures.

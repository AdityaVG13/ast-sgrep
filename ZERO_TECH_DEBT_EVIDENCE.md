# Zero Tech Debt Evidence — `fix/fusion-normalization-e2hc-14`

Hard evidence for deep zero-tech-debt cleanup on the fusion/normalization tip.
Commands assume `PATH="/usr/local/cargo/bin:$PATH"` and cwd `/workspace/.worktrees/pr22`.
**Note:** `.beads/` was not modified (per task instruction). No new PR opened; this branch only.

---

## Intended end state

Search / ranking / fusion is one coherent flow:

| Concern | Surface |
|---------|---------|
| Shared ranking comparator | `cmp_coverage_score` / `cmp_ranked_hits` in `search/mod.rs` |
| Shared pattern signatures | `ast_sgrep_lang::{cached_pattern_signatures, structural_term_signatures, required_pattern_literal}` |
| No dead channel wrappers | deleted `Searcher::search_regex` / `search_word`; deleted unused `ast_sgrep_cli::run` |
| Readable `finish_response` / gates | single `emit_response`; gate order unchanged (`enforce_result_gates` = cap_per_file → truncate) |
| CLI not a god-file | `machine.rs` / `bench.rs` / `watch.rs` / `search_cmd.rs`; `lib.rs` ≈ 420L dispatch |

---

## Batch A — search ranking + dead wrappers

### Caller verification (rg) before deletes

| Symbol | Callers outside definition | Action |
|--------|----------------------------|--------|
| `Searcher::search_regex` | **zero** (modes via `search("regex:…")`) | deleted |
| `Searcher::search_word` | **zero** (modes via `search("word:…")`) | deleted |
| `ast_sgrep_cli::run` / `pub fn run()` | **zero** (entry is `main` → `run_process`) | deleted |
| `clear_semantic_ivf_session_cache` | only `mark_semantic_ivf_stale` | made private |
| `load_or_build_semantic_ivf` / `cached_semantic_ivf` | in-crate only | demoted to private |
| `DEFAULT_ANN_THRESHOLD` | in-crate only | `pub(crate)` |
| `last_identifier_chain` thin wrapper | only self | deleted; call `last_identifier_in_chain` |

### Extracts

| Helper | Location | Purpose |
|--------|----------|---------|
| `cmp_coverage_score` / `cmp_ranked_hits` | `search/mod.rs` | one sort key for pre-truncate + keyed ranking |
| `emit_response` | same | flatten count-only / normal response + ledger |
| `lock_response_cache` | same | poison-clear helper |
| `structural_term_signatures` | lang → `structural_index_pass` | byte-identical hybrid keys |
| `wait_child_deadline` | `pattern.rs` | shared timed `try_wait` for ast-grep |
| `clamp_channel_weight` | `intent.rs` | shared weight clamp |
| `reassign_stale_ivf_partition` | `semantic_ann.rs` | early-return flatten rebuild |
| `measure_hit_len` | `pipeline_parts.rs` | shared hit-count work unit |

### Behavior invariants

- Hybrid ranking / `finish_response` gate order unchanged (shared comparator preserves score→coverage→file→line).
- Structural index score still `SCORE_PATTERN * 0.35` (noik).
- Regex/word modes still work through `ParsedQuery::parse` prefixes on `Searcher::search`.

---

## Batch B — pattern classifier / signatures / kind constants

### End state

| Surface | Location |
|---------|----------|
| `classify_native` / `NativeKind` / `DECL_PATTERN_PREFIXES` | `crates/ast-sgrep-lang/src/pattern.rs` (exported) |
| `DECL_KIND_PREFIXES` / `declaration_prefix` | same |
| `cached_pattern_signatures` / `required_pattern_literal` / `structural_term_signatures` | `crates/ast-sgrep-lang/src/signature.rs` |
| `IDENT_KINDS` / `MEMBER_EXPR_KINDS` / `is_ident_kind` / `is_member_expr_kind` | `extract.rs` (`pub(crate)`) |
| Query tables | `pattern_queries.rs` via `queries_for` |
| Core pattern search | consumes lang `cached_pattern_signatures` |
| Hybrid `structural_index_pass` | consumes `structural_term_signatures` |

### Refactors pinned

- Flattened `classify_native` trailing-paren empty-ok branch to early return.
- Table-drove `function_queries` / `class_queries` via `FUNCTION_QUERY_TABLE` / `CLASS_QUERY_TABLE`.
- Unified identifier / member kind lists between `pattern.rs` and `extract.rs`.
- Demoted extract helpers to `pub(crate)` (no external crate callers).
- Removed unused `Default for Extractor`.
- Kept `needs_ast_grep_fallback` for this tip’s fail-closed layer-3 path.

### Signature byte-identity checks

- Lang unit tests in `signature::tests::*` pin `decl:` / `call-name:` / `kind:` formats and structural term keys.
- `structural_term_signatures_match_legacy_formats` pins the six hybrid keys.

---

## Batch C — CLI god-file split

### End state

| Surface | Location / change |
|---------|-------------------|
| Machine envelopes | `crates/ast-sgrep-cli/src/machine.rs` (success envelope **without** inventing `exit_code` — tip contract) |
| Bench suite/batch | `bench.rs` |
| Watch loop | `watch.rs` |
| Search/chain/format | `search_cmd.rs` (`resolve_output_format`) |
| `lib.rs` | clap + dispatch (~420 lines; was ~921) |
| Dead delete | unused `pub fn run()` |

### Behavior invariants

- Clap surface / machine envelopes unchanged.
- Chain still honors `--limit` for `top_n` (ql1u).

---

## Batch D — Pi runtime + release-acceptance density

### Decision counts (if+match+while+`=>` for Rust; if+else+&&+||+ternary-ish for JS)

| File | Before | After | Δ |
|------|--------|-------|---|
| `crates/ast-sgrep-lang/src/pattern.rs` | 113 | 63 | −50 |
| pattern + `pattern_queries` + `signature` | 113 | 97 | −16 (tables/signatures moved; control-flow thinner) |
| `packages/pi/scripts/release-acceptance.mjs` | 121 (dens 0.443) | 111 (dens 0.385) | −10 |
| `packages/pi/extension/src/runtime.ts` | 156 (dens 0.311) | 150 (dens 0.295) | −6 |
| `crates/ast-sgrep-core/src/search/mod.rs` | 62 / 1017L | 62 / 983L | helper extract; lines −34 |
| `crates/ast-sgrep-core/src/pattern.rs` | 39 | 30 | −9 |
| `crates/ast-sgrep-cli/src/lib.rs` | 115 / 921L | 55 / 420L | split out modules |
| `crates/ast-sgrep-core/src/semantic_ann.rs` | 41 | 40 | demote + extract |

### Refactors pinned

| Change | File |
|--------|------|
| `LEGACY_NUMBER_FIELDS` for migrate↔rollback; unified `assertVersionTriple`; `isContained`→`pathContained` | `runtime.ts` (+ regenerated `dist/runtime.js`) |
| `packageSpec` / `requiredFilesFor` / `expectReject` / `isForbiddenPackEntry` / `assertDirectoryEmpty` / `sameJson` / `COMMANDS` | `release-acceptance.mjs` |
| Fail codes / rejection labels | **unchanged** |

---

## Commands run

```bash
export PATH="/usr/local/cargo/bin:$PATH" CARGO_BUILD_RUSTC_WRAPPER= RUSTC_WRAPPER=

cargo test -p ast-sgrep-lang --lib --test pattern
# → lib: 6 passed; pattern: 5 passed

cargo test -p ast-sgrep-core --lib
# → 32 passed

cargo test -p ast-sgrep-core --lib search::
# → 8 passed

cargo test -p ast-sgrep-core --test downstream_correctness --test search_correctness_epics
# → downstream_correctness: 6 passed
# → search_correctness_epics: 10 passed

cargo test -p ast-sgrep-cli --test machine_contracts
# → 6 passed

cargo test -p ast-sgrep-cli --test surface_equivalence
# → 2 passed

cargo clippy -p ast-sgrep-core -p ast-sgrep-cli -p ast-sgrep-lang --all-targets -- -D warnings
# → ok

node packages/pi/scripts/release-acceptance.mjs self-test
# → gate self-test accepted; rejection codes unchanged
```

### Observed results

| Suite | Result |
|-------|--------|
| `ast-sgrep-lang --lib` | **6 passed** |
| `ast-sgrep-lang --test pattern` | **5 passed** |
| `ast-sgrep-core --lib` | **32 passed** |
| `downstream_correctness` | **6 passed** |
| `search_correctness_epics` | **10 passed** |
| `machine_contracts` | **6 passed** |
| `surface_equivalence` | **2 passed** |
| clippy `-D warnings` (core/cli/lang) | **ok** |
| pi release-acceptance self-test | **accepted** |

---

## Thin-wrapper audit (final)

| Symbol | Callers | Action |
|--------|---------|--------|
| `search_regex` / `search_word` | 0 | deleted |
| `ast_sgrep_cli::run` | 0 | deleted |
| `clear_semantic_ivf_session_cache` | 1 | private |
| `load_or_build_semantic_ivf` / `cached_semantic_ivf` | in-crate | private |
| `function_queries` / `class_queries` | 1 each | replaced by `queries_for` |
| `last_identifier_chain` | 1 | inlined to `last_identifier_in_chain` |
| CLI clap `parse_*` value parsers | 1 each | **kept** (clap requires named fn) |
| migrate/rollback | have callers | **kept** |

---

## Out of scope / preserved on this tip

- Ranking gate semantics (`enforce_result_gates` order) unchanged.
- Fail-closed exotic pattern → ast-grep layer retained (iva9.7).
- Machine success envelopes do **not** invent `exit_code` (this tip’s contract differs from other branches).
- `.beads/` untouched.
- `downstream_correctness` oracles not weakened (no test rewrites).

---

## Batch E — C# grammar alignment + bench honesty + dead-code cleanup (this session)

### Bugfix: C# pattern channel grammar

| Before | After |
|--------|-------|
| `tree_sitter_language(CSharp)` → `tree_sitter_java::LANGUAGE` | `tree_sitter_c_sharp::LANGUAGE` (matches `langs.rs` extraction) |

Regression: `csharp_pattern_channel_uses_csharp_grammar` (unit) + `csharp_literal_pattern_uses_csharp_fixture` (integration).

### Deleted / demoted surfaces

| Symbol | Action |
|--------|--------|
| `ranking_stability` / `RankingStability` | deleted (only self-reference in `bench_suite.rs`) |
| `gitignore::is_ignored` free fn | deleted (unused; `IgnoreMatcher` used directly) |
| `skip` / `text` / `output` lib facades | deleted from `ast-sgrep-core` `lib.rs` |
| `finish_response` | `pub` → `pub(crate)` |
| `search_callers` / `search_defs` / `search_imports` | `pub` → `pub(crate)` |
| `matches_lang` / `dedup_hits` | `pub` → `pub(crate)` |

### Bench honesty (`bench.rs`)

- `speedup_vs_ast_grep` nested under `ast_grep_comparison`; only emitted for `pattern:` queries with ast-grep binary.
- Hybrid/token queries emit `compared: false` + `skipped_reason` (no vacuous speedup).

### CLI index errors

- `index_db_display` helper; `open_indexer` / `open_searcher` include DB path + root in context.

### Indexer

- `index_all` reuses `self.ignore` after `clear()` instead of constructing a second `IgnoreMatcher`.

### Release acceptance dens helpers

- `packageSpecs` / `priorPublishedForLayer` ported from pr25; fail codes unchanged.

### Commands run (this session)

```bash
export PATH="/usr/local/cargo/bin:$PATH"
cd /workspace/.worktrees/pr22
cargo test -p ast-sgrep-lang --test pattern
cargo test -p ast-sgrep-core --test downstream_correctness
cargo test -p ast-sgrep-cli --test machine_contracts
node packages/pi/scripts/release-acceptance.mjs self-test
```

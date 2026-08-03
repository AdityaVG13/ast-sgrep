# Zero Tech Debt Evidence — `test/quality-batch-e2hc-19-oxbj`

Hard evidence for zero-tech-debt cleanup batches.
Commands assume `PATH="/usr/local/cargo/bin:$PATH"` and cwd `/workspace/.worktrees/pr21`.
**Note:** `.beads/` was not modified (per task instruction).

---

## Batch A — live surfaces only; delete zero-caller wrappers

### Philosophy

Rework from intended end state: product crates expose only live surfaces; ranking/CLI helpers exist once; no zero-caller wrappers. Product behavior (search/index/agent contracts) unchanged.

### Caller verification (rg) before deletes

| Symbol | Callers outside definition | Action |
|--------|----------------------------|--------|
| `ast_sgrep_cli::run` / `pub fn run()` | **zero** (`ast_sgrep_cli::run` unused; entry is `main` → `run_process`) | deleted |
| `Searcher::search_regex` / `search_word` | **zero** (modes via `search("regex:…")` / `search("word:…")`) | deleted |
| `last_identifier_chain` thin wrapper | only self | deleted; call `last_identifier_in_chain` directly |
| `tree_sitter_language` crate re-export | unused outside crate | demoted `pub(crate)`; dropped from `lib.rs` re-exports |
| `clear_semantic_ivf_session_cache` | only `mark_semantic_ivf_stale` in-crate | made private |
| `validate_member_indices` | integration test only | `#[cfg(test)]`; test now uses stronger `validate_partition` |
| `isContained` vs `pathContained` | duplicate | unified as `pathContained` |
| `Default for Extractor` | unused (`Extractor::new` only) | removed |

### Extracts / demotions

| Helper | Location | Purpose |
|--------|----------|---------|
| `resolve_output_format` | `crates/ast-sgrep-cli/src/lib.rs` | one format parse path for keyword/search |
| `ensure_nonempty_index` | same | shared empty-index bail for searcher/chain |
| `index_db_display` | same | shared DB path for open error messages |
| `cmp_ranked_hits` | `crates/ast-sgrep-core/src/search/mod.rs` | shared pre-truncate + final sort key |
| `invalidate_response_cache` / `lock_response_cache` | same | ResponseCache poison-clear helper |
| `wait_child_deadline` | `crates/ast-sgrep-core/src/pattern.rs` | shared timed `try_wait` for ast-grep probe/bench |
| extract helpers → `pub(crate)` | `crates/ast-sgrep-lang/src/extract.rs` | crate-internal only |
| `assertVersionTriple` | `packages/pi/extension/src/runtime.ts` | shared version-triple assert (`requireIdentity` for `checkCompatibility`) |
| `packageSpec` / `requiredFilesFor` / `expectReject` / `isForbiddenPackEntry` | `packages/pi/scripts/release-acceptance.mjs` | reindent + pure helpers; fail codes/messages unchanged |
| migrate/rollback | `runtime.ts` | **kept** (have callers) |

### Commands run

```bash
cargo test -p ast-sgrep-cli --test machine_contracts
# → 13 passed

cargo test -p ast-sgrep-core --lib search::
# → 13 passed

cargo test -p ast-sgrep-core --test semantic_ivf_roundtrip
# → 8 passed; 1 ignored

cargo test -p ast-sgrep-lang --lib
# → 6 passed

cargo test -p ast-sgrep-lang --test pattern
# → 5 passed

node packages/pi/scripts/release-acceptance.mjs self-test
# → gate self-test accepted; all expectReject codes unchanged

cd packages/pi/extension && npm run build
# → tsc ok; dist/runtime.js regenerated
```

### Behavior invariants

- Hybrid ranking / `finish_response` gate order unchanged (shared comparator preserves multi-term coverage-first key).
- Machine envelopes / fail codes in release-acceptance unchanged.
- Regex/word modes still work through `ParsedQuery::parse` prefixes on `Searcher::search`.

---

## Batch B — pattern classifier / signatures / kind constants

### End state

| Surface | Location |
|---------|----------|
| `classify_native` / `NativeKind` | `crates/ast-sgrep-lang/src/pattern.rs` (exported) |
| `DECL_PATTERN_PREFIXES` / `DECL_KIND_PREFIXES` / `declaration_prefix` | `ast-sgrep-lang` pattern module |
| `cached_pattern_signatures` / `required_pattern_literal` / `structural_term_signatures` | `crates/ast-sgrep-lang/src/signature.rs` |
| `IDENT_KINDS` / `MEMBER_EXPR_KINDS` / `is_ident_kind` / `is_member_expr_kind` | `crates/ast-sgrep-lang/src/extract.rs` (`pub(crate)`) |
| Core pattern search | consumes lang `cached_pattern_signatures` + `required_pattern_literal` |
| Hybrid `structural_index_pass` | consumes `structural_term_signatures` (byte-identical keys) |

### Refactors pinned

- Flattened `classify_native` trailing-paren empty-ok branch to early return.
- Table-drove `function_queries` / `class_queries` via `FUNCTION_QUERY_TABLE` / `CLASS_QUERY_TABLE`.
- Unified identifier / member kind lists between `pattern.rs` and `extract.rs` (single constants in extract).
- Kept `needs_ast_grep_fallback` for exotic/capability paths; production search still does not spawn ast-grep (bench helper remains gated on `ASGREP_ALLOW_AST_GREP` + absolute `ASGREP_AST_GREP`).

### Commands run

```bash
cargo test -p ast-sgrep-lang -p ast-sgrep-core --lib
# → ast-sgrep-core: 50 passed; ast-sgrep-lang: 6 passed

cargo test -p ast-sgrep-core --test pattern_prefilter --test pattern_routing
# → pattern_prefilter: 3 passed; pattern_routing: 3 passed

cargo test -p ast-sgrep-lang --test pattern
# → 5 passed
```

### Signature byte-identity checks

- Lang unit tests in `signature::tests::*` pin `decl:` / `call-name:` / `kind:` formats and structural term keys.
- Core bakeoff suite `pattern::tests::fixed_bakeoff_suite_is_index_or_native_resolvable` still resolves all 29 fixed patterns via shared `cached_pattern_signatures`.
- Prefilter semantics unchanged: declaration keywords alone are not cross-language required literals (`pattern_prefilter::declaration_keyword_is_not_a_cross_language_required_literal`).

---

## Batch C — index pipeline / module resolve / CLI god-file split

### End state

| Surface | Location / change |
|---------|-------------------|
| One prepare→upsert materialization | `hash_content` + `materialize_upsert` shared by `prepare_file` and `index_content_at` |
| One IgnoreMatcher ownership | `collect_index_candidates` uses `self.ignore` for dir prune + file skip (no second matcher) |
| Watch path normalize | `normalize_watch_path` extracted from `update_paths` |
| Trivia table | `is_trailing_trivia_line` prefix tables (`HASH_PREFIXES` / `C_FAMILY_PREFIXES`) |
| Module resolve table | `module_resolve_rules(lang) → {exts, bases, add_extras}`; BTreeSet candidates unchanged |
| CLI modules | `machine.rs`, `bench.rs`, `watch.rs`, `search_cmd.rs`; `lib.rs` thin clap dispatch (~722 lines) |
| `raw_command_name` | added missing `keyword` (envelope command correctness) |
| Dead delete | `gitignore::is_ignored` free function (zero callers; matcher API remains) |

### Lean wins (low-risk)

| Change | File |
|--------|------|
| Structural markers table for `looks_structural` | `intent.rs` |
| Extract `reassign_stale_ivf_partition`; early-return flatten `rebuild_semantic_ivf_sidecar` | `semantic_ann.rs` |
| MCP `code_search` deprecated alias | **kept** (clients may still call it) |
| Dry-run extension set | **unchanged** — intentional source-like counts; documented vs `INDEXABLE_EXTENSIONS` |

### Commands run

```bash
cargo check -p ast-sgrep-core -p ast-sgrep-cli
# → ok

cargo test -p ast-sgrep-core --lib
# → 50 passed

cargo test -p ast-sgrep-lang --lib
# → 6 passed

cargo test -p ast-sgrep-cli --lib
# → 3 passed

cargo test -p ast-sgrep-cli --test machine_contracts
# → 13 passed

cargo test -p ast-sgrep-core --test resolve_module
# → 5 passed

cargo test -p ast-sgrep-core --lib index::
# → 2 passed (walk prune + body_hash trivia)

cargo test -p ast-sgrep-core --test e2e_smoke --test store_delete --test semantic_ivf_roundtrip
# → e2e_smoke: 5 passed, 1 ignored
# → store_delete: 8 passed
# → semantic_ivf_roundtrip: 8 passed, 1 ignored
```

### Behavior invariants

- Clap surface / machine envelopes unchanged (keyword now correct in pre-parse failure `command`).
- `resolve_module` candidate sets preserved (BTreeSet key order; regression suite green).
- Index prepare/upsert semantics unchanged: same hash, body-hash trivia, semantic chunk gating.
- Dry-run still uses its own source-oriented extension list (not silently merged with `INDEXABLE_EXTENSIONS`).

---

## Batch D — density sweep

### Philosophy

Lean end-state pass: densest remaining modules become table-driven / early-return / single-helper where safe. No ranking-gate or fail-code changes. MCP `code_search` alias kept (compat). `.beads/` untouched. Untracked `package-lock.json` ignored.

### Decision counts (if+match+while+`=>` for Rust; if+else+&&+||+ternary for JS)

| File | Before | After | Δ |
|------|--------|-------|---|
| `crates/ast-sgrep-lang/src/pattern.rs` | 90 | 63 | −27 |
| `pattern.rs` + new `pattern_queries.rs` | 90 | 85 | −5 (tables moved; control-flow thinner) |
| `packages/pi/scripts/release-acceptance.mjs` | 116 (dens 0.364) | 104 (dens 0.295) | −12 |
| `packages/pi/extension/src/runtime.ts` | 170 (dens 0.326) | 162 (dens 0.313) | −8 |
| `crates/ast-sgrep-core/src/search/mod.rs` | 86 | 86 | 0 (helper extract only) |
| `crates/ast-sgrep-core/src/fusion.rs` | 80 | 79 | −1 |
| `crates/ast-sgrep-core/src/semantic_ann.rs` | 52 | 52 | 0 (inline dead thin wrapper) |
| `crates/ast-sgrep-core/src/pipeline_parts.rs` | 16 | 16 | 0 (`measure_hit_len` extract) |
| `crates/ast-sgrep-mcp/src/lib.rs` | — | unchanged | no proven zero-caller dead code beyond compat alias |

### Refactors pinned

| Change | File |
|--------|------|
| Declaration query maps → `pattern_queries` module; shared `queries_for` | `ast-sgrep-lang` |
| Flatten `match_structural` Function/Class arms; `call_match_path` / `call_field_node`; `CALL_KINDS` table; early-return signature recording | `pattern.rs` |
| Required-file consts, `COMMANDS` dispatch, `validatePlatformTarget` / `verifyArtifact` / `assertDirectoryEmpty` / `priorPublishedForLayer` / `sameJson` | `release-acceptance.mjs` |
| `LEGACY_NUMBER_FIELDS` for migrate↔rollback; unified `assertVersionTriple`; merged freshness index branches (missing/dirty/expired) | `runtime.ts` |
| `same_definition_locus` shared by finish + gates (gate order unchanged) | `search/mod.rs` |
| `clamp_channel_weight` shared by weight get/set | `fusion.rs` |
| Inline single-caller `clear_semantic_ivf_session_cache` | `semantic_ann.rs` |
| `measure_hit_len` for literal/lexical/semantic benches | `pipeline_parts.rs` |

### Thin-wrapper audit

| Symbol | Callers | Action |
|--------|---------|--------|
| `clear_semantic_ivf_session_cache` | 1 (`mark_semantic_ivf_stale`) | inlined |
| `function_queries` / `class_queries` | 1 each | replaced by `queries_for` |
| CLI `parse_*` clap parsers | 1 each (value_parser) | **kept** (clap requires named fn) |
| MCP `code_search` | listed + dispatched | **kept** (compat alias) |

### Behavior invariants

- Native pattern match results unchanged (query strings / path rules identical).
- Release fail codes and self-test rejection labels unchanged.
- Runtime migrate/rollback retained; version-triple and freshness lease semantics preserved.
- Hybrid ranking / `enforce_result_gates` order unchanged (shared locus predicate only).

### Commands run

```bash
cargo test -p ast-sgrep-lang --lib --test pattern
cargo test -p ast-sgrep-core --lib search::
cargo test -p ast-sgrep-cli --test machine_contracts
node packages/pi/scripts/release-acceptance.mjs self-test
cd packages/pi/extension && npm run build && npm test
```

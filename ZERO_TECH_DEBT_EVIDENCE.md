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
# → lib: 6 passed; pattern: 5 passed

cargo test -p ast-sgrep-core --lib search::
# → 13 passed (37 filtered out)

cargo test -p ast-sgrep-core --test pattern_prefilter --test pattern_routing
# → pattern_prefilter: 3 passed; pattern_routing: 3 passed

cargo test -p ast-sgrep-cli --test machine_contracts
# → 13 passed

node packages/pi/scripts/release-acceptance.mjs self-test
# → gate self-test accepted; rejection codes unchanged

cd packages/pi/extension && npm run build && npm test
# → tsc ok; 59 passed (runtime + extension suites)
```

---

## Batch E — deep remaining hotspots

### Philosophy

Dig past Batches A–D into densest remaining modules (sqlite / extract / runtime / code-mode / mcp / fusion / embed / agent / semantic_ivf). Prefer table-drive, insert helpers, private demotion, early-return, and single-helper extracts. **No SQL semantic changes.** MCP `code_search` alias kept with protocol-test proof. `.beads/` untouched.

### Decision counts (Rust: if+match+while+`=>`; JS: if+else+&&+||+ternary)

| File | Before | After | Δ |
|------|--------|-------|---|
| `crates/ast-sgrep-core/src/store/sqlite.rs` | 85d / 1303L | 84d / 1323L | −1d (helpers + demotions; SQL strings identical) |
| `crates/ast-sgrep-lang/src/extract.rs` | 71d / 513L | 60d / 521L | −11d |
| `packages/pi/extension/src/runtime.ts` | 129d / 520L | 128d / 587L | −1d (freshness/rebuild helpers; dens 0.248→0.218) |
| `packages/pi/extension/src/code-mode.ts` | 102d / 417L | 102d / 450L | 0d (search/read helpers; dens 0.245→0.227) |
| `crates/ast-sgrep-mcp/src/lib.rs` | 79d / 618L | 78d / 634L | −1d |
| `crates/ast-sgrep-core/src/fusion.rs` | 79d / 628L | 80d / 640L | +1d (stencil extract; control flow flatter) |
| `crates/ast-sgrep-embed/src/math.rs` | 27d / 403L | 26d / 394L | −1d |
| `crates/ast-sgrep-embed/src/embedder.rs` | 65d / 665L | 65d / 657L | 0d (fallback flag via shared `env_flag`) |
| `crates/ast-sgrep-cli/src/supervisor.rs` | 33d / 356L | 30d / 358L | −3d |
| `crates/ast-sgrep-cli/src/agent.rs` | 28d / 345L | 28d / 344L | 0d (early-return doctor) |
| `crates/ast-sgrep-core/src/semantic_ivf.rs` | 51d / 523L | 39d / 524L | −12d |
| `crates/ast-sgrep-core/src/search/mod.rs` | 86d / 1186L | 86d / 1186L | 0 (no safe further lean without ranking risk) |
| `crates/ast-sgrep-lang/src/pattern.rs` | — | inline `call_target_path` | thin single-caller wrapper deleted |

### Refactors pinned

| Change | File |
|--------|------|
| `insert_each` helper for callers/pattern_nodes/imports; SQL text byte-identical | `sqlite.rs` |
| `map_sorted_files` shared by semantic/legacy per-file queries | `sqlite.rs` |
| Demote zero-external-caller APIs: `delete_meta`, `bump_semantic_data_version`, `file_lines`, `file_exists` → private; many in-crate-only methods → `pub(crate)` | `sqlite.rs` |
| Early-return `remove_file` | `sqlite.rs` |
| Kind tables + `field_name_text` for `enclosing_symbol_name`; comment/string kind table; early-return KindRule arms | `extract.rs` |
| Freshness helpers: `emptyRootFreshness` / `leaseExpired` / `isFresh` / `resolveIndexHealth` / `reconcileIndex` / `#absorbPending` | `runtime.ts` |
| Rebuild helpers: `swapRebuiltIndex` / `rebuildFailureDetails` | `runtime.ts` |
| `runSearch` + `resolveReadableFile` / `openStableHandle` (delete search/read dupes) | `code-mode.ts` |
| `dispatch_tool` merges `keyword_search`\|`code_search`; `search_tool` schema helper; **alias kept** | `mcp/lib.rs` |
| `channel_sensitivity` stencil extract; empty early-return in `pairwise_loss` | `fusion.rs` |
| Delete unused `cosine_scores_for`; flatten `top_k_flat_similarity` push path | `math.rs` + `embed/lib.rs` |
| `ASGREP_EMBED_FALLBACK` via shared `env_flag` | `embedder.rs` |
| Inline single-caller `call_target_path` | `pattern.rs` |
| Flatten `map_and_parse` Option/filter chains | `semantic_ivf.rs` |
| Early-return doctor; early-return nonce auth | `agent.rs` / `supervisor.rs` |

### Thin-wrapper / dead-path audit

| Symbol | Callers | Action |
|--------|---------|--------|
| `cosine_scores_for` | **zero** (only re-export) | deleted + dropped from `ast-sgrep-embed` re-exports |
| `call_target_path` | 1 (`call_match_path`) | inlined |
| `fuse_rrf` | fuzz target + `score_lexical_rrf` | **kept** (fuzz uses public API) |
| MCP `code_search` | listed + `dispatch_tool` arm + `tests/protocol.rs` | **kept** (compat) |
| Code-mode `find`/`astFind`/`semantic`/`read` | public API aliases | **kept** |
| `field_child` | many KindRule arms | **kept** (multi-caller) |
| Search ranking helpers | shared by gates | untouched |

### MCP compat proof

- `crates/ast-sgrep-mcp/tests/protocol.rs` still enumerates `code_search` in tools/list and exercises `("code_search", …)` tool calls.
- `dispatch_tool` documents `keyword_search | code_search` → `AgentSearchMode::Keyword`.

### Behavior invariants

- All sqlite INSERT/UPDATE/DELETE/SELECT SQL strings for mutated paths unchanged; demotions are visibility-only.
- Hybrid ranking / fusion RRF math unchanged (helper extract only).
- Freshness lease + incompatible rebuild swap/backup semantics preserved.
- Code-mode search argv shape unchanged (`keyword` / bare `--` pattern / `semantic`).

### Clippy dead_code (touched crates)

```bash
cargo clippy -p ast-sgrep-core -p ast-sgrep-lang -p ast-sgrep-embed -p ast-sgrep-mcp -p ast-sgrep-cli --lib -- -W dead_code
# → no dead_code diagnostics on Batch E surfaces
# → applied clippy::manual_contains on extract kind tables
```

### Commands run

```bash
cargo test -p ast-sgrep-core -p ast-sgrep-lang -p ast-sgrep-embed --lib
# → core: 50 passed; lang: 6 passed; embed: 16 passed

cargo test -p ast-sgrep-cli --lib --test machine_contracts
# → cli lib: 3 passed; machine_contracts: 13 passed

cargo test -p ast-sgrep-mcp --test protocol
# → 9 passed (includes code_search listing + dispatch)

cargo test -p ast-sgrep-lsp --tests
# → lsp.rs: 4 passed

cargo test -p ast-sgrep-lang --test pattern --test extraction_goldens
# → pattern: 5 passed; extraction_goldens: 1 passed

cargo test -p ast-sgrep-core --test store_delete --test semantic_ivf_roundtrip --test e2e_smoke
# → store_delete: 8 passed
# → semantic_ivf_roundtrip: 8 passed; 1 ignored
# → e2e_smoke: 5 passed; 1 ignored

cd packages/pi/extension && npm run build && npm test
# → tsc ok; 59 passed
```

---

## Batch F — zero-tech-debt follow-up (PR21)

### Philosophy

Finish remaining cleanup from Batches A–E: delete orphaned helpers, split CLI god-file, extract module resolve / embed pass boundaries, table-drive fusion channel accessors, MCP env-flag parity. **No ranking-gate order, machine envelope, or beads changes.**

### Deletes / demotions

| Symbol | Action |
|--------|--------|
| `validate_member_indices` | deleted (`semantic_ann.rs`; zero callers) |
| `bases_python` / `bases_js` / `bases_go` / `bases_rust` thin wrappers | deleted; `module_resolve_rules` points at `resolve_bases_*` directly |
| `declaration_prefix` / `match_literal_pattern` | `pub(crate)`; dropped from `ast-sgrep-lang` public exports; integration tests use `match_pattern` |

### Extracts / splits

| Surface | Location |
|---------|----------|
| `cli_args.rs` | clap structs (`Cli`, `Commands`, `SearchTuning`, parsers, `Cli::active_tuning`) |
| `index_cmd.rs` | `open_indexer`, `index_options`, dry-run, status, `with_index`, `search_options` |
| `lib.rs` | thin `main` / `run_cli` / `run_command` dispatch (~210 lines) |
| `store/module_resolve.rs` | `collect_module_candidates` + language tables (split from `sqlite.rs`) |
| `search/passes/embed.rs` | `SemanticCache`, `load_semantic_context`, `run_embed_pass` (boundary from `search/mod.rs`) |
| MCP `searcher_key`, `lock_or_recover`, `base_index_options` | `ast-sgrep-mcp/src/lib.rs` |
| MCP `ASGREP_NO_EMBED` | `!ast_sgrep_core::env_flag::env_flag("ASGREP_NO_EMBED")` |

### Fusion table-drive

| Accessor | Mechanism |
|----------|-----------|
| `channel_for_kind` | `HIT_KIND_CHANNEL_IDX` + `FusionChannel::ALL` |
| `weight` / `set_weight` | `WEIGHT_GETTERS` / `WEIGHT_SETTERS` indexed by `channel.index()` |
| `ChannelRanks::get` / `set_best` | `RANK_GETTERS` / `RANK_SETTERS` tables |
| `canonical_priority` | `CANONICAL_PRIORITY` table keyed by `hit_kind_idx` |
| `FusionChannel::index` | enum discriminant (`self as usize`) |

`fuse_rrf` in `rank.rs` **kept** (fuzz target + `score_lexical_rrf`).

### Behavior invariants

- Hybrid ranking / `finish_response` gate order unchanged.
- `resolve_module` candidate sets unchanged (5 regression tests green).
- Fusion weighted RRF math unchanged (5 fusion unit tests green).
- Machine envelopes unchanged (`machine_contracts`: 13 passed).

### Skips (explicit)

- `.beads/` untouched (per task).
- `fuse_rrf` public API retained.
- MCP `code_search` compat alias retained.
- Dry-run extension set unchanged.

### Commands run

```bash
cargo test -p ast-sgrep-cli --test machine_contracts
# → 13 passed

cargo test -p ast-sgrep-core --lib
# → 50 passed

cargo test -p ast-sgrep-mcp --lib
# → 0 tests (lib crate; protocol tests separate)

cargo test -p ast-sgrep-lang --lib
# → 6 passed

cargo test -p ast-sgrep-core --lib fusion::
# → 5 passed

cargo test -p ast-sgrep-core --test resolve_module
# → 5 passed

cargo test -p ast-sgrep-lang --test pattern
# → 5 passed
```

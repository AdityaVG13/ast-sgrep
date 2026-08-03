# Zero Tech Debt Evidence — `fix/p1-correctness-batch`

Hard evidence for zero-tech-debt cleanup on the P1 correctness / durability tip.
Commands assume `PATH="/usr/local/cargo/bin:$PATH"` and cwd `/workspace/.worktrees/pr20`.
**Note:** `.beads/` was not modified (per task instruction). No new PR opened; branch pushed only.

---

## End state

Store/index/durability path is table-driven and single-pipeline where safe: one module-resolve story, one prepare/upsert materialization, no dead wrappers; `sqlite.rs` / `index.rs` leaner without changing durability (tx/PRAGMA / meta bump / UTF-8 path) semantics.

---

## Batch A — dead wrappers + live helpers

### Caller verification (rg) before deletes

| Symbol | Callers outside definition | Action |
|--------|----------------------------|--------|
| `ast_sgrep_cli::run` / `pub fn run()` | **zero** (entry is `main` → `run_process`) | deleted |
| `Searcher::search_regex` / `search_word` | **zero** (modes via `search("regex:…")` / `search("word:…")`) | deleted |
| `gitignore::is_ignored` free fn | **zero** (matcher API remains) | deleted |
| `last_identifier_chain` thin wrapper | only self | deleted; call `last_identifier_in_chain` directly |
| `tree_sitter_language` crate re-export | unused outside crate | demoted `pub(crate)`; dropped from `lib.rs` re-exports |
| `clear_semantic_ivf_session_cache` | only `mark_semantic_ivf_stale` | made private |
| `validate_member_indices` | production + test | production → `validate_partition`; member helper `#[cfg(test)]` |
| `isContained` vs `pathContained` | duplicate | unified as `pathContained` |
| `Default for Extractor` | unused (`Extractor::new` only) | removed |
| `file_tx_depth_for_test` | tests only | `#[cfg(test)]` (inject already was) |

### Extracts

| Helper | Location | Purpose |
|--------|----------|---------|
| `resolve_output_format` | `crates/ast-sgrep-cli/src/search_cmd.rs` | one format parse path |
| `cmp_ranked_hits` | `crates/ast-sgrep-core/src/search/mod.rs` | shared pre-truncate + final sort key |
| `lock_response_cache` | same | ResponseCache poison-tolerant lock |
| `assertVersionTriple` | `packages/pi/extension/src/runtime.ts` | shared version identity (`requireIdentity` for `checkCompatibility`) |
| `packageSpec` / `requiredFilesFor` / `expectReject` / `COMMANDS` | `packages/pi/scripts/release-acceptance.mjs` | pure helpers; fail codes/messages unchanged |

### Behavior invariants

- Regex/word modes still work through `ParsedQuery::parse` prefixes on `Searcher::search`.
- Hybrid ranking / `finish_response` gate order unchanged (shared comparator preserves coverage-first key; pre-truncate still uses coverage=0).
- Machine envelopes / fail codes in release-acceptance unchanged.
- Version-triple present-field mismatch order preserved (no incompleteness conjunction added on this tip).

---

## Batch B — store/index single pipeline

### End state

| Surface | Location / change |
|---------|-------------------|
| Module resolve table | `module_resolve_rules(lang) → {exts, bases, add_extras}`; BTreeSet candidates + UTF-8 `normalize_rel` Result skip unchanged |
| Insert helpers | `insert_neural_file_chunks`; `structure_equal_file_id` early-return flatten on upsert |
| One prepare→upsert materialization | `hash_content` + `materialize_upsert` shared by `prepare_file` and `index_content_at` |
| One IgnoreMatcher ownership | `collect_index_candidates` uses `self.ignore` for dir prune + file skip |
| Watch path normalize | `normalize_watch_path` extracted from `update_paths` |
| Trivia table | `is_trailing_trivia_line` prefix table (`//`, `#`, `/*`, `*`, `--`) — language-agnostic on this tip |
| CLI modules | `machine.rs`, `bench.rs`, `watch.rs`, `search_cmd.rs`; `lib.rs` thin clap dispatch (~423 lines) |
| ANN rebuild | `reassign_stale_ivf_partition` early-return flatten |
| Intent markers | `looks_structural` marker table |

### Durability invariants (must stay green)

- File/bulk tx + PRAGMA synchronous restore paths untouched.
- `set_meta` body-hash failures still propagate (`j97d.3ddd`).
- `needs_semantic_v1_rewrite` gates on structure-equal refresh + single-file body-hash refresh unchanged.
- `bump_semantic_data_version` / `bump_index_data_version` call sites preserved.
- UTF-8 path rejection (`indexed_rel_path` / `normalize_rel`) preserved.
- semantic-v2 promotion only after clean full `index_all`.

---

## Commands run

```bash
cargo test -p ast-sgrep-core --test durability_epics --test store_delete \
  --test store_pragmas --test resolve_module --test semantic_ivf_roundtrip \
  --test semantic_cache_version
# → durability_epics: 16 passed
# → resolve_module: 5 passed
# → semantic_cache_version: 4 passed
# → semantic_ivf_roundtrip: 3 passed
# → store_delete: 8 passed
# → store_pragmas: 1 passed

cargo test -p ast-sgrep-core --lib
# → 24 passed

cargo test -p ast-sgrep-lang --lib
# → 3 passed

cargo test -p ast-sgrep-cli --lib --test machine_contracts
# → lib (watch): 5 passed; machine_contracts: 6 passed

cargo test -p ast-sgrep-core --test p1_correctness_batch \
  --test response_cache_version --test semantic_v1_rewrite
# → p1_correctness_batch: 4 passed
# → response_cache_version: 2 passed
# → semantic_v1_rewrite: 2 passed

node packages/pi/scripts/release-acceptance.mjs self-test
# → gate self-test accepted; rejection codes unchanged

cd packages/pi/extension && npm test
# → 53 passed (runtime + extension suites)
```

### Decision density (if+else+&&+||+ternary)

| File | Before | After | Δ dens |
|------|--------|-------|--------|
| `packages/pi/extension/src/runtime.ts` | 190 / 502 (0.378) | 184 / 512 (0.359) | −0.019 |
| `packages/pi/scripts/release-acceptance.mjs` | 121 / 274 (0.442) | 110 / 299 (0.368) | −0.074 |

---

## Line counts (approx)

| File | Before | After |
|------|--------|-------|
| `store/sqlite.rs` | 1305 | 1381 (table + helpers; control-flow flatter) |
| `index.rs` | 839 | 867 (shared materialize; single matcher) |
| `search/mod.rs` | 855 | 830 |
| `cli/lib.rs` | 1119 | 423 (+ machine/bench/watch/search_cmd) |

---

## Batch C — store/search debt cleanup (this session)

### Caller verification (rg) before deletes

| Symbol | Callers outside definition | Action |
|--------|----------------------------|--------|
| `bases_python` / `bases_js` / `bases_go` / `bases_rust` adapters | only `module_resolve_rules` table | deleted; table calls `resolve_bases_*` directly |
| `ranking_stability` | **zero** | deleted |
| `RankingStability` | **zero** (only used by deleted fn) | deleted |
| `gitignore::is_ignored` free fn | **zero** (matcher API remains) | already deleted in Batch A — skip |

### Changes

| Item | Location | Notes |
|------|----------|-------|
| Regex `file_map` | `search/passes/regex.rs` | Reuse `lines` when not on trigram path; trigram path still loads full index for context |
| `with_file_tx` | `index.rs` `index_content_at` | Replaces manual begin/commit/rollback on body-hash refresh path |
| `delete_meta` | `store/sqlite.rs` | `pub` → private (same-crate callers only) |
| `refresh_lines_only` | `store/sqlite.rs` | `pub` → `pub(crate)` |
| `with_file_tx` | `store/sqlite.rs` | `fn` → `pub(crate)` for `index.rs` |
| `emb_vec` | `store/sql.rs` | `pub(crate)`; shared by `read_sem_row` / `read_legacy_emb` and `semantic_chunks_by_ids` |
| Module resolve | `store/module_resolve.rs` | Move-only split from `sqlite.rs`; unified `resolve_bases_*` signatures |

### Durability invariants (unchanged)

- Nested `with_file_tx` depth/poison semantics untouched.
- Body-hash / structure-equal refresh gates unchanged.
- Ranking scores and hybrid sort keys unchanged.

### Commands run (this session)

```bash
export PATH="/usr/local/cargo/bin:$PATH"
cd /workspace/.worktrees/pr20
cargo test -p ast-sgrep-core --test durability_epics
# → 16 passed
cargo test -p ast-sgrep-core --lib
# → 24 passed
cargo test -p ast-sgrep-cli --test machine_contracts
# → 6 passed
```

---

## Batch D — second-pass zero tech debt (PR20)

### Caller verification (rg) before deletes

| Symbol | Callers outside definition | Action |
|--------|----------------------------|--------|
| `pub mod skip` / `text` / `output` facades | **zero** | deleted; `format_hit_line` re-exported from `search` |
| Vacuous `ast_grep_pattern_for_query` bench speedup | hybrid/token queries | demoted via `ast_grep_comparison` nested object |

### Bench honesty port (from pr25)

| Change | Location |
|--------|----------|
| `ast_grep_comparison`: pattern-only timing, vacuous speedup demotion | `bench.rs` |
| `cv_pct`, `mean_ms`, bench history / ratchet | `bench.rs` |
| `print_machine_json_with_ok` for suite single-envelope failures | `machine.rs` / `bench.rs` |

### Commands run (this session)

```bash
export PATH="/usr/local/cargo/bin:$PATH"
cd /workspace/.worktrees/pr20
cargo test -p ast-sgrep-core --test durability_epics
cargo test -p ast-sgrep-cli --test machine_contracts
```

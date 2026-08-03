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
# (filled after test pass)
```

---

## Line counts (approx)

| File | Before | After |
|------|--------|-------|
| `store/sqlite.rs` | 1305 | 1381 (table + helpers; control-flow flatter) |
| `index.rs` | 839 | 867 (shared materialize; single matcher) |
| `search/mod.rs` | 855 | 830 |
| `cli/lib.rs` | 1119 | 423 (+ machine/bench/watch/search_cmd) |

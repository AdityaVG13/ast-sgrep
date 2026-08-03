# Zero Tech Debt Evidence — `fix/lsp-symbol-correctness-p1` (PR #14)

Hard evidence for zero-tech-debt cleanup on the LSP + VS Code multi-root surface.
Commands assume `PATH="/usr/local/cargo/bin:$PATH"` and cwd `/workspace/.worktrees/pr14`.
**Note:** `.beads/` was not modified (per task instruction). `editors/vscode/out/**` ignored (compiled).

---

## Intended end state

One clear flow: **folder → client binding → search limit clamp → hit path resolve**.
No dead facade wrappers, no duplicated containment/root logic, `support.rs` / `extension.ts` lean.

---

## Caller verification (rg) before deletes

| Symbol | Callers outside definition | Action |
|--------|----------------------------|--------|
| `ast_sgrep_lsp::{uri,convert,symbols,text_edit,transport}` facades | **zero** | deleted from `lib.rs` |
| crate-root `pub use support::path_to_file_uri` | **zero** (tests used `backend::path_to_uri`) | deleted; tests use `support::path_to_file_uri` |
| `backend::{path_to_uri, uri_to_rel_path}` re-exports | alias only / internal | deleted; call `path_to_file_uri` / `uri_to_rel_path` directly |
| `isContained` vs `pathContained` in `runtime.ts` | duplicate body | unified as `pathContained` |
| `resolveHitUri` / `resolveHitUriMultiRoot` in `extension.ts` | duplicated `multiRoot.resolveHitPath` | deleted; extension calls `resolveHitPath` |

Kept live: `settings::AsgrepSettings` (testkit), multi-root fail-closed binding, `clamp_lsp_search_limit` (0→default, cap 1000).

---

## Extracts / lean rewrites

| Helper / change | Location | Purpose |
|-----------------|----------|---------|
| `document_symbol_kind` table | `support.rs` | one SymbolKind map for document symbols |
| `hit_symbol_kind` / `hit_detail` | `support.rs` | workspace-symbol kind/detail without inline match clutter |
| `is_windows_drive` / early-return URI encode | `support.rs` | product-intent path URI helpers |
| `dirty_map` | `backend.rs` | one poison-aware dirty-buffer lock |
| `sync_rel_path` | `server.rs` | one didOpen/didSave/didChange index-error path (`window/showMessage`) |
| `folderForUriPath` / `resolveHitPath` / `hitFilePath` / `hitLineNumber` | `multiRoot.ts` | single path-resolve + hit-field source |
| `LEGACY_NUMBER_FIELDS` | `runtime.ts` | migrate/rollback share one field table |
| `packageSpec` / `requiredFilesFor` / `isForbiddenPackEntry` / `expectReject` / `sameJson` / `validatePlatformTarget` | `release-acceptance.mjs` | reindent + pure helpers; **fail codes unchanged** |

---

## Behavior invariants (must hold)

1. Multi-root: one LSP client per folder; search binds to active editor folder; no silent `folders[0]` fallback.
2. Relative hits resolve preferred folder first, then other roots; miss falls back to preferred join.
3. `asgrep/search` limit: `0` → `SearchOptions::default_limit()`, hard cap **1000**.
4. `index_ready` only after successful full `index_all`; dirty buffers reapplied after disk reindex.
5. Sync notification index errors still surface via `window/showMessage`.
6. Release-acceptance self-test fail codes/messages unchanged.

---

## Commands run

```bash
export PATH="/usr/local/cargo/bin:$PATH"
cd /workspace/.worktrees/pr14

cargo test -p ast-sgrep-lsp
# → lib: 1 passed (limit_tests)
# → integration tests/lsp.rs: 11 passed
# → 0 failed

cd editors/vscode && npm run test:multi-root
# → 6 passed (folder binding, hit resolve, hit fields)

node packages/pi/scripts/release-acceptance.mjs self-test
# → gate self-test accepted; rejected dirty=ASGREP_RELEASE_DIRTY,
#   wrong-tag=ASGREP_RELEASE_TAG_VERSION, wrong-commit=ASGREP_RELEASE_TAG_COMMIT,
#   fully-published=ASGREP_RELEASE_DUPLICATE_VERSION, version-skew=ASGREP_RELEASE_VERSION_SKEW,
#   missing-checksum=ASGREP_RELEASE_CHECKSUM_MISSING, checksum-mismatch=ASGREP_RELEASE_CHECKSUM_MISMATCH,
#   local-publish=ASGREP_RELEASE_OIDC_REQUIRED
```

---

## Follow-up — shared CLI / lang / search dens (post-LSP ZTD)

Deep dig beyond the LSP/VS Code surface into shared CLI god-file, lang pattern/extract,
core search wrappers, and Pi scripts. Same tip; **no new PR**; `.beads/` untouched.
Chain contract on this tip remains `top_n: 1`. Ranking pre-truncate stays score-only
(this tip’s order — not the coverage-aware pool from other tips). Pattern search stays
tip fail-open (no exotic fail-closed layer). CSharp pattern grammar remains Java stand-in.

### Caller verification (rg) before deletes

| Symbol | Callers outside definition | Action |
|--------|----------------------------|--------|
| `ast_sgrep_cli::run` / `pub fn run()` | **zero** (entry is `main` → `run_process`) | deleted |
| `Searcher::search_regex` / `search_word` | **zero** (modes via `search("regex:…")` / `search("word:…")`) | deleted |
| `function_queries` / `class_queries` | 1 each (match_structural) | replaced by `queries_for` tables |
| `last_identifier_chain` thin wrapper | only self | deleted; call `last_identifier_in_chain` |
| Core local `cached_pattern_signatures` / `is_pattern_*` | duplicated | deleted; import lang `signature` |

### CLI god-file split

| Metric | Before | After |
|--------|--------|-------|
| `crates/ast-sgrep-cli/src/lib.rs` lines | **919** | **420** |
| Decisions (`if`+`while`+`=>`) | 103 | 50 |
| Decisions (`if`+`match`+`while`+`=>`) | 115 | 55 |

| Module | Lines | Role |
|--------|-------|------|
| `machine.rs` | 86 | envelopes + `raw_command_name` / failure helpers |
| `bench.rs` | 293 | suite / batch / single bench |
| `watch.rs` | 84 | incremental watch loop |
| `search_cmd.rs` | 98 | search/chain + `resolve_output_format` (`top_n: 1` preserved) |

### Lang pattern / extract / signatures

| Surface | Location |
|---------|----------|
| `FUNCTION_QUERY_TABLE` / `CLASS_QUERY_TABLE` / `queries_for` | `pattern_queries.rs` |
| `classify_native` / `NativeKind` / `DECL_PATTERN_PREFIXES` / `DECL_KIND_PREFIXES` | `pattern.rs` (exported) |
| `cached_pattern_signatures` / `required_pattern_literal` / `structural_term_signatures` | `signature.rs` |
| `IDENT_KINDS` / `MEMBER_EXPR_KINDS` / `is_ident_kind` / `is_member_expr_kind` | `extract.rs` (`pub(crate)`) |

| File | Before (user / full) | After (user / full) |
|------|----------------------|---------------------|
| `ast-sgrep-lang/src/pattern.rs` | 81 / 113 (578L) | 56 / 63 (526L) |
| `ast-sgrep-lang/src/extract.rs` | 69 / 73 (499L) | 68 / 71 (513L) |
| `ast-sgrep-core/src/pattern.rs` | — / — | 29 / 31 (296L; local signature fns deleted) |

Signature byte-identity pinned by `signature::tests::*` (`decl:` / `call-name:` / `kind:` / six structural keys).

### Core search helpers (ranking unchanged)

| Helper / change | Location | Purpose |
|-----------------|----------|---------|
| delete `search_regex` / `search_word` | `search/mod.rs` | zero-caller wrappers |
| `structural_term_signatures` | lang → `structural_index_pass` | byte-identical keys; score still `SCORE_PATTERN * 0.85` |
| `emit_response` / `cmp_coverage_score` / `lock_response_cache` | `search/mod.rs` | flatten response + keyed compare + poison lock |
| Tip pre-truncate | same | still score→file→line to `keep` (not coverage-aware `pre_keep`) |

### Pi dens (if+else+&&+||+ternary)

| File | Before | After |
|------|--------|-------|
| `packages/pi/scripts/release-acceptance.mjs` | 624L (expanded); absolute decisions high | **288L**; `COMMANDS` + `assertDirectoryEmpty`; fail codes unchanged |
| `packages/pi/extension/src/runtime.ts` | 501L | `assertVersionTriple`; merged missing/dirty/expired index branch; dist regenerated |

### Behavior invariants (follow-up)

1. Chain still hardcodes `top_n: 1` (this tip).
2. Hybrid ranking gate order / score-only pre-truncate unchanged.
3. Regex/word modes still work through `ParsedQuery` prefixes on `Searcher::search`.
4. Machine success envelopes still omit `exit_code` when `ok: true`.
5. Release-acceptance self-test rejection codes unchanged.
6. LSP / symbol extraction goldens still pass; CSharp still shares Java grammar for patterns.

### Commands run (follow-up)

```bash
export PATH="/usr/local/cargo/bin:$PATH"
cd /workspace/.worktrees/pr14

cargo test -p ast-sgrep-lang --lib --test pattern --test extraction_goldens
# → lib: 6 passed (incl. signature byte-identity)
# → pattern: 5 passed
# → extraction_goldens: 1 passed

cargo test -p ast-sgrep-lsp
cargo test -p ast-sgrep-cli --test machine_contracts
node packages/pi/scripts/release-acceptance.mjs self-test
```

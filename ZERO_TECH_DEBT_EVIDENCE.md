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
# → lib + integration results recorded below after push

cd editors/vscode && npm run test:multi-root
# → multi-root helpers recorded below

node packages/pi/scripts/release-acceptance.mjs self-test
# → gate self-test; fail codes unchanged
```

(Results appended after the test run in this session.)

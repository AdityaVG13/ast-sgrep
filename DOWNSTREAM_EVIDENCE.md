# DOWNSTREAM_EVIDENCE — PR #14 (`fix/lsp-symbol-correctness-p1`)

Hard evidence for beads closed on this branch tip.
Commands assume `PATH="/usr/local/cargo/bin:$PATH"` and cwd `/workspace/.worktrees/pr14`.
**Note:** `.beads/` was not modified (per task instruction).

---

| Bead | Evidence |
|------|----------|
| `ast-sgrep-y50x` | VS Code extension no longer binds server cwd / hit resolution to `workspaceFolders[0]` alone. **One `LanguageClient` per workspace folder** (`cwd` = that folder). Search uses the **active editor’s folder** and **fails closed** in multi-root when no active document (no silent first-folder fallback). Relative hits resolve via preferred search folder then other roots (`resolveHitUriMultiRoot` / `multiRoot.ts`). Tests: `npm run test:multi-root` (5 passed). README documents multi-root behavior. |
| `ast-sgrep-ei0i` (LSP path) | `asgrep/search` limit now uses `clamp_lsp_search_limit`: `0` → `SearchOptions::default_limit()`, hard cap **1000** (was `.clamp(1, 500)`). README updated. Test: `cargo test -p ast-sgrep-lsp --lib limit_tests` → passed. |

## Focused commands run

```bash
cargo test -p ast-sgrep-lsp --lib limit_tests
cd editors/vscode && npm run test:multi-root && npx tsc --noEmit -p ./
```

All passed.

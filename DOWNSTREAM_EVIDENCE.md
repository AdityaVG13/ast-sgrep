# DOWNSTREAM_EVIDENCE — PR #23 (`fix/csharp-grammar-pattern-difu-5`)

Hard evidence for beads closed on this branch tip.
Commands assume `PATH="/usr/local/cargo/bin:$PATH"` and cwd `/workspace/.worktrees/pr23`.
**Note:** `.beads/` was not modified (per task instruction).

---

| Bead | Evidence |
|------|----------|
| `ast-sgrep-ufsd` | Docs already list 13 languages matching `Language::all()` (`README.md`, `docs/comparison.md`, `docs/how-it-works.md`). VS Code `package.json` activationEvents + `extension.ts` documentSelector include swift/c/cpp/kotlin/php. **Updated** `editors/vscode/README.md` from the stale Rust/Python/TS/Go-only surface to the full 13-language list. |
| `ast-sgrep-amm8` | Production still has `search_pattern_ast_grep` / `parse_ast_grep_json`. Added `Language::parse` + `Language::normalize_id`; ast-grep JSON `language` now maps Title Case / aliases → `Language::as_str`. `matches_lang` compares normalized ids. Tests: `language_id_tests::*`, `pattern::tests::ast_grep_language_field_normalizes_to_as_str`, `native_and_normalized_ast_grep_share_as_str_casing`. |
| `ast-sgrep-0b1a` | Deliverable: `docs/validation/symbol-canonicalization-audit.md` enumerates produce/consume sites (rank, query, semantic_ann/ivf fingerprints, intent, store/sql, LSP, plugins), equivalence rules, and divergences. Clear bugs fixed while auditing: amm8 language casing + case-tolerant `matches_lang`. Follow-ups noted (dedup case sensitivity, SQL vs Rust `lower`). |

## Focused commands run

```bash
cargo test -p ast-sgrep-lang --lib language_id_tests
cargo test -p ast-sgrep-core --lib pattern::tests
```

Both passed (2 + 2 tests).

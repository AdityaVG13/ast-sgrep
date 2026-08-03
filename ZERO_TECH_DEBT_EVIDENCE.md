# Zero Tech Debt Evidence — `test/quality-batch-e2hc-19-oxbj`

Hard evidence for consolidated pattern/signature helpers.
Commands assume `PATH="/usr/local/cargo/bin:$PATH"` and cwd `/workspace/.worktrees/pr21`.
**Note:** `.beads/` was not modified (per task instruction).

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

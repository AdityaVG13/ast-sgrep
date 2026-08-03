# Zero Tech Debt Evidence — `fix/csharp-grammar-pattern-difu-5`

Hard evidence for deep zero-tech-debt cleanup on the 13-language tip
(C/C++/Kotlin/PHP/Swift/Ruby singleton behavior preserved).
Commands assume `PATH="/usr/local/cargo/bin:$PATH"` and cwd `/workspace/.worktrees/pr23`.
**Note:** `.beads/` was not modified (per task instruction). No new PR opened.

---

## Intended end state

Language surface is table-driven: pattern matching and extraction for 13 languages
use shared kind constants, query tables, and one classifier. `pattern.rs` and
`extract.rs` are dramatically leaner without changing extraction/pattern goldens.

| Surface | Location |
|---------|----------|
| `FUNCTION_QUERY_TABLE` / `CLASS_QUERY_TABLE` / `queries_for` / `class_queries_for` | `crates/ast-sgrep-lang/src/pattern_queries.rs` |
| `classify_native` / `NativeKind` / `DECL_PATTERN_PREFIXES` / `DECL_KIND_PREFIXES` | `crates/ast-sgrep-lang/src/pattern.rs` (exported) |
| `cached_pattern_signatures` / `required_pattern_literal` / `structural_term_signatures` | `crates/ast-sgrep-lang/src/signature.rs` |
| `IDENT_KINDS` / `MEMBER_EXPR_KINDS` / `STRING_KINDS` / `COMMENT_OR_STRING_KINDS` | `crates/ast-sgrep-lang/src/extract.rs` (`pub(crate)`) |
| Core pattern search | consumes lang `cached_pattern_signatures` |
| Hybrid `structural_index_pass` | consumes `structural_term_signatures` (byte-identical keys) |

---

## Decision counts

Methodology: `if` + `while` + `=>` (user-facing density; matches the pre-task ~160 / ~91 report).
Also report `if` + `match` + `while` + `=>` for continuity with other ZTD batches.
`pattern_queries.rs` “full” is inflated by `@match` capture names inside query strings.

| File | Before (user / full) | After (user / full) | Δ user |
|------|----------------------|---------------------|--------|
| `crates/ast-sgrep-lang/src/pattern.rs` | **159** / 250 (806 lines) | **80** / 91 (638 lines) | **−79** |
| `crates/ast-sgrep-lang/src/extract.rs` | **96** / 101 (636 lines) | **93** / 95 (657 lines) | −3 |
| `pattern.rs` + new `pattern_queries.rs` | 159 user | 82 user | −77 (tables moved; control-flow thinner) |
| `crates/ast-sgrep-core/src/pattern.rs` | 40 / 43 | 31 / 34 | −9 (classifier deleted; lang export) |
| `crates/ast-sgrep-core/src/index.rs` | 95 / 108 | 95 / 108 | 0 (`hash_content` + trivia tables) |
| `crates/ast-sgrep-cli/src/lib.rs` | 103 / 115 | 103 / 115 | 0 (`resolve_output_format` extract) |

---

## Lang crate refactors pinned

### `pattern.rs`

- Moved language→query maps into `pattern_queries` (`FUNCTION_QUERY_TABLE`, keyword-scoped `CLASS_QUERY_TABLE`).
- Flattened `classify_native` trailing-paren empty-ok branch to early return; shared `DECL_PATTERN_PREFIXES`.
- Shared walks: `call_match_path` / `call_field_node` / `record_node_signatures`; deleted `last_identifier_chain` wrapper.
- Unified identifier / member kind checks via extract `is_ident_kind` / `is_member_expr_kind`.
- `CALL_KINDS` table (keeps C# `invocation_expression` + PHP call forms).
- `declaration_prefix` table-driven via `DECL_KIND_PREFIXES`; `class_declaration` still inspects Swift `declaration_kind` / Kotlin keywords.
- Kept keyword-specific `NativeKind::Class { keyword, … }` so C#/Swift/C++/Kotlin/PHP singleton filters stay exact.
- Kept Swift/Kotlin `call_expression` first-named-child fallback.

### `extract.rs`

- Shared `IDENT_KINDS` / `MEMBER_EXPR_KINDS` / `STRING_KINDS` / `COMMENT_OR_STRING_KINDS`.
- Demoted helpers / `KindRule` / `Extractor` to `pub(crate)` (crate-internal only).
- Removed unused `Default for Extractor` (`Extractor::new` only).
- `last_identifier_in_chain` / `is_in_comment_or_string` / `collect_identifiers` / `declarator_name` use kind consts.
- Did **not** split `apply_kind_rule` (clarity does not win — single dispatch match remains).

### `signature.rs` (new)

- Moved duplicated core classifiers into lang; `fn`/`def`/`$F($$$)` legacy shapes stay **byte-identical**.
- Broader `function`/`class`/`struct`/`interface` kind tables cover kinds already emitted by `collect_pattern_nodes` for all 13 languages.
- `structural_term_signatures` preserves historical 6-key hybrid boost formats.

### `langs.rs`

- Already table-consistent for 13 languages; no dead `KindRule` rows found.
- Ruby `singleton_method`, C# `local_function_statement` / `invocation_expression`, C/C++ declarator rules retained.

---

## Core / CLI shared debt (adapted playbook)

| Change | File |
|--------|------|
| Delete local `cached_pattern_signatures` / `is_pattern_identifier` / `is_pattern_path`; import lang | `ast-sgrep-core/src/pattern.rs` |
| `structural_index_pass` → `structural_term_signatures` | `ast-sgrep-core/src/search/mod.rs` |
| `wait_child_deadline` for ast-grep probe | `ast-sgrep-core/src/pattern.rs` |
| Shared `hash_content`; trivia prefix tables for body-hash | `ast-sgrep-core/src/index.rs` |
| `resolve_output_format`; `raw_command_name` includes `keyword` | `ast-sgrep-cli/src/lib.rs` |

Full CLI god-file module split deferred on this tip (different surface than quality-batch); lang density was the mandated dig-deep target.

---

## Signature byte-identity checks

- Lang unit tests in `signature::tests::*` pin `decl:` / `call-name:` / `kind:` formats and structural term keys.
- Core test `pattern::tests::cached_signatures_delegate_to_lang_byte_identically` pins:
  - `fn parse_low($$$)` → `["decl:fn:parse_low"]`
  - `fn $NAME($$$)` → `["kind:function_item"]`
  - `$F($$$)` → `["kind:call_expression", "kind:call"]`
- Hybrid keys unchanged: `call-name:` / `call:` / `decl:fn:` / `decl:def:` / `decl:function:` / bare term.

---

## Behavior invariants

- C# uses real `tree_sitter_c_sharp` (not Java); `invocation_expression` remains a call kind.
- Keyword-scoped class patterns: C#/Swift/C++/Kotlin/PHP do not cross-match class↔struct↔interface.
- Ruby `singleton_method` still extracts as Method with call ownership.
- Extraction / pattern goldens unchanged for all 13 languages.
- Native pattern query strings / path rules identical (moved as data, not rewritten).

---

## Commands run

```bash
cargo test -p ast-sgrep-lang --lib --test pattern --test extraction_goldens
# → lib: 9 passed
# → extraction_goldens: 1 passed
# → pattern: 13 passed (incl. csharp grammar, all-lang function, C#/Swift/C++/Kotlin/PHP
#    singleton kind filters, ruby singleton_method)

cargo test -p ast-sgrep-core --lib pattern::
# → 3 passed

cargo test -p ast-sgrep-core --lib search::
# → 3 passed

cargo check -p ast-sgrep-cli -p ast-sgrep-core -p ast-sgrep-lang
# → ok
```

---

## Thin-wrapper audit

| Symbol | Callers | Action |
|--------|---------|--------|
| `last_identifier_chain` | 1 (self) | deleted; call `last_identifier_in_chain` |
| `function_queries` / `class_queries` | 1 each | replaced by table lookups |
| `Default for Extractor` | zero | removed |
| extract `pub` helpers | crate-only | demoted `pub(crate)` |
| Core local signature classifiers | duplicated | deleted; use lang exports |

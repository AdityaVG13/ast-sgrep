# Epic Evidence: `ast-sgrep-difu` remaining children

**Branch:** `fix/csharp-grammar-pattern-difu-5` (PR #23)  
**Base SHA before this pass:** `aa37f45ff92ef3722b785895b9b4a32c0f0663e5`  
**Date (UTC):** 2026-08-03  

Closed earlier on this PR: `difu.1` (conformance contract), `difu.2` (Swift), `difu.5` (C#).  
This pass closes the remaining kids with hard test evidence (beads tracker not mutated; `.beads/` untouched).

---

## `ast-sgrep-difu.6` — Ruby `singleton_method` invisible to extraction

### Bug
Ruby `def self.foo` / `def obj.foo` parse as `singleton_method`, not `method`. The kind map only handled `method`, so singleton methods produced neither symbols nor caller attribution.

### Fix
- `langs.rs`: add `("singleton_method", MethodIn(RB_CLASS))`
- `extract.rs`: include `singleton_method` in `enclosing_symbol_name`
- `pattern.rs`: `singleton_method` → declaration prefix `function` (already queried)

### Evidence
Fixture `tests/fixtures/extract/ruby.rb` now includes `def self.create` with a call inside.

```text
cargo test -p ast-sgrep-lang --test pattern ruby_singleton -- --nocapture
test ruby_singleton_method_is_visible_to_extraction ... ok
test result: ok. 1 passed; 0 failed
```

Conformance (`extraction_goldens`): Ruby case requires symbol `create` (Method), call `create -> format_widget`, and pattern match on `create`.

---

## `ast-sgrep-difu.3` — C and C++ grammars + extraction + conformance

### Delivered
| Surface | C | C++ |
|---|---|---|
| Grammar | `tree-sitter-c` 0.24 | `tree-sitter-cpp` 0.23 |
| Language enum | `Language::C` (`c`) | `Language::Cpp` (`cpp`) |
| Exts | `.c`, `.h` | `.cpp/.cc/.cxx/.hpp/.hxx/.hh/.ipp` |
| Symbols | `function_definition` via declarator chain; `struct_specifier`; `enum_specifier`; `typedef` | same + `class_specifier`; methods inside class/struct |
| Imports | `#include` → `preproc_include` | same |
| Calls | `call_expression` | same; MethodInDeclarator for member functions |
| Patterns | function + struct/type queries | function + class/struct/type queries |
| Fixtures | `fixtures/extract/c.c` | `fixtures/extract/cpp.cpp` |
| Discovery | `INDEXABLE_EXTENSIONS`, `resolve_module_path`, VS Code activation | same |

New `KindRule`s: `SymDeclarator`, `MethodInDeclarator` (C/C++ names live under nested `declarator` fields, not `name`).

### Evidence
Conformance cases for `Language::C` and `Language::Cpp` in `extraction_goldens.rs` (parse fidelity, symbols, imports, callers, patterns, spans, forbid doc-only tokens).

```text
cargo test -p ast-sgrep-lang --test extraction_goldens --test pattern
test all_languages_satisfy_shared_parse_extract_and_pattern_contract ... ok
test function_pattern_matches_all_languages ... ok
test cpp_class_pattern_does_not_match_struct ... ok
```

```text
cargo test -p ast-sgrep-core indexes_ --lib
test gitignore::tests::indexes_c_cpp_kotlin_php_source_files ... ok
```

---

## `ast-sgrep-difu.4` — Kotlin and PHP grammars + extraction + conformance

### Delivered
| Surface | Kotlin | PHP |
|---|---|---|
| Grammar | `tree-sitter-kotlin-ng` 1.1 | `tree-sitter-php` 0.24 (`LANGUAGE_PHP`) |
| Language enum | `Language::Kotlin` (`kotlin`) | `Language::Php` (`php`) |
| Exts | `.kt`, `.kts` | `.php` |
| Symbols | `function_declaration`; `class_declaration` via keyword/modifier (`class`/`interface`/`enum`); `object_declaration` | `function_definition`; `method_declaration`; class/interface/enum |
| Imports | `import` → joined identifiers | `namespace_use_declaration` → qualified path |
| Calls | `call_expression` (`CallFirstNamed`) | `function_call_expression` / `member_call_expression` / `scoped_call_expression` |
| Patterns | function + class/interface filters | function + class/interface |
| Fixtures | `fixtures/extract/kotlin.kt` | `fixtures/extract/php.php` |

New `KindRule`: `SymByKeywords` (Kotlin reuses `class_declaration` for class/interface/enum).  
Pattern channel filters Kotlin class vs interface vs enum via `class_keyword_matches`.

### Evidence
```text
cargo test -p ast-sgrep-lang --test extraction_goldens --test pattern
test all_languages_satisfy_shared_parse_extract_and_pattern_contract ... ok  # includes Kotlin + PHP cases
test function_pattern_matches_all_languages ... ok
test kotlin_class_pattern_does_not_match_interface_or_enum ... ok
test php_class_pattern_does_not_match_interface ... ok
```

---

## Shared surface updates (all three kids)

- `Language::all()` now has **13** languages (was 9).
- Lockfile: `tree-sitter-c`, `tree-sitter-cpp`, `tree-sitter-kotlin-ng`, `tree-sitter-php`.
- Docs: `README.md`, `docs/how-it-works.md`, `docs/comparison.md` → 13-language surface.
- Indexing: `gitignore.rs` INDEXABLE_EXTENSIONS; `sqlite.rs` `resolve_module_path` exts; `semantic_chunk.rs` comment markers.
- Editor: `editors/vscode` activation + documentSelector for c/cpp/kotlin/php.

### Focused test gate (authoritative)

```text
$ cargo test -p ast-sgrep-lang --test pattern --test extraction_goldens
running 1 test
test all_languages_satisfy_shared_parse_extract_and_pattern_contract ... ok
test result: ok. 1 passed; 0 failed

running 13 tests
test ruby_singleton_method_is_visible_to_extraction ... ok
test function_pattern_matches_all_languages ... ok
test cpp_class_pattern_does_not_match_struct ... ok
test kotlin_class_pattern_does_not_match_interface_or_enum ... ok
test php_class_pattern_does_not_match_interface ... ok
… (13 passed)
test result: ok. 13 passed; 0 failed

$ cargo test -p ast-sgrep-core indexes_ --lib
test gitignore::tests::indexes_c_cpp_kotlin_php_source_files ... ok
test result: ok. 2 passed; 0 failed
```

Cargo: `/usr/local/cargo/bin/cargo` (1.97.1).

---

## Verdict

| Bead | Status | Hard evidence |
|---|---|---|
| `ast-sgrep-difu.6` | **SATISFIED** | singleton extraction + call ownership tests green |
| `ast-sgrep-difu.3` | **SATISFIED** | C/C++ fixtures + conformance + pattern + discovery tests green |
| `ast-sgrep-difu.4` | **SATISFIED** | Kotlin/PHP fixtures + conformance + pattern + discovery tests green |

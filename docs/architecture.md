# ast-sgrep documentation

## Overview

See [README.md](../README.md) and [PRD.md](../PRD.md) for full details.

## Crates

| Crate | Purpose |
|-------|---------|
| `ast-sgrep-lang` | tree-sitter parsers and symbol extractors |
| `ast-sgrep-core` | SQLite index, hybrid search, ranking |
| `ast-sgrep-cli` | `asgrep` binary |

## Adding a language

1. Add tree-sitter grammar dependency to `ast-sgrep-lang`
2. Implement `LanguageParser` trait
3. Register in `ParserRegistry::new()`
4. Add extension mapping in `detect_language()`

## Index schema

```sql
files(id, path, language, mtime_secs, mtime_nanos, content_hash)
lines(file_id, line_no, content)
symbols(id, file_id, name, kind, line_start, line_end, byte_start, byte_end)
callers(id, file_id, caller, callee, line_no, byte_start, byte_end)
imports(id, file_id, module_path, line_no)
lines_fts(content, file_id, line_no)  -- FTS5 virtual table
```

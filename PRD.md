# ast-sgrep PRD

Version 0.2 | Repo: ast-sgrep | CLI: asgrep | Language: Rust

## Executive summary

ast-sgrep is a standalone, polyglot, local code search engine.
Answers intent questions like "where is auth refreshed?" or "who calls process_request?"
by combining lexical search + AST symbols + call-graph neighborhood.

**Tagline:** ast-grep finds shapes. ast-sgrep finds intent with graph context.

Not a fork of ast-grep. ast-grep = structural patterns + codemods.
ast-sgrep = hybrid discovery + cross-file graph. Fully standalone.

## Goals

- Polyglot v1: Rust, TS/JS, Python, Go
- Keyword/NL queries
- Hybrid rank: lexical + defs + caller graph
- Line + JSON output
- Local only, `.asgrep/` index
- Incremental reindex
- Standalone (no agent/MCP coupling)

## Non-goals

- Codemods → use ast-grep
- Raw text primary UX → use ripgrep
- Cloud embeddings v1 → later plugin
- LSP server → v3+
- Agent token contracts → integrator's job

## CLI

```
asgrep index [ROOT]
asgrep "auth refresh" [ROOT]
asgrep --json --limit 32 "process_request" [ROOT]
asgrep "callers:main" [ROOT]
asgrep "defs:process_request" [ROOT]
asgrep "imports:" [ROOT]
asgrep status [ROOT]
asgrep reindex [ROOT]
```

Flags: `--root`, `--limit`, `--json`, `--index-path`, `--lang`

## Search passes

1. **Lexical:** BM25-like line score → `ASGREP: file:line-line: excerpt`
2. **Symbol+graph:** match symbols → `DEF`, `CALLER`, `GRAPH`
3. **Anchor:** query contains known symbol → `ANCHOR`

Fusion: sort by score, dedup, truncate (default 16).

## Ranking

- `score_lexical(rank) = 1.0 / (60 + rank + 1)`
- exact symbol term match: +5.0
- substring in symbol: +2.0
- `score_def = score_symbol*2 + 3`
- `score_caller = score_symbol*2 + 1.5`
- `score_graph = 5.0`
- `score_anchor = 6.0`

## Index store

SQLite at `.asgrep/index.db`

Tables: `files`, `lines`, `symbols`, `callers`, `imports`

Respects `.gitignore` and `.asgrepignore`. Incremental single-file reindex.

## Repo layout

```
ast-sgrep/
  Cargo.toml (workspace)
  PRD.md README.md LICENSE
  crates/ast-sgrep-core/
  crates/ast-sgrep-cli/
  crates/ast-sgrep-lang/
  tests/fixtures/
```

## Roadmap

- Phase 0: repo + PRD + cargo workspace ✅
- Phase 1 v0.1: Rust+TS, SQLite, CLI, JSON ✅
- Phase 2 v0.2: Python+Go, incremental, benchmarks ✅
- Phase 3 v0.3: false-positive tests, crates.io
- Phase 4 v0.4: pattern: delegates to ast-grep
- Phase 5 v1.0: optional embedding plugin

## License

MIT

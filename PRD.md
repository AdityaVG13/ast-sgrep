# ast-sgrep PRD

Version 1.0 | Repo: ast-sgrep | CLI: asgrep / ast-sgrep | Language: Rust

## Executive summary

ast-sgrep is a standalone, polyglot, local code search engine.
Answers intent questions like "where is auth refreshed?" or "who calls process_request?"
by combining lexical search + AST symbols + call-graph neighborhood.

**Tagline:** ast-grep finds shapes. ast-sgrep finds intent with graph context.

## v1.0 deliverables (complete)

- Polyglot: Rust, TS/JS, Python, Go
- Keyword/NL queries with RRF lexical fusion
- Hybrid rank: lexical + defs + caller graph + anchor
- `pattern:` query prefix delegating to ast-grep
- Optional local embedding plugin (`--embed`, `ast-sgrep-embed`)
- Line + JSON output
- SQLite index at `.asgrep/index.db` (or `ASGREP_INDEX_PATH`, `~/.cache/asgrep/`)
- Incremental reindex
- False-positive regression tests (0% false callers in strings/comments)
- Benchmarks (`asgrep bench`, criterion suite)
- crates.io-ready workspace crates

## CLI

```
asgrep index [ROOT]
asgrep "auth refresh" [ROOT]
asgrep --json --limit 32 "process_request" [ROOT]
asgrep --embed "auth refresh" [ROOT]
asgrep "callers:main" [ROOT]
asgrep "defs:process_request" [ROOT]
asgrep "imports:" [ROOT]
asgrep "pattern:fn $NAME()" [ROOT]
asgrep status [ROOT]
asgrep reindex [ROOT]
asgrep bench [ROOT]
```

Flags: `--root`, `--limit`, `--json`, `--index-path`, `--lang`, `--embed`

## Success metrics (v1.0)

- 4+ languages
- search <20ms target (see `asgrep bench`)
- 0% false callers in regression suite

## License

MIT

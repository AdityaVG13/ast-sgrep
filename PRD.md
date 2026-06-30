# ast-sgrep PRD

Version 1.0 | Repo: ast-sgrep | CLI: asgrep / ast-sgrep | Language: Rust

## Roadmap — all phases complete

| Phase | Scope | Status |
|-------|-------|--------|
| 0 | Repo + PRD + Cargo workspace | ✅ |
| 1 v0.1 | Rust+TS, SQLite, CLI, JSON | ✅ |
| 2 v0.2 | Python+Go, incremental, benchmarks | ✅ |
| 3 v0.3 | False-positive tests, crates.io | ✅ |
| 4 v0.4 | `pattern:` delegates to ast-grep | ✅ |
| 5 v1.0 | Optional embedding plugin (local + cloud) | ✅ |
| 6 v3+ | Full LSP server — symbols, defs, refs, call hierarchy, executeCommand | ✅ |

Phase 6: Content-Length JSON-RPC, cursor-aware symbol resolution, incremental reindex on save.

## Deliverables

- **Polyglot:** Rust, TS/JS, Python, Go
- **Search:** keyword/NL, RRF lexical fusion, hybrid rank (lexical + defs + callers + graph + anchor)
- **Queries:** `callers:`, `defs:`, `imports:`, `pattern:`
- **Embeddings:** local (`--embed`) + cloud (`--cloud-embed`, OpenAI-compatible API)
- **Scale:** lexical sidecar at `.asgrep/lexical.db` (`--tantivy`, auto at 1000+ files)
- **Index:** SQLite `.asgrep/index.db`, incremental, `.gitignore` + `.asgrepignore`
- **Output:** line + JSON
- **CLI:** `index`, `status`, `reindex`, `bench`
- **LSP:** full `asgrep-lsp` — see [docs/lsp.md](docs/lsp.md)
- **Tests:** 33+ unit/integration (incl. 9 LSP tests) + false-positive regression suite
- **Publish:** crates.io metadata + `scripts/publish.sh` (no CI — publish manually)

## CLI

```
asgrep index [ROOT]
asgrep "auth refresh" [ROOT]
asgrep --embed "auth refresh" [ROOT]
asgrep --cloud-embed "auth refresh" [ROOT]
asgrep --tantivy index [ROOT]
asgrep "pattern:fn $NAME()" [ROOT]
asgrep bench [ROOT]
asgrep-lsp   # LSP over stdio
```

## Success metrics

- 4+ languages ✅
- search <20ms ✅ (`asgrep bench`)
- 0% false callers in regression suite ✅

## License

MIT

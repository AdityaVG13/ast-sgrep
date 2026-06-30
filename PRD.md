# ast-sgrep PRD

Version 1.1 | Repo: ast-sgrep | CLI: asgrep / ast-sgrep | Language: Rust

## Roadmap — all phases complete

| Phase | Scope | Status |
|-------|-------|--------|
| 0 | Repo + PRD + Cargo workspace | ✅ |
| 1 v0.1 | Rust+TS, SQLite, CLI, JSON | ✅ |
| 2 v0.2 | Python+Go, incremental, benchmarks | ✅ |
| 3 v0.3 | False-positive tests, crates.io | ✅ |
| 4 v0.4 | `pattern:` delegates to ast-grep | ✅ |
| 5 v1.0 | Optional embedding plugin (local + cloud) | ✅ |
| 5.1 v1.1 | **World-class semantic (S)** — symbol chunks, concept expansion, default on | ✅ |
| 6 v3+ | Full LSP server — symbols, defs, refs, call hierarchy, executeCommand | ✅ |

Phase 6: Content-Length JSON-RPC, cursor-aware symbol resolution, incremental reindex on save.

## Deliverables

- **Polyglot:** Rust, TS/JS, Python, Go, Java, C#, Ruby (8 languages)
- **Search:** keyword/NL, RRF lexical fusion, SQL-bounded hybrid rank
- **Queries:** `callers:`, `defs:`, `imports:`, `pattern:`
- **Semantic (S):** symbol-chunk embeddings with call-graph context — **on by default**
  - Offline: code-aware semantic local (concept expansion, char n-grams)
  - Optional neural: Ollama (`--ollama-embed`) → cloud (`--cloud-embed`)
  - Disable: `--no-embed`
- **Scale:** lexical sidecar at `.asgrep/lexical.db` (`--tantivy`, auto at 1000+ files)
- **Index:** SQLite `.asgrep/index.db`, incremental, `.gitignore` + `.asgrepignore`
- **Output:** line + JSON
- **CLI:** `index`, `status`, `reindex`, `bench`
- **LSP:** full `asgrep-lsp` — see [docs/lsp.md](docs/lsp.md)
- **Tests:** semantic regression suite (zero token-overlap synonym queries) + e2e/LSP
- **Plugins:** `ast-sgrep-plugins` — GitHub/GitLab/Agent JSON (`--format`)
- **Publish:** crates.io metadata + `scripts/publish.sh` (no CI — publish manually)

## Semantic layer (v1.1)

### Index time

For each extracted symbol, build an enriched chunk:

```
symbol: auth_refresh kind: function called_by: main calls: fetch_token store_token
excerpt: fn auth_refresh() { ... }
```

Expand with code-domain concept groups (auth ↔ credential ↔ token, refresh ↔ renewal, etc.), embed via provider chain, store in `semantic_chunks`.

### Search time

Hybrid fusion includes semantic pass by default. Query `"credential renewal"` must rank `auth_refresh` without token overlap (regression-tested).

### Provider chain

1. **Cloud** — OpenAI-compatible (`ASGREP_EMBED_API_KEY`)
2. **Ollama** — local neural (`ASGREP_OLLAMA_URL`, default `nomic-embed-text`)
3. **Semantic local** — offline code-aware fallback (always available)

## CLI

```
asgrep index [ROOT]                    # semantic chunks indexed by default
asgrep "credential renewal" [ROOT]     # synonym NL query
asgrep semantic "credential renewal"   # embed-only + agent JSON with --json
asgrep --json --format agent "QUERY"   # LLM tool-calling shape
asgrep --no-embed index [ROOT]         # disable semantic
asgrep --cloud-embed index [ROOT]    # neural cloud embeddings
asgrep --ollama-embed index [ROOT]   # neural Ollama embeddings
asgrep --tantivy index [ROOT]
asgrep "pattern:fn $NAME()" [ROOT]
asgrep bench [ROOT]
asgrep-lsp   # LSP over stdio
```

## Success metrics

- 4+ languages ✅
- search <20ms ✅ (`asgrep bench`)
- 0% false callers in regression suite ✅
- zero-overlap semantic queries hit correct symbols ✅

## License

MIT

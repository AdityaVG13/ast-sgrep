# Getting started

This guide walks through install, first index, everyday queries, and common configuration. For architecture and internals, see [how-it-works.md](how-it-works.md).

## Install

```bash
# planned once the crates are published:
cargo install ast-sgrep-cli   # CLI (asgrep / ast-sgrep)
cargo install ast-sgrep-lsp   # LSP server (editor integration)
```

Requires a Rust toolchain. Until the crates are published, build from source:

```bash
git clone https://github.com/AdityaVG13/ast-sgrep
cd ast-sgrep
cargo build --release -p ast-sgrep-cli
./target/release/asgrep --help
```

## Quickstart

From a source checkout, build once and exercise the six core workflows:

```bash
cargo build --release -j1
./target/release/asgrep index .
./target/release/asgrep 'defs:auth_refresh' . --limit 3
./target/release/asgrep semantic 'credential renewal' . --limit 3
./target/release/asgrep chain 'auth_refresh' . --limit 3
./target/release/asgrep bench . --query auth_refresh --iterations 1
```

The commands above cover installation from source, incremental indexing, grammar-directed search, semantic-only retrieval, relationship traversal, and a one-iteration local benchmark smoke test. See the [query grammar](QUERY_GRAMMAR.md) for prefixes and composition, the [architecture](ARCHITECTURE.md) for data flow, and [benchmark methodology](benchmarks.md) before interpreting timing output.

## First index

From your project root:

```bash
asgrep index .
```

This creates `.asgrep/index.db` (and optionally `.asgrep/lexical.db`, `.asgrep/semantic.ivf` at scale). The index is **incremental**: unchanged files are skipped via content hash + mtime. Respects `.gitignore` and `.asgrepignore`.

Check what was indexed:

```bash
asgrep status .
```

`status` reports file count, symbol count, caller edges, embed backend/dimension, and whether an IVF sidecar is present.

Force a full re-parse (bypass hash skip):

```bash
asgrep reindex .
```

## Everyday queries

### Hybrid search (default)

```bash
asgrep "auth refresh"
asgrep "how does process_request work"
```

Combines lexical FTS, symbol name match, caller/callee graph, anchor excerpts around symbols, and semantic similarity. No prefix needed.

### Graph prefixes

| Prefix | Example | Returns |
|--------|---------|---------|
| `callers:` | `asgrep "callers:main"` | Who calls `main` |
| `defs:` | `asgrep "defs:auth_refresh"` | Where `auth_refresh` is defined |
| `imports:` | `asgrep "imports:serde"` | Import statements mentioning `serde` |
| `pattern:` | `asgrep "pattern:fn $NAME($$$)"` | Structural match via ast-grep |

### Semantic / synonym queries

Semantic search is **on by default**, no API key.

```bash
asgrep "credential renewal"          # → auth_refresh (no shared tokens)
asgrep "sanitize user input"         # → validate_input

asgrep semantic "persist access token" --json   # semantic-only pass
```

See [semantic-search.md](semantic-search.md) for how symbol chunks and concept expansion work.

## JSON output

```bash
asgrep --json "auth refresh"
asgrep --json --format agent "where is auth refreshed"
asgrep --json --format github "process_request"
asgrep --json --format gitlab "auth refresh"
```

| Format | Flag aliases | Best for |
|--------|--------------|----------|
| `native` | (default) | General automation |
| `agent` | `llm`, `ai` | LLM tool-calling with follow-up hints |
| `github` | `gh` | GitHub code-search-shaped JSON |
| `gitlab` | `gl` | GitLab code-search-shaped JSON |

Details and examples: [use-cases.md](use-cases.md).

## CLI reference

### Commands

| Command | Description |
|---------|-------------|
| `asgrep index [ROOT]` | Build or incrementally update the index |
| `asgrep reindex [ROOT]` | Force full reindex |
| `asgrep status [ROOT]` | Index statistics |
| `asgrep semantic "QUERY" [ROOT]` | Semantic-only search |
| `asgrep bench [ROOT]` | Search latency benchmark |
| `asgrep "QUERY" [ROOT]` | Hybrid search (default) |

### Important flags

| Flag | Env var | Description |
|------|---------|-------------|
| `--root` |, | Project root (default `.`) |
| `--limit` | `ASGREP_LIMIT` | Max results (default 16) |
| `--json` |, | JSON output |
| `--format` |, | `native`, `agent`, `github`, `gitlab` |
| `--no-embed` | `ASGREP_NO_EMBED=1` | Disable semantic indexing + search |
| `--tantivy` | `ASGREP_TANTIVY=1` | Lexical FTS sidecar |
| `--cloud-embed` | `ASGREP_CLOUD_EMBED=1` | Prefer cloud neural embeddings |
| `--ollama-embed` | `ASGREP_OLLAMA_EMBED=1` | Prefer Ollama embeddings |
| `--semantic-only` | `ASGREP_SEMANTIC_ONLY=1` | Force offline semantic only |
| `--ann-threshold` | `ASGREP_ANN_THRESHOLD` | Symbol count before IVF-ANN (default 2000) |
| `--lang` |, | Filter: `rust`, `typescript`, `javascript`, `python`, `go`, etc. |
| `--index-path` | `ASGREP_INDEX_PATH` | Custom index DB path |

Store index in cache instead of repo:

```bash
ASGREP_USE_CACHE=1 asgrep index .
# → ~/.cache/asgrep/
```

### Hit kinds

| Kind | Meaning |
|------|---------|
| `ASGREP` | Lexical line hit (FTS5) |
| `DEF` | Symbol definition |
| `CALLER` | Caller → callee edge |
| `GRAPH` | Graph neighborhood summary |
| `ANCHOR` | Excerpt around a matched symbol |
| `IMPORT` | Import statement |
| `PATTERN` | Structural match via ast-grep |
| `EMBED` | Semantic symbol-chunk hit |

### Example line output

```
DEF: src/main.rs: auth_refresh span=19..22 | fn auth_refresh() { ... }
CALLER: src/main.rs: main -> auth_refresh
GRAPH: src/main.rs: main calls auth_refresh
ANCHOR: src/main.rs:19-22: fn auth_refresh() { ... }
```

## Neural embedding backends (optional)

Default workflow needs **no API key**. To upgrade vectors:

```bash
# Cloud (OpenAI-compatible)
export ASGREP_EMBED_API_KEY=sk-...
asgrep --cloud-embed index .

# Ollama (e.g. nomic-embed-text)
asgrep --ollama-embed index .
# ASGREP_OLLAMA_URL=http://127.0.0.1:11434 (default)
```

Query vectors should match the backend used at index time for best results. `asgrep status` shows the stored backend and dimension.

## Large repos

| Threshold | Behavior |
|-----------|----------|
| 1000+ files | Lexical FTS sidecar auto-enabled (`--tantivy` to force) |
| 2000+ symbols | IVF-ANN with persisted `.asgrep/semantic.ivf` |

Tune ANN: `--ann-threshold N` or `ASGREP_ANN_THRESHOLD`.

## Benchmarks

On the sample fixture (5 files, 25 symbols):

```bash
asgrep bench . --iterations 100
# Index: ~0.19 ms · Avg search: ~0.29 ms (target < 20 ms)
```

## Troubleshooting

| Symptom | Check |
|---------|-------|
| No semantic hits | `asgrep status`, embed backend, chunk count; try without `--no-embed` |
| Stale results after edit | `asgrep reindex .` or re-run `index` (incremental should catch changes) |
| `pattern:` returns nothing | Install [ast-grep](https://github.com/ast-grep/ast-grep) CLI |
| Slow first search after clone | Index not built, run `asgrep index .` |
| IVF not loading | Fingerprint mismatch after reindex, sidecar rebuilds automatically |

## Next steps

- [How it works](how-it-works.md), pipeline and index schema
- [Semantic search](semantic-search.md), the S layer in depth
- [Use cases](use-cases.md), agents, LSP, CI
- [Comparison](comparison.md), vs ast-grep and ripgrep

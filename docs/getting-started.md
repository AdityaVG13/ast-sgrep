# Getting started

This guide walks through install, first index, everyday queries, and common configuration. For architecture and internals, see [how-it-works.md](how-it-works.md).

## Install

### Pi (recommended for Pi users)

```bash
pi install npm:pi-ast-sgrep
```

Open Pi in the project you want to search. The extension immediately provides the primary `asgrep` Code Mode tool, plus `asgrep_search`, `asgrep_edit`, `asgrep_index`, and `asgrep_status`; `/asgrep-doctor`, `/asgrep-status`, `/asgrep-index`, and `/asgrep-reindex`; and the `ast-sgrep` skill. The first search lazily creates the index, so no separate setup command is required. Start with:

1. `/asgrep-doctor` if native availability or configuration is uncertain.
2. `/asgrep-status` to inspect the current project.
3. Ask Pi to use `asgrep` for multi-step lookup, or `asgrep_search` with `mode: "defs"`, `"callers"`, or `"semantic"` for one-shot queries. Use `asgrep_edit` for replace/write.

No Cargo build, source checkout, PATH configuration, MCP adapter, API key, or runtime executable download is part of the Pi package path. Read the [canonical Pi package guide](pi-package.md) before configuring external embeddings, updating, rolling back, or removing the package; it also documents supported hosts, `.asgrep` retention, security, and complete troubleshooting.

### Standalone CLI/LSP from source

Requires a Rust toolchain. Until the crates are published, build from source:

```bash
git clone https://github.com/AdityaVG13/ast-sgrep
cd ast-sgrep
cargo build --release -p ast-sgrep-cli
./target/release/asgrep --help
```

## Standalone quickstart

From a source checkout, build once and exercise the six core workflows:

```bash
cargo build --release -j1
./target/release/asgrep index .
./target/release/asgrep 'defs:auth_refresh' . --limit 3
./target/release/asgrep semantic 'credential renewal' . --limit 3
./target/release/asgrep chain 'auth_refresh' . --limit 3  # graph node cap; chain seeds use top_n=1
./target/release/asgrep bench . --query auth_refresh --iterations 1
```

The commands above cover installation from source, incremental indexing, grammar-directed search, semantic-only retrieval, relationship traversal, and a one-iteration local benchmark smoke test. See the [query prefixes](QUERY_GRAMMAR.md) for mode prefixes, the [architecture](ARCHITECTURE.md) for data flow, and [benchmark methodology](benchmarks.md) before interpreting timing output.

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
| *(none)* | `asgrep "auth refresh"` | Hybrid retrieval |
| `callers:` | `asgrep "callers:main"` | Who calls `main` |
| `defs:` | `asgrep "defs:auth_refresh"` | Where `auth_refresh` is defined |
| `imports:` | `asgrep "imports:serde"` | Import statements mentioning `serde` |
| `pattern:` | `asgrep "pattern:fn $NAME($$$)"` | Structural match via native tree-sitter (ast-grep fallback only for exotic shapes) |
| `literal:` | `asgrep "literal:foo_bar"` | Exact substring |
| `regex:` | `asgrep "regex:foo.*bar"` | Line regex |
| `word:` | `asgrep "word:token"` | Word-boundary token match |

See [QUERY_GRAMMAR.md](QUERY_GRAMMAR.md) for the normative prefix table and unsupported grammar.

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

Full query mode prefixes: [QUERY_GRAMMAR.md](QUERY_GRAMMAR.md)
(`callers:`, `defs:`, `imports:`, `pattern:`, `literal:`, `regex:`, `word:`, or unprefixed hybrid).
Machine-oriented catalog: `asgrep capabilities --json` (clap-derived; preferred for agents).

### Commands

| Command | Description |
|---------|-------------|
| `asgrep "QUERY" [ROOT]` | Hybrid search (default when no subcommand) |
| `asgrep index [ROOT]` | Build or incrementally update the index |
| `asgrep reindex [ROOT]` | Force full reindex |
| `asgrep status [ROOT]` | Index statistics |
| `asgrep semantic "QUERY" [ROOT]` | Semantic-only search |
| `asgrep chain "QUERY" [ROOT]` | Relationship / neighborhood expansion |
| `asgrep bench [ROOT]` | Search latency benchmark (`--query`, `--iterations`, `--suite`, `--fixture`, `--queries-file`, `--skip-index`) |
| `asgrep watch [ROOT]` | Incremental reindex on save (`--debounce-ms`) |
| `asgrep eval` | Gold / A/B evaluation harness |
| `asgrep capabilities` | Machine-readable command/flag catalog (`--json`) |
| `asgrep version` | Version (`--json`) |
| `asgrep robot-docs` | Agent-oriented docs / guides |
| `asgrep doctor [ROOT]` | Environment / index triage (`--robot-triage`) |

### Important flags

| Flag | Env var | Description |
|------|---------|-------------|
| `--root` | | Project root (default `.`; also positional `ROOT` on many commands) |
| `--limit` | `ASGREP_LIMIT` | Max results (default 16; hard-capped) |
| `--json` | | JSON output on stdout |
| `--format` | | `native`, `agent` (`llm`/`ai`), `github` (`gh`), `gitlab` (`gl`), `agent-capsule` |
| `--excerpt-lines` | | Extra excerpt lines in structured formats (capped) |
| `--no-embed` | `ASGREP_NO_EMBED=1` | Disable semantic indexing + search |
| `--tantivy` | `ASGREP_TANTIVY=1` | Force secondary FTS5 lexical DB (`.asgrep/lexical.db`; flag name is historical) |
| `--cloud-embed` | `ASGREP_CLOUD_EMBED=1` | Prefer cloud neural embeddings |
| `--ollama-embed` | `ASGREP_OLLAMA_EMBED=1` | Prefer Ollama embeddings |
| `--neural-embed` | `ASGREP_NEURAL_EMBED=1` | Prefer local neural embeddings (feature-gated) |
| `--semantic-only` | `ASGREP_SEMANTIC_ONLY=1` | Force offline semantic only |
| `--ann-threshold` | `ASGREP_ANN_THRESHOLD` | Symbol count before IVF-ANN (default 2000) |
| `--ann-probes` | `ASGREP_ANN_PROBES` | IVF clusters to probe |
| `--rerank` | `ASGREP_RERANK` | Local cross-encoder rerank (feature-gated) |
| `--rerank-top-k` | `ASGREP_RERANK_TOP_K` | Rerank candidate pool (default 20) |
| `--lang` | | Filter: `rust`, `typescript`, `javascript`, `python`, `go`, … |
| `--index-path` | `ASGREP_INDEX_PATH` | Custom index DB path (**privileged sink**; pin disables gen reindex) |

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
| `PATTERN` | Structural match (native tree-sitter; optional ast-grep fallback) |
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
| 1000+ files | Secondary FTS5 lexical DB auto-enabled (`--tantivy` to force) |
| 2000+ symbols | IVF-ANN with persisted `.asgrep/semantic.ivf` |

Tune ANN: `--ann-threshold N` or `ASGREP_ANN_THRESHOLD`.

## Benchmarks

On the sample fixture (tiny corpus; informational only — not a CI-enforced product SLO):

```bash
asgrep bench . --iterations 100
# Example local medians can land under 1 ms; real indexed repos are typically tens of ms warm.
# See benchmarks/results/baselines.md for published corpus latencies (UNREPRODUCIBLE from this tree).
```

## Troubleshooting

| Symptom | Check |
|---------|-------|
| No semantic hits | `asgrep status`, embed backend, chunk count; try without `--no-embed` |
| Stale results after edit | `asgrep reindex .` or re-run `index` (incremental should catch changes) |
| `pattern:` returns nothing | Prefer simpler native shapes; optional [ast-grep](https://github.com/ast-grep/ast-grep) CLI only for exotic fallbacks |
| Slow first search after clone | Index not built, run `asgrep index .` |
| IVF not loading | Fingerprint mismatch after reindex, sidecar rebuilds automatically |

## Next steps

- [How it works](how-it-works.md), pipeline and index schema
- [Semantic search](semantic-search.md), the S layer in depth
- [Use cases](use-cases.md), agents, LSP, CI
- [Comparison](comparison.md), vs ast-grep and ripgrep

# Architecture

ast-sgrep is a persistent, hybrid retrieval system. Indexing turns source files into lexical, structural, graph, and semantic representations; querying runs only the channels relevant to the parsed request, fuses their candidates, reranks them, and formats the result for a human or agent.

```text
source tree
  -> language extraction
  -> SQLite index + optional sidecars
  -> query prefixes / intent routing
  -> retrieval passes
  -> fusion + reranking
  -> CLI, JSON, MCP, or LSP output
```

## Workspace crates

| Crate | Responsibility |
|---|---|
| `ast-sgrep-cli` | `asgrep` / `ast-sgrep` binaries, command dispatch, output formats, benchmark command, agent capabilities, robot docs, and supervisor integration |
| `ast-sgrep-core` | Index orchestration, SQLite storage, query parsing, retrieval passes, ranking, semantic chunks/ANN, and result models |
| `ast-sgrep-lang` | Language-aware parsing and extraction of symbols, calls, imports, and pattern nodes |
| `ast-sgrep-embed` | Embedding providers and the always-available offline semantic backend |
| `ast-sgrep-mcp` | stdio MCP server (transport only; never linked to Code Mode) |
| `ast-sgrep-codemode` | Code Mode catalog/session/plan + host adapters (sibling to MCP; Pi JS sandbox is the agent executor) |
| `ast-sgrep-lsp` | Language Server Protocol navigation surfaces |
| `ast-sgrep-plugins` | Platform/output integrations |
| `ast-sgrep-testkit` | Shared fixtures and helpers for integration tests |

The dependency direction is intentionally toward `ast-sgrep-core`: front ends translate their protocol into core operations instead of reimplementing retrieval.

## Persistent index

The default index is `<root>/.asgrep/index.db`. File updates are incremental and stored transactionally. The schema separates source facts from derived acceleration structures:

| Structure | Purpose |
|---|---|
| metadata and files | Schema/index metadata plus file identity and change detection |
| lines and lexical FTS | Searchable source lines and token-oriented lexical lookup |
| `lines_trigram` | Trigram candidate lookup for substring/fuzzy lexical paths |
| symbols, callers, imports | Definitions and directed code-relationship edges |
| `pattern_nodes` | Normalized AST node facts used by structural/pattern retrieval |
| `semantic_chunks` and embeddings | Symbol-level semantic documents and their vectors |
| `embed_cache` | Reusable embedding results, avoiding recomputation for unchanged content |

Large semantic indexes may also persist `.asgrep/semantic.ivf`, an IVF approximate-nearest-neighbor sidecar. The IVF file accelerates vector candidate selection; SQLite remains the source of indexed symbol/chunk metadata. An optional **secondary SQLite FTS5** lexical database (`.asgrep/lexical.db`, historically flagged `--tantivy` / `ASGREP_TANTIVY`) can be enabled for larger repositories — there is no Tantivy crate dependency. Sidecars are derived data and are tied to the index configuration, not independent sources of truth.

### Indexing flow

1. Walk the root while applying ignore and skip rules.
2. Detect changed and removed files.
3. Use `ast-sgrep-lang` to extract symbols, caller/callee edges, imports, and pattern nodes.
4. Upsert source and derived facts into SQLite.
5. Build enriched symbol chunks, consult `embed_cache`, and write semantic vectors when embeddings are enabled.
6. Build or refresh IVF / secondary FTS5 (`lexical.db`) sidecars when their configured thresholds require them.

## Query and search pipeline

The [query prefixes](QUERY_GRAMMAR.md) doc is the public routing contract. Prefixes such as `defs:`, `callers:`, and `pattern:` select a mode; the `semantic` command explicitly isolates semantic retrieval; an unprefixed query uses hybrid retrieval. There is no composable `AND` / `path:` / `lang:` clause grammar.

### Candidate passes

The core search module owns independent passes for literal/BMH scanning, regex, lexical FTS, symbol/graph lookup, structural modes, and semantic embedding retrieval. Mode dispatch avoids running unrelated work—for example, a definition lookup can favor symbol facts, while a semantic-only command does not pretend to be a lexical query.

### Fusion

Each pass emits typed hits with source location, provenance, and a channel score. Fusion deduplicates overlapping locations and combines ranked candidate lists. Lexical term lists use reciprocal-rank fusion (RRF); structural, graph, and semantic evidence then enter the shared ranking model with explicit channel weights rather than being compared as if their raw scores had identical units.

### Reranking and output

Reranking applies query intent and code-aware evidence such as symbol identity, definition/call relationships, path/name matches, and semantic similarity. The final stage enforces limits, attaches previews/excerpts, and renders human text or a selected JSON format. This boundary keeps retrieval deterministic independently of CLI presentation.

### Supervisor boundary

On Unix the CLI supervisor wraps the worker process and can enforce a wall-time duty cycle via `ASGREP_CPU_LIMIT_PERCENT` (SIGSTOP/CONT in a 10 ms window). It does not maintain a second index or a separate ranking implementation. Retrieval stays in `ast-sgrep-core`; the supervisor only bounds runnable time and process lifecycle.

## Agent surfaces

Two self-describing CLI surfaces let an agent discover the live contract instead of relying on copied prompt text:

```bash
./target/release/asgrep capabilities --json
./target/release/asgrep robot-docs guide
```

- `capabilities --json` emits machine-readable commands, flags, output formats, and exit-code meanings for the current binary.
- `robot-docs guide` prints the operational guide intended for tool-using agents.
- Read-side commands accept `--json`; `--format agent` and `--format agent-capsule` provide agent-oriented result shapes.
- `ast-sgrep-mcp` exposes search over MCP (transport). `ast-sgrep-lsp` maps indexed navigation to editor protocol operations.
- Code Mode is a separate execution model: Pi's `asgrep_codemode` JS sandbox (and the `ast-sgrep-codemode` Rust catalog/session) let the model write code that orchestrates parallel search. MCP and Code Mode must not depend on each other. See [codemode.md](codemode.md).

Protocol consumers should discover capabilities first, treat stdout JSON as data, and interpret documented exit codes rather than scraping human-readable lines.

## Further reading

- [Query prefixes](QUERY_GRAMMAR.md)
- [Semantic search](semantic-search.md)
- [Index and retrieval walkthrough](how-it-works.md)
- [MCP setup](mcp.md)
- [Benchmark methodology](benchmarks.md)
- [Head-to-head table](../benchmarks/results/head-to-head.md), [bake-off](../benchmarks/results/bakeoff.md), and [known losses](../benchmarks/results/losses.md)

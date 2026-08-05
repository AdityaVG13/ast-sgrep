# pi-ast-sgrep

Native Code Mode, structural, graph, and semantic code search for [Pi](https://github.com/earendil-works/pi).

[![pi-ast-sgrep: native code search inside Pi](https://cdn.jsdelivr.net/npm/pi-ast-sgrep/assets/preview.png)](https://pi.dev/packages/pi-ast-sgrep?name=pi-ast-sgrep)

`pi-ast-sgrep` gives Pi a warm, project-aware search engine for understanding code. It finds behavior by intent, resolves definitions and callers, traces relationships, matches syntax-aware patterns, and searches local semantic embeddings. The primary `asgrep_codemode` tool lets Pi compose several searches in one JavaScript program instead of spending one model round trip per lookup.

## Install

```bash
pi install npm:pi-ast-sgrep
```

Restart Pi if the current session does not load newly installed package resources. For a project-local installation, add `-l`:

```bash
pi install -l npm:pi-ast-sgrep
```

No Rust toolchain or separate MCP server is required. The npm package selects the native binary and in-process addon for the current supported platform.

## What this package adds

| Resource | Purpose |
|---|---|
| `asgrep_codemode` | Primary tool. Run a bounded JavaScript program that composes typed `asgrep.*` calls. |
| `asgrep_search` | Run one natural, structural, symbol, graph, semantic, word, literal, or regex lookup. |
| `asgrep_index` | Create, refresh, or explicitly rebuild the current project index. |
| `asgrep_status` | Inspect the selected root, index, backend, counts, and capabilities. |
| `ast-sgrep` skill | Teach Pi when and how to use Code Mode, direct search, or exact-text search. |

The package also registers `/asgrep-doctor`, `/asgrep-status`, `/asgrep-index`, and `/asgrep-reindex`.

## Start with Code Mode

Ask Pi:

> Use ast-sgrep Code Mode to find where access tokens are refreshed, trace the top result's callers, and return only the relevant files, symbols, and lines.

Pi can make one `asgrep_codemode` call like this:

```json
{
  "code": "async () => {\n  const seed = await asgrep.search({ query: 'where are access tokens refreshed?', limit: 5 });\n  const symbol = seed.hits?.[0]?.symbol;\n  if (!symbol) return { seed };\n  const [defs, callers] = await Promise.all([\n    asgrep.defs({ symbol, limit: 5 }),\n    asgrep.callers({ symbol, limit: 10 }),\n  ]);\n  return { symbol, defs: defs.hits, callers: callers.hits };\n}"
}
```

This workflow narrows the first result, runs independent follow-up searches together, and returns a small shaped value to the model.

### Code Mode API

The sandbox exposes these asynchronous methods:

| Method | Use |
|---|---|
| `asgrep.search({ query, limit?, excerptLines? })` | Search by intent, symbol, or a prefixed structural query. |
| `asgrep.semantic({ query, limit?, excerptLines? })` | Search local semantic embeddings directly. |
| `asgrep.defs({ symbol, limit? })` | Find definitions for one symbol. |
| `asgrep.callers({ symbol, limit? })` | Find call sites for one symbol. |
| `asgrep.imports({ module, limit? })` | Find imports of one module. |
| `asgrep.chain({ query, limit? })` | Trace related symbols and graph edges. |
| `asgrep.indexStatus()` | Read index and backend state. |
| `asgrep.indexRepo({ force? })` | Create, refresh, or rebuild the index. |
| `asgrep.catalogSearch({ query })` | Discover less common ast-sgrep operations. |
| `asgrep.catalogDescribe({ name })` | Read the schema for a discovered operation. |

Use `Promise.all` for independent calls. Filter, map, sort, and slice intermediate values in JavaScript. Return only the evidence needed for the next reasoning step.

Code Mode includes `Promise`, `JSON`, arrays, objects, `Map`, `Set`, and `Math`. It does not expose `require`, `process`, `fetch`, or filesystem APIs. This is a capability boundary for generated code, not an OS sandbox for the installed Pi package.

## Direct one-shot search

Use `asgrep_search` when one lookup is enough:

```json
{"query":"auth_refresh","mode":"defs","limit":8}
{"query":"auth_refresh","mode":"callers","limit":8}
{"query":"where are credentials renewed?","mode":"semantic","limit":8}
{"query":"$CLIENT.post($URL)","mode":"pattern","limit":8}
```

Available modes:

| Mode | Best for |
|---|---|
| `natural` | Intent or mixed code-language queries when exact spelling is unknown. |
| `pattern` | Syntax-aware ast-sgrep patterns with metavariables. |
| `defs`, `callers`, `imports` | Symbol and module navigation. |
| `chain` | Multi-hop relationship tracing. |
| `semantic` | Meaning-based local vector search. |
| `word`, `literal`, `regex` | Explicit text-oriented matching. |

`limit` accepts 1–100 and defaults to 8. Excerpts are disabled by default; set `excerptLines` only after narrowing the result set.

## Why Code Mode is fast

Official platform packages include `ast-sgrep-codemode.node`. The extension loads an in-process native `CodeModeSession` and keeps one warm Searcher per project root for Code Mode, direct tools, and freshness checks. Normal searches do not spawn a CLI process.

Independent calls created in the same JavaScript turn are coalesced into a batch. `Promise.all` can therefore fan out several lookups while the model makes one tool call. If the native addon is unavailable, the bundled CLI service is a degraded fallback; `/asgrep-doctor` reports the active backend.

Code Mode and `ast-sgrep-mcp` are separate front ends over the same Rust search core. Pi uses Code Mode directly and does not use an MCP adapter.

## Indexing and freshness

Start Pi in the repository you want to search. The first search validates the index and lazily creates `<project-root>/.asgrep/` when needed. Run `/asgrep-index` if you want to build it before searching.

After a successful Pi `write` or `edit`, the extension marks the affected path dirty and refreshes it before the next search. Concurrent searches for the same root share one in-flight refresh. Use `/asgrep-reindex` only for an incompatible or corrupt index, or when you explicitly need a full rebuild.

The package never edits `.gitignore`. Add this entry yourself if index data must stay untracked:

```gitignore
.asgrep/
```

## Commands

| Command | Action |
|---|---|
| `/asgrep-doctor` | Check package versions, native runtime, protocol, index, and project settings. |
| `/asgrep-status` | Show the current root and index state. |
| `/asgrep-index` | Create or incrementally refresh the index. |
| `/asgrep-reindex` | Build and atomically replace the index from scratch. |

These commands take no arguments.

## Requirements

- Node.js `>=22.19.0`.
- Pi currently tested with `@earendil-works/pi-coding-agent >=0.80.6 <1`.
- macOS arm64 or x64, glibc Linux arm64 or x64, or Windows x64.

Alpine/musl Linux, Windows arm64, and other hosts are not packaged. The package does not compile Rust, search `PATH`, or download executables at runtime. On an unsupported host, run `/asgrep-doctor` for the exact platform error.

The extension, `ast-sgrep` launcher, platform package, native addon, and embedded CLI are exact-version matched. Update or reinstall the complete package if doctor reports a version or protocol mismatch.

## Local by default

The default semantic backend works offline. It needs no credential, sends no telemetry, and downloads no model on first use. Search data stays under the project's `.asgrep/` directory.

External cloud, Ollama, and neural embedding providers are optional. If you enable one, source text and queries needed for embeddings can be sent to that provider. Its credential, retention, and privacy rules then apply.

Pi packages are trusted code. Installation grants this JavaScript extension and its native code the permissions of the OS user running Pi. Project-root confinement is a package policy, not an operating-system security boundary.

## Configuration

Defaults are a 30-second operation timeout, 4 MiB output limit, and 30-second freshness interval. Supported environment settings are:

| Setting | Purpose |
|---|---|
| `ASGREP_ROOT` | Select the project root. |
| `ASGREP_TIMEOUT_MS` | Set the native operation timeout. |
| `ASGREP_MAX_OUTPUT_BYTES` | Bound native output. |
| `ASGREP_REFRESH_INTERVAL_MS` | Set the idle freshness-check interval. |
| `ASGREP_BIN` | Override the packaged binary for development. |

Explicit project configuration can opt into `allowOutsideProject`; global settings and environment variables cannot relax the default project boundary. See the [complete package guide](https://github.com/AdityaVG13/ast-sgrep/blob/main/docs/pi-package.md) for schema and precedence details.

## Update, rollback, or remove

```bash
pi update npm:pi-ast-sgrep
pi remove npm:pi-ast-sgrep
```

Removal preserves each project's `.asgrep/` data for reinstall or rollback. Delete that directory separately only when you no longer need the index.

To roll back, install one prior version as a matched unit:

```bash
pi remove npm:pi-ast-sgrep
pi install npm:pi-ast-sgrep@<previous-version>
```

Then run `/asgrep-doctor`. Compatible updates reuse validated data. Incompatible formats rebuild atomically and preserve recoverable prior data when a rebuild fails.

## More documentation

- [Complete Pi package guide](https://github.com/AdityaVG13/ast-sgrep/blob/main/docs/pi-package.md)
- [Code Mode architecture and performance](https://github.com/AdityaVG13/ast-sgrep/blob/main/docs/codemode.md)
- [Query grammar](https://github.com/AdityaVG13/ast-sgrep/blob/main/docs/QUERY_GRAMMAR.md)
- [Release provenance](https://github.com/AdityaVG13/ast-sgrep/blob/main/docs/RELEASING.md)

MIT

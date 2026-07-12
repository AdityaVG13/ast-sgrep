# Use cases

ast-sgrep is built for **navigation**, **intent queries**, and **machine-readable context**, terminal workflows, editors, CI, and AI agents.

## AI agents and LLM pipelines

### Why agents use ast-sgrep

Agents need **ranked, structured hits** with enough context to choose the next tool call, not 500 raw grep lines. ast-sgrep returns symbol names, excerpts, caller/callee hints, and semantic scores in one JSON payload.

### Quick start

```bash
asgrep index .
asgrep --json --format agent "where is auth refreshed"
asgrep semantic "credential renewal" --json
```

### Agent JSON shape

```json
{
  "provider": "ast-sgrep",
  "version": "1.1.0-alpha",
  "query": "credential renewal",
  "hit_count": 3,
  "has_semantic_hits": true,
  "stack_hint": "Use ast-sgrep for intent/navigation; ast-grep for patterns; ripgrep for grep.",
  "suggested_next": [
    "asgrep semantic \"credential renewal\"",
    "defs:auth_refresh",
    "callers:auth_refresh"
  ],
  "hits": [{
    "kind": "embed",
    "semantic": true,
    "score": 3.42,
    "file": "src/main.rs",
    "lines": { "start": 19, "end": 22 },
    "symbol": "auth_refresh",
    "excerpt": "fn auth_refresh() { ... }",
    "follow_up_queries": ["defs:auth_refresh", "callers:auth_refresh"]
  }]
}
```

Each hit includes `follow_up_queries` so agents can drill into defs/callers without guessing prefix syntax.

### Recommended agent loop

1. `asgrep index .`, build persistent index (once per repo revision)
2. `asgrep --json --format agent "<user intent>"`, ranked hits with follow-ups
3. For each symbol: `defs:` and `callers:` queries
4. Structural rewrites: `pattern:` → ast-grep
5. Logs / unindexed files: ripgrep

### Tool stack for LLM pipelines

| Task | Tool | Example |
|------|------|---------|
| Natural language / synonyms | **asgrep** | `asgrep semantic "persist access token"` |
| Symbol definitions | **asgrep** | `asgrep "defs:process_request"` |
| Caller graph | **asgrep** | `asgrep "callers:main"` |
| Structural patterns | **ast-grep** | `asgrep "pattern:fn $NAME($$$)"` |
| Raw text / logs | **ripgrep** | `rg "ERROR" logs/` |

### No API key required

Default offline semantic path is fully functional. Cloud and Ollama are optional upgrades, see [semantic-search.md](semantic-search.md).

---

## LSP, editor integration

### Install

```bash
cargo install --path crates/ast-sgrep-lsp  # from a cloned checkout
```

### Editor configuration

```json
{
  "asgrep-lsp": {
    "command": "asgrep-lsp",
    "transport": "stdio",
    "initializationOptions": {
      "asgrep": {
        "cloudEmbed": false,
        "ollamaEmbed": false,
        "semanticOnly": false,
        "annThreshold": 2000,
        "embedBackend": "auto"
      }
    }
  }
}
```

Settings may be nested under `"asgrep"` or at the top level of `initializationOptions`.

### `initializationOptions`

| Key | Type | Description |
|-----|------|-------------|
| `noEmbed` | bool | Disable semantic indexing and search |
| `cloudEmbed` | bool | Prefer cloud neural embeddings |
| `ollamaEmbed` | bool | Prefer Ollama embeddings |
| `semanticOnly` | bool | Offline semantic only |
| `annThreshold` | number | Symbol count before IVF-ANN (default 2000) |
| `embedBackend` | string | `auto`, `cloud`, `ollama`, `semantic` |
| `indexPath` | string | Custom `index.db` path |

### Capabilities

| LSP method | Feature |
|------------|---------|
| `workspace/symbol` | Hybrid search across workspace |
| `textDocument/documentSymbol` | AST symbols per file |
| `textDocument/definition` | Go-to-definition at cursor |
| `textDocument/references` | References + callers |
| `callHierarchy/prepareCallHierarchy` | Symbol at cursor |
| `callHierarchy/incomingCalls` | Who calls this |
| `callHierarchy/outgoingCalls` | What this calls |
| `workspace/executeCommand` | Custom asgrep commands |
| `textDocument/didSave` | Incremental reindex on save |
| `textDocument/didChange` | Index unsaved buffer (full-sync) |

### Execute commands

```json
{ "command": "asgrep.search", "arguments": ["auth refresh"] }
{ "command": "asgrep.search.semantic", "arguments": ["credential renewal"] }
{ "command": "asgrep.callers", "arguments": ["process_request"] }
{ "command": "asgrep.defs", "arguments": ["main"] }
{ "command": "asgrep.reindex", "arguments": [] }
```

### Semantic metadata in workspace symbols

```json
{
  "name": "auth_refresh",
  "kind": 15,
  "detail": "semantic · score 3.42",
  "containerName": "src/main.rs",
  "data": {
    "asgrepKind": "embed",
    "score": 3.42,
    "excerpt": "fn auth_refresh() { ... }",
    "semantic": true
  }
}
```

Kind `15` (`String`) marks semantic hits.

### Protocol notes

- JSON-RPC 2.0 over stdio, Content-Length framing (50 MB max)
- Non-blocking `initialize`: workspace index on background thread
- Index updates on save and full-buffer `didChange`

---

## JSON output plugins

`ast-sgrep-plugins` adapts search results for CI, platforms, and agents.

### CLI

```bash
asgrep --json "auth refresh"                    # native
asgrep --json --format agent "credential renewal"
asgrep --json --format github "process_request"
asgrep --json --format gitlab "auth refresh"
asgrep semantic "credential renewal" --json     # semantic-only, agent default
```

### Formats

| Format | Flag aliases | Top-level keys |
|--------|--------------|----------------|
| Native | `native` (default) | `query`, `limit`, `hits` |
| Agent | `agent`, `llm`, `ai` | `hits`, `suggested_next`, `has_semantic_hits`, `stack_hint` |
| GitHub | `github`, `gh` | `total_count`, `items`, `provider` |
| GitLab | `gitlab`, `gl` | `data`, `query`, `provider` |

### Library

```rust
use ast_sgrep_core::Searcher;
use ast_sgrep_plugins::{format_response, OutputFormat};

let response = searcher.search("auth refresh")?;
let agent = format_response(&response, OutputFormat::Agent);
```

---

## CI and automation

### Index in CI (optional)

For repos where you run code intelligence in CI:

```bash
asgrep index . --no-embed          # faster, lexical + graph only
asgrep --json "security audit" > results.json
```

Full semantic in CI works offline, no API key, but adds index time.

### Platform-shaped output

Emit GitHub- or GitLab-compatible JSON for tools that expect those schemas:

```bash
asgrep --json --format github "TODO" > gh-shaped.json
```

### Benchmark gate

```bash
asgrep bench . --iterations 100
# Assert avg search < 20ms in your environment
```

---

## Human terminal workflows

| Goal | Command |
|------|---------|
| Onboard to unfamiliar repo | `asgrep index .` then `asgrep "how does routing work"` |
| Trace a bug | `asgrep "callers:handle_error"` |
| Find all defs of a symbol | `asgrep "defs:UserService"` |
| Check imports | `asgrep "imports:tokio"` |
| Compare semantic vs lexical | `asgrep "credential renewal"` vs `asgrep --no-embed "credential renewal"` |

---

## Related docs

- [Getting started](getting-started.md)
- [Semantic search](semantic-search.md)
- [Comparison](comparison.md)

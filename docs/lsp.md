# asgrep-lsp — Language Server

Full LSP implementation for ast-sgrep.

## Install

```bash
cargo install ast-sgrep-lsp
```

## Editor setup

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

### `initializationOptions` (all optional)

| Key | Type | Description |
|-----|------|-------------|
| `noEmbed` | bool | Disable semantic indexing and search |
| `cloudEmbed` | bool | Prefer OpenAI-compatible cloud embeddings |
| `ollamaEmbed` | bool | Prefer local Ollama embeddings |
| `semanticOnly` | bool | Offline code-aware semantic only |
| `annThreshold` | number | Symbol count before IVF-ANN kicks in (default 2000) |
| `embedBackend` | string | `auto`, `cloud`, `ollama`, or `semantic` |
| `indexPath` | string | Custom path to `index.db` |

Settings may be nested under `"asgrep"` or provided at the top level of `initializationOptions`.

## Capabilities

| LSP method | ast-sgrep feature |
|------------|-------------------|
| `workspace/symbol` | Hybrid search across workspace |
| `textDocument/documentSymbol` | AST symbol index per file |
| `textDocument/definition` | `defs:` query at cursor (UTF-16 aware) |
| `textDocument/references` | `callers:` + defs at cursor |
| `callHierarchy/prepareCallHierarchy` | Symbol at cursor |
| `callHierarchy/incomingCalls` | Caller graph (who calls this) |
| `callHierarchy/outgoingCalls` | Callee graph (what this calls) |
| `workspace/executeCommand` | `asgrep.search`, `asgrep.search.semantic`, `asgrep.reindex`, `asgrep.callers`, `asgrep.defs` |
| `textDocument/didSave` | Incremental single-file reindex from disk |
| `textDocument/didChange` | Index unsaved buffer content (full-sync) |

## Protocol

- JSON-RPC 2.0 over stdio
- **Content-Length** framing (50 MB max message size)
- **Non-blocking** `initialize`: workspace index runs on a background thread
- Index updated on save and on full-buffer `didChange` events

## Execute commands

```json
{ "command": "asgrep.search", "arguments": ["auth refresh"] }
{ "command": "asgrep.search.semantic", "arguments": ["credential renewal"] }
{ "command": "asgrep.callers", "arguments": ["process_request"] }
{ "command": "asgrep.defs", "arguments": ["main"] }
{ "command": "asgrep.reindex", "arguments": [] }
```

### Workspace symbol semantic metadata

`workspace/symbol` results include `detail` and `data` for editor extensions:

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

Kind `15` (`String`) marks semantic similarity hits; other hit kinds use function/method kinds.

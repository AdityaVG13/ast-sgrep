# asgrep-lsp — Language Server

Phase 6 full LSP implementation for ast-sgrep.

## Install

```bash
cargo install ast-sgrep-lsp
```

## Editor setup

```json
{
  "asgrep-lsp": {
    "command": "asgrep-lsp",
    "transport": "stdio"
  }
}
```

## Capabilities

| LSP method | ast-sgrep feature |
|------------|-------------------|
| `workspace/symbol` | Hybrid search across workspace |
| `textDocument/documentSymbol` | AST symbol index per file |
| `textDocument/definition` | `defs:` query at cursor |
| `textDocument/references` | `callers:` + defs at cursor |
| `callHierarchy/prepareCallHierarchy` | Symbol at cursor |
| `callHierarchy/incomingCalls` | Caller graph (who calls this) |
| `callHierarchy/outgoingCalls` | Callee graph (what this calls) |
| `workspace/executeCommand` | `asgrep.search`, `asgrep.reindex`, `asgrep.callers`, `asgrep.defs` |
| `textDocument/didSave` | Incremental single-file reindex |

## Protocol

- JSON-RPC 2.0 over stdio
- **Content-Length** framing (spec-compliant)
- Index built on `initialize`, updated on save

## Execute commands

```json
{ "command": "asgrep.search", "arguments": ["auth refresh"] }
{ "command": "asgrep.callers", "arguments": ["process_request"] }
{ "command": "asgrep.defs", "arguments": ["main"] }
{ "command": "asgrep.reindex", "arguments": [] }
```

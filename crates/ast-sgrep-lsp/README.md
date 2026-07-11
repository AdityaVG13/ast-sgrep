# ast-sgrep LSP

`asgrep-lsp` exposes the ast-sgrep index to editors over Language Server Protocol 3.x JSON-RPC on standard input/output.

## Run

```sh
asgrep-lsp --stdio
```

The client should set the workspace root with `workspaceFolders` or `rootUri`. Indexing starts after `initialize`; requests that need the index wait for the initial indexing pass, while `workspace/symbol` returns an empty list until that pass is ready.

## Standard LSP capabilities

| Capability | Request | Index operation |
|---|---|---|
| Workspace symbols | `workspace/symbol` | ranked index search |
| Go to definition | `textDocument/definition` | `defs:<identifier>` |
| Find references | `textDocument/references` | `callers:<identifier>`, optionally definitions |
| Document symbols | `textDocument/documentSymbol` | symbols indexed for the document |
| Call hierarchy | `callHierarchy/*` | indexed caller/callee edges |
| Incremental sync | `didOpen`, `didChange`, `didSave` | reindex changed in-memory or on-disk content |

The server also advertises the supported `workspace/executeCommand` commands: `asgrep.search`, `asgrep.search.semantic`, `asgrep.reindex`, `asgrep.callers`, and `asgrep.defs`.

## Native search request

For clients that want complete ast-sgrep hits rather than LSP `SymbolInformation`, the server advertises `capabilities.experimental.asgrepSearchProvider` and accepts:

```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "method": "asgrep/search",
  "params": {
    "query": "callers:process_request",
    "semantic": false,
    "limit": 32
  }
}
```

`query` is required. `semantic` defaults to `false`; `limit` defaults to 32 and is clamped to 1–500. The result is the same serialized search response used by ast-sgrep core, including its `hits` array.

## Initialization options

Options may be passed directly or nested under `asgrep`:

```json
{
  "initializationOptions": {
    "asgrep": {
      "noEmbed": true,
      "indexPath": "/absolute/path/to/index.sqlite3",
      "annThreshold": 50000
    }
  }
}
```

Supported keys are `noEmbed`, `cloudEmbed`, `ollamaEmbed`, `semanticOnly`, `embedBackend`, `annThreshold`, and `indexPath`. File URIs and LSP positions use standard percent-encoding and UTF-16 character offsets.

The integration test in `tests/lsp.rs` is an executable client transcript: it launches the binary, frames JSON-RPC over stdio, initializes a fixture workspace, and verifies native search, workspace symbols, definition, references, and shutdown.

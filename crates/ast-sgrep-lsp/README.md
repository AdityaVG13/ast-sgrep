# ast-sgrep LSP

`asgrep-lsp` exposes the ast-sgrep index to editors over Language Server Protocol 3.x JSON-RPC on standard input/output.

## Run

```sh
asgrep-lsp --stdio
```

The client should set the workspace root with `workspaceFolders` or `rootUri`. Indexing starts after `initialize` on a background thread. `index_ready` becomes true only after a successful full `index_all` (that background pass or an `asgrep.reindex` / `ensure_index` call). Single-file open/change/save indexing does not flip readiness.

Index reads and writes share one **blocking** `Mutex`. While a full reindex or document sync holds the lock, other index-backed requests wait on that mutex; the server does **not** implement `try_lock`, retryable "index is currently being updated" busy errors, or an `index_hold_p99` metric. Unsaved editor buffers are retained in memory and re-applied after every full disk `index_all` so background reindex cannot clobber dirty content. Document sync failures (`didOpen` / `didChange` / `didSave`) are reported to the client with `window/showMessage` (error) rather than swallowed.

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

`query` is required. `semantic` defaults to `false`; `limit` defaults to 32, remaps `0` to the default, and is clamped to 1–1000 (ei0i-style). The result is the same serialized search response used by ast-sgrep core, including its `hits` array.

## Initialization options

Options may be passed directly or nested under `asgrep`:

```json
{
  "initializationOptions": {
    "asgrep": {
      "noEmbed": true,
      "annThreshold": 50000
    }
  }
}
```

Supported keys are `noEmbed`, `neuralEmbed`, `semanticOnly`, `embedBackend`, `annThreshold`, and `indexPath`. Concurrent `neuralEmbed` / `semanticOnly` (and `embedBackend`) collapse the same way as the CLI: Neural > Semantic > Auto. Boolean keys overlay the string backend when set. By default the LSP stores its database in the user's private `asgrep` cache, outside workspace-controlled paths. Custom `indexPath` values are rejected unless a trusted operator sets `ASGREP_ALLOW_EXTERNAL_INDEX=1`; with that opt-in, relative paths resolve under the workspace and the operator is responsible for path security. File URIs and LSP positions use standard percent-encoding and UTF-16 character offsets.

Search and index behavior is covered by the repo-root `tests/` suites via `ast-sgrep-testkit`.
# ast-sgrep for VS Code

This extension connects VS Code to the `asgrep-lsp` language server for indexed structural search and code navigation in Rust, Python, TypeScript, TSX, and Go workspaces.

## Prerequisites

Build or install `asgrep-lsp` and make it available on `PATH`:

```sh
cargo build --release-perf -p ast-sgrep-lsp
export PATH="$PWD/target/release-perf:$PATH"
```

Alternatively, set **ast-sgrep: Server Path** (`asgrep.serverPath`) to the absolute path of the binary. The optional **ast-sgrep: Index Path** (`asgrep.indexPath`) selects a specific SQLite index. Reload the VS Code window after changing either startup setting.

## Install a packaged extension

Build the VSIX, then use **Extensions: Install from VSIX...** in the Command Palette:

```sh
cd editors/vscode
npm install
npm run compile
npm run package
```

`npm run package` runs `vsce package --no-dependencies` through the local `@vscode/vsce` development dependency and writes `ast-sgrep-<version>.vsix` in this directory. VSIX files are local build artifacts and are ignored by Git.

## Use

Opening a Rust, Python, TypeScript, TSX, or Go file starts `asgrep-lsp --stdio`. Standard **Go to Definition**, **Find All References**, workspace symbol, document symbol, and call hierarchy features are provided by the server.

Run **ast-sgrep: Search Workspace** from the Command Palette for native `asgrep/search` results. Enter ordinary search text or a query such as `callers:process_request`, `defs:Router`, or `pattern:unwrap()`; choose a result to open it at the matching line.

## Develop

```sh
cd editors/vscode
npm install
npm run watch
```

Open this directory in VS Code and press **F5** to launch an Extension Development Host. For a one-shot clean build, run `npm run compile`.

The LSP crate includes a scripted, real-process JSON-RPC client smoke test. It builds and launches `asgrep-lsp`, sends `initialize`/`initialized`, exercises `asgrep/search` and navigation requests, then shuts the process down:

```sh
CARGO_BUILD_JOBS=1 cargo test -p ast-sgrep-lsp --test lsp scripted_stdio_initialize_search_symbols_definition_and_references -- --exact
```

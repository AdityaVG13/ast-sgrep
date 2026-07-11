# MCP server (asgrep-mcp)

`asgrep-mcp` exposes ast-sgrep hybrid code search to AI agents via the [Model Context Protocol](https://modelcontextprotocol.io/) over stdio.

## Install

```bash
git clone https://github.com/AdityaVG13/ast-sgrep
cd ast-sgrep
cargo install --path crates/ast-sgrep-mcp
# or from the workspace root after `cargo build --release`:
# ./target/release/asgrep-mcp
```

## Cursor / Claude Desktop

Add the server to your client's MCP config:

```json
{
  "mcpServers": {
    "ast-sgrep": {
      "command": "asgrep-mcp",
      "env": {
        "ASGREP_ROOT": "/path/to/your/repo",
        "ASGREP_LIMIT": "16"
      }
    }
  }
}
```

Environment variables:

| Variable | Purpose |
|----------|---------|
| `ASGREP_ROOT` | Project root (default: cwd) |
| `ASGREP_INDEX_PATH` | Custom `.asgrep/index.db` path |
| `ASGREP_LIMIT` | Max hits per search (default 16) |
| `ASGREP_NO_EMBED` | Set to `1` to disable semantic pass |

## Tools

### `code_search`

Hybrid search: lexical + symbols + call graph + semantic.

Arguments:

- `query` (required), e.g. `auth refresh`, `defs:auth_refresh`, `callers:process_request`
- `root` (optional), override `ASGREP_ROOT`
- `semantic_only` (optional), embed pass only
- `limit` (optional), max hits

Returns agent JSON (`follow_up_queries`, excerpts, stack hints). See [use-cases.md](use-cases.md).

### `index_status`

Index statistics: file/symbol/chunk counts, embed backend, IVF sidecar presence.

### `index_repo`

Build or incrementally update the index. Pass `force: true` for full reindex.

## Recommended agent loop

1. `index_repo` on first open (or rely on prior `asgrep index .`)
2. `code_search` with a natural-language or graph query
3. Follow `follow_up_queries` in hits (`defs:…`, `callers:…`)
4. Use `pattern:…` in the CLI for structural shapes; ast-grep metavariables still delegate to the ast-grep CLI when installed

## LSP vs MCP

| Surface | Best for |
|---------|----------|
| **MCP** (`asgrep-mcp`) | Headless agents, Cursor Cloud, Claude Desktop |
| **LSP** (`asgrep-lsp`) | In-editor defs/refs/call hierarchy |

Both use the same `.asgrep/` index.

## Capsule mode (`--format agent-capsule`)

For agent pipelines where context is the budget, capsule mode returns refs
and one-line previews instead of full excerpts -- roughly 3x smaller than
the `agent` format at the same limit, with identical ranking:

```bash
asgrep --json --format agent-capsule --limit 5 "hybrid ranking fusion" .
```

Each hit carries `file`, `symbol`, `kind`, `score`, `lines`, a `preview`
(first non-empty line, <=120 chars), and a `ref` like
`crates/core/src/search/mod.rs#L120-L132`. Bodies appear only on request:

- re-run with `--excerpt-lines N` to inline up to N lines per hit, or
- hand the `ref` span to your own file reader.

### Agent interop (any stack)

Capsule hits are meant to stay cheap: resolve only the spans you need with
your own file reader (editor API, `sed`/`nl`, MCP filesystem tools, etc.):

```bash
# Example: search, then read only the top hit span
asgrep --json --format agent-capsule 'auth refresh' .
# Each hit has file + lines.start/end + ref; open that window in your editor
# or agent file-read tool -- no special host product required.
```

This keeps the search step capsule-cheap and defers content bytes to the
reader, which can apply its own caching and token budgets.

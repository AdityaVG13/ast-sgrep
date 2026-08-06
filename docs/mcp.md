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

### `keyword_search`, `ast_search`, `semantic_search`

Three nonfused retrieval channels. Each accepts `query`, optional `root`, and optional `limit`, and returns abbreviated one-line previews plus stable `file#Lstart-Lend` node IDs. The agent chooses the granularity; MCP never auto-fuses channels.

- `keyword_search`: indexed lexical evidence only.
- `ast_search`: AST pattern evidence only.
- `semantic_search`: embedding evidence only.

`code_search` remains a deprecated compatibility alias for `keyword_search`; it no longer auto-fuses channels.

### `code_read`

Expands 1 to 20 selected node IDs into full code. Optional `context_lines` reads adjacent lines and `max_chars` sets an aggregate response budget. Reads enforce project containment, strict UTF-8, regular files, and scan bounds.

### `index_status`

Index statistics: file/symbol/chunk counts, embed backend, IVF sidecar presence.

### `index_repo`

Build or incrementally update the index. Pass `force: true` for full reindex.

## Recommended agent loop

1. `index_repo` on first open (or rely on prior `asgrep index .`).
2. Choose one of `keyword_search`, `ast_search`, or `semantic_search` with a bounded limit.
3. Inspect abbreviated previews and retain only relevant node IDs.
4. Call `code_read` for selected IDs, adding adjacent context only when needed.
5. Use the one-shot CLI when automatic fusion is explicitly desired. Structural rewrites remain delegated to ast-grep.

## LSP vs MCP

| Surface | Best for |
|---------|----------|
| **MCP** (`asgrep-mcp`) | Headless agents, Cursor Cloud, Claude Desktop |
| **LSP** (`asgrep-lsp`) | In-editor defs/refs/call hierarchy |

Both use the same `.asgrep/` index.

## Compact mode (`--format compact`)

Compact mode is the lowest-token CLI search contract. It emits one minified
JSON value, deduplicates paths, omits absent and decorative fields, preserves
rank order, and applies hard snippet ceilings:

```bash
asgrep --json --format compact \
  --snippet-tokens 96 --response-snippet-tokens 768 \
  "hybrid ranking fusion" .
```

The compact payload uses this versioned schema:

```json
{"v":1,"q":"query","n":1,"p":{"2jl...":"src/auth.rs"},"h":[["2jl...:10-42","d","t","refresh","fn refresh()"]],"b":[96,768,12],"t":0}
```

- `p` maps stable base-36 path hashes to paths. Repeated paths occur once.
- Each `h` row is `[id, kind, signal, symbol, snippet]` in rank order.
- `id` is `<path-id>:<start>-<end>`. To call `code_read`, expand it to
  `p[path-id]#L<start>-L<end>`; `code_read` retains its canonical path and
  containment validation.
- Kind codes are `x` exact, `d` definition, `c` caller, `g` graph, `a`
  anchor, `i` import, `p` pattern, and `e` embedding. Signal codes are `x`
  exact, `t` structural, and `m` semantic.
- `b` is `[per-result ceiling, response ceiling, used]`; `t` counts snippets
  cut by either ceiling. Metadata is never dropped when snippet budget is
  exhausted.

A token unit is one UTF-8 byte. This conservative, deterministic ceiling is
model-independent and cannot underestimate byte-fallback tokenizers. Limits
are bounded to 4,096 per result and 65,536 per response; zero is valid. The
fixed-query identity and 89.0% reduction evidence is recorded in
[compact output validation](validation/compact-output.md).

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

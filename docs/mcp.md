# MCP server (asgrep-mcp)

`asgrep-mcp` exposes ast-sgrep hybrid code search to AI agents via the [Model Context Protocol](https://modelcontextprotocol.io/) over stdio.


## Code Mode XOR MCP

Pick **one** agent surface per client:

- **MCP hosts** → this server (`asgrep-mcp`)
- **Pi** → Code Mode (`pi install npm:pi-ast-sgrep`) — do **not** also enable this MCP server in that Pi session

They share `ast-sgrep-core` but must not be stacked. See [codemode.md](codemode.md).
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
| `ASGREP_ROOT` | Project / workspace root (default: cwd). Tool `root` args must stay under this jail. |
| `ASGREP_INDEX_PATH` | **Privileged sink** — absolute writable DB path. Pins which file; rebuilds stay in-place (no generation swap). |
| `ASGREP_DURABILITY` | `strict` \| `balanced` \| `fast-unsafe` (MCP inherits; FastUnsafe is power-loss risky) |
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

**Concurrency and cancel (intentional limits for trusted local agents):**

- stdio MCP handles `tools/call` **sequentially** on one thread. A long
  `index_repo` blocks other tool calls until it returns.
- Concurrent `index_repo` calls share a process-wide single-flight lock; wait
  time counts toward a **soft wall deadline** (600s). The deadline is checked
  before start and after index work finishes -- it is **not** cooperative
  mid-build cancellation. If the post-mutation check fails, the error notes
  that the **index may already have committed** (caches are still invalidated).
- There is **no** `$/cancel` / `notifications/cancelled` path and no cancel
  token into `Indexer::{index_all,reindex_all}`. Clients cannot abort an
  in-flight index over the wire.
- Acceptable for single-tenant local agents (Cursor, Claude Desktop, Pi).
  Multi-tenant hosts that need preemptive cancel require a product change:
  multiplexed request read, request-scoped cancel flag, and cooperative
  checkpoints in the indexer (tracked as `ast-sgrep-d2a1.16`).

## Recommended agent loop

1. `index_repo` on first open (or rely on prior `asgrep index .`).
2. Choose one of `keyword_search`, `ast_search`, or `semantic_search` with a bounded limit.
3. Inspect abbreviated previews and retain only relevant node IDs.
4. Call `code_read` for selected IDs, adding adjacent context only when needed.
5. Use the one-shot CLI when automatic fusion is explicitly desired. Structural rewrites, multi-statement templates, and YAML rules are out of contract; use standalone ast-grep, they are not silently delegated (`DISC-pattern-native-subset`). Single-statement nested templates (`fn $N($$$) { $STMT }`, `if ($COND) { $BODY }`) are native — see `docs/structural-patterns.md`.

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
{"h":[["2jl...:10-42","d","t","refresh","fn refresh()"]],"p":{"2jl...":"src/auth.rs"},"q":"query","v":1,"zb":[96,768,12],"zn":1,"zt":0}
```

- `p` maps stable base-36 path hashes to paths. Repeated paths occur once.
- A `p` entry is either a plain path string or, when a shared directory prefix
  is worth folding, `[root_index, suffix]` into the optional `r` root table:

  ```json
  {"r":["crates/ast-sgrep-core/src/"],"p":{"2jl...":[0,"search/mod.rs"]}}
  ```

  `r` is present only when folding is strictly smaller than the verbatim table,
  measured on serialized bytes, so the encoding can never inflate a result set
  with no shared structure. Both forms can appear in one table. Use
  `ast_sgrep_plugins::resolve_compact_paths` rather than decoding by hand.
- Each `h` row is `[id, kind, signal, symbol, snippet]` in rank order.
- `id` is `<path-id>:<start>-<end>`. Pass it straight to the MCP `code_read`
  tool, which resolves path ids from the same session. Outside a session,
  expand it to `p[path-id]#L<start>-L<end>`; `code_read` retains its canonical
  path and containment validation either way.
- Kind codes are `x` exact, `d` definition, `c` caller, `g` graph, `a`
  anchor, `i` import, `p` pattern, and `e` embedding. Signal codes are `x`
  exact, `t` structural, and `m` semantic.
- `zb` is `[per-result ceiling, response ceiling, used]`, `zn` is the hit
  count, and `zt` counts snippets cut by either ceiling. Metadata is never
  dropped when snippet budget is exhausted.
- A snippet of `~` means this MCP session already sent that exact body for
  that id, so it was not sent again; `ze` counts how many were elided. Reuse
  the earlier result or call `code_read`. Elision is keyed on a content hash,
  so an edited file re-sends in full, and it is cleared by `index_repo` so it
  never spans index generations. Pass `resend_seen: true` if your client does
  not retain earlier results. `ze` appears on the MCP surface only.
- Per-call accounting is named `z*` on purpose. `serde_json` orders object
  keys alphabetically, so this keeps content keys (`h`, `p`, `q`, `v`) in a
  stable head and confines volatile numbers to a trailing block a consumer
  can strip. Repeated identical searches are byte-stable.

### Structured results and protocol revision (r2lu)

Every search tool declares an `outputSchema`, and `tools/call` returns typed
`structuredContent` alongside the minified text fallback, so a current client
parses results directly instead of reverse-engineering the compact envelope.
The two always agree: the text is the same JSON, minified.

`initialize` negotiates. A client asking for `2024-11-05` keeps it, a client
asking for `2025-11-25` gets it, and an unrecognized revision is answered with
the server's current revision. The server deliberately does not advertise
`2026-07-28`: that revision replaced this handshake lifecycle with
`server/discover`, which this stdio server does not implement.

### Misses

A search that finds nothing returns a diagnostic envelope instead of an empty
hit list, because the four causes below need four different next moves:

```json
{"h":[],"next":"drop the lang filter","q":"absent_symbol","scope":{"lang":"rust"},"tried":["lexical"],"v":1,"why":"filters_excluded_all","zn":0}
```

- `why` is one of `empty_index`, `filters_excluded_all`, `channel_unavailable`,
  or `no_match`. An empty index outranks the other explanations, and filters
  outrank a genuine absence.
- `tried` lists the channels that actually ran; `down` lists any that could not.
- `scope` echoes the effective filters, so the agent can see what excluded its
  candidates.
- `next` is exactly one actionable step, not a menu.

The miss envelope is far cheaper than the result envelope it replaces (131 vs
421 bytes against the agent format on a fixed query). The point is not only the
bytes: an unexplained empty result drives speculative retries that cost more
than the search did.

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

## Agent Plugins package

Portable skills + MCP wiring live in [`packages/agent-plugin`](../packages/agent-plugin) ([Agent Plugins 1.0](https://agent-plugins.org/)). Clients that load Agent Plugins use that directory as the plugin root; `mcp.json` launches `asgrep-mcp` on stdio.

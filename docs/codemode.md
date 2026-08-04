# Code Mode (`ast-sgrep-codemode` + Pi)

## What Code Mode is

Code Mode is a **tool-use pattern**, not a transport:

> The model writes JavaScript that calls typed methods. That code runs in a
> restricted executor, can fan out work in parallel, filter intermediates, and
> return only the shaped value the model needs.

That is the same idea as:

- [Cloudflare Code Mode](https://developers.cloudflare.com/agents/tools/codemode/) — one `codemode` tool, typed connector globals, sandbox executor
- [Anthropic programmatic tool calling](https://platform.claude.com/docs/en/agents-and-tools/tool-use/programmatic-tool-calling) — tools callable from code execution via `allowed_callers`
- [OpenAI programmatic tool calling](https://developers.openai.com/api/docs/guides/tools-programmatic-tool-calling) — JS in a V8 runtime coordinates tools

Traditional MCP/tool calling does **one model round-trip per operation**. Code Mode
moves loops, branching, filtering, and parallel fan-out into executable code.

## MCP vs Code Mode (do not link them)

```text
                 ast-sgrep-core / native asgrep binary
                    /                         \
                   /                           \
          ast-sgrep-mcp                   Code Mode
     (stdio JSON-RPC transport)     (JS sandbox execution)
     one tool call ↔ one RPC         model writes JS once
                                            │
                                            ▼
                                      asgrep.search()
                                      asgrep.chain()
                                      Promise.all([...])
                                      filter / shape
                                            │
                                            ▼
                                      final value → model
```

| | **MCP** (`asgrep-mcp`) | **Code Mode** |
|---|---|---|
| Role | Protocol transport for hosts that speak MCP | Execution model: code orchestrates search |
| Unit of work | One `tools/call` | One JS program (many calls inside) |
| Parallelism | Host/model schedules calls | `Promise.all` / loops inside the sandbox |
| Pi | Not used (`pi-mcp-adapter` forbidden) | **Primary Pi agent surface** |
| Coupling | — | **Never imports MCP; MCP never imports Code Mode** |

Both share the same retrieval base. They are sibling front ends.

## Pi: built on Code Mode

`pi-ast-sgrep` exposes **`asgrep_codemode`** as the primary tool:

```text
Model ──► asgrep_codemode({ code }) ──► Node capability sandbox
                                              │
                                              │  asgrep.search / chain / defs / …
                                              │  Promise.all → same-tick coalesce
                                              │       │
                                              │       ├─ sticky: one `codemode-serve`
                                              │       │         (warm Searcher, NDJSON)
                                              │       ├─ else N>1: one `codemode-batch`
                                              │       │       (serial warm / Auto parallel)
                                              │       └─ fallback: overlapped CLI spawns
                                              ▼
                                        shaped return + stats
```

### Amdahl note

Wall time ≈ serial + parallel_work / N.

| Serial cost (cut hard) | Parallel fraction |
|------------------------|-------------------|
| Process spawn, SQLite open, freshness once per Code Mode call | Independent searches inside `Promise.all` |

Same-tick coalesce turns N serial spawn costs into **one** batch process. Prefer
**sticky serve** (`codemode-serve`): one warm Searcher for the whole Code Mode
program (multi-wave), which removes spawn from every wave. Inside a one-shot
batch, Rust defaults to **serial warm** (shared Searcher); parallel opens only
when Auto sees ≥4 read-only calls or Parallel is forced.

Direct tools (`asgrep_search`, `asgrep_index`, `asgrep_status`) remain for simple one-shot lookups. Prefer Code Mode whenever the task needs composition, parallel lookups, or filtering before the model sees data.

Example the model writes:

```js
async () => {
  const [seed, status] = await Promise.all([
    asgrep.search({ query: "auth refresh", limit: 5 }),
    asgrep.indexStatus(),
  ]);
  const symbol = seed.hits?.[0]?.symbol;
  if (!symbol) return { seed, status };
  const graph = await asgrep.chain({ query: symbol, limit: 20 });
  return { symbol, nodes: graph.nodes?.slice?.(0, 10) ?? graph, status };
}
```

Sandbox capabilities: `asgrep.*`, `Promise`, `JSON`, arrays/objects/math. No
`require`, `process`, `fetch`, or filesystem — same trust model as the Pi package
(capability restriction, not an OS jail).

## Rust crate `ast-sgrep-codemode`

Separate library for:

- Typed tool **catalog** + JSON Schema
- Warm **`CodeModeSession`** over `ast-sgrep-core` (fast in-process dispatch for Rust hosts)
- Deterministic **JSON plan runner** (hosts without a JS sandbox)
- **Adapters** that emit Anthropic / OpenAI / Cloudflare-shaped tool defs for hosts that already provide a code-execution sandbox

It does **not** depend on `ast-sgrep-mcp`, and MCP must not depend on it.

```bash
cargo test -p ast-sgrep-codemode
```

## Layout

| Path | Role |
|------|------|
| `crates/ast-sgrep-codemode` | Rust catalog + session + plan + host adapters |
| `packages/pi/extension/src/codemode/` | JS connector + sandbox executor |
| `packages/pi/extension` tool `asgrep_codemode` | Pi primary Code Mode entry |
| `crates/ast-sgrep-mcp` | Unrelated MCP transport |

## Non-goals

- Linking MCP ↔ Code Mode
- Embedding Cloudflare Workers / V8 isolates in Rust (Pi uses Node `vm`; cloud hosts bring their own executor)
- Replacing the native binary with a JS search reimplementation

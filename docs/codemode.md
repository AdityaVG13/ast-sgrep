# Code Mode (`ast-sgrep-codemode` + Pi)

## What Code Mode is

Code Mode is a **tool-use pattern**, not a transport:

> The model writes JavaScript that calls typed methods. That code runs against
> an explicit tool surface (`asgrep.*`), can fan out work in parallel, filter
> intermediates, and return only the shaped value the model needs.

That is the same idea as:

- [Cloudflare Code Mode](https://developers.cloudflare.com/agents/tools/codemode/) — one `codemode` tool, typed connector globals
- [OpenCode CodeMode](https://github.com/anomalyco/opencode/tree/dev/packages/codemode) — orchestration over host-supplied tools (authority = tools you expose, not an OS jail)
- [Anthropic programmatic tool calling](https://platform.claude.com/docs/en/agents-and-tools/tool-use/programmatic-tool-calling) — tools callable from code execution via `allowed_callers`
- [OpenAI programmatic tool calling](https://developers.openai.com/api/docs/guides/tools-programmatic-tool-calling) — JS in a V8 runtime coordinates tools

Traditional MCP/tool calling does **one model round-trip per operation**. Code Mode
moves loops, branching, filtering, and parallel fan-out into executable code.

## MCP vs Code Mode — pick one (XOR)

**Use either Code Mode or MCP in a given client — not both.** They are sibling
front ends on the same retrieval core. Stacking them doubles tool catalogs,
duplicates index opens, and confuses the model about which surface to call.

| Client | Choose |
|--------|--------|
| **Pi** | Code Mode via `pi install npm:pi-ast-sgrep` (`asgrep` tool). Do **not** also register `asgrep-mcp`. |
| **MCP hosts** (Cursor Cloud, Claude Desktop, Agent Plugins, …) | `asgrep-mcp` / `packages/agent-plugin`. Do **not** also load the Pi Code Mode package in that same agent. |

```text
                 ast-sgrep-core / native asgrep binary
                    /                         \
                   /                           \
          ast-sgrep-mcp                   Code Mode
     (stdio JSON-RPC transport)     (JS program → asgrep.*)
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
| Parallelism | Host/model schedules calls | `Promise.all` / loops in the program |
| Pi | Not used | **Primary Pi agent surface** |
| Coupling | — | **Never imports MCP; MCP never imports Code Mode** |

## Pi: built on Code Mode

`pi-ast-sgrep` exposes **`asgrep`** as the primary tool:

```text
Model ──► asgrep({ code }) ──► same-realm AsyncFunction runner
                                              │
                                              │  asgrep.search / chain / defs / …
                                              │  Promise.all → same-tick coalesce
                                              │       │
                                              │       ├─ in-process NAPI Session
                                              │       │     (CodeModeSession → core)
                                              │       └─ degraded: CLI sticky serve
                                              ▼
                                        shaped return + stats
```

Authority is the explicit `asgrep.*` API (OpenCode-style: host omits tools it
does not expose). There is no `node:vm` / isolate sandbox — the Pi package
already has full user privileges; Code Mode is for orchestration speed, not OS
isolation.

### Amdahl note

Wall time ≈ serial + parallel_work / N.

| Serial cost (cut hard) | Parallel fraction |
|------------------------|-------------------|
| Process spawn, SQLite open, freshness once per Code Mode call | Independent searches inside `Promise.all` |

Same-tick coalesce turns N serial spawn costs into **one** batch process. Prefer
**session-scoped sticky serve** (`codemode-serve`): one warm Searcher per project
root for the whole Pi session — shared by Code Mode programs, direct tools, and
freshness checks (same idea as pi-codex-conversion's long-lived Code Mode host).
Inside a one-shot batch, Rust defaults to **serial warm**; parallel opens only
when Auto sees ≥4 read-only calls or Parallel is forced.

### Why no CLI spawn (Pi / Code Mode)

| Surface | Process model |
|---------|----------------|
| **MCP** (`asgrep-mcp`) | **In-process** — links `ast-sgrep-core`, warm `Searcher`. |
| **Pi / Code Mode** | **In-process NAPI** — `ast-sgrep-codemode-napi` loads `CodeModeSession` inside Node. Same retrieval core as MCP; no `asgrep` child on the hot path. |

Install **either** MCP **or** the Pi package (Code Mode) — siblings, not a stack. Both should feel like a native grep: warm index, zero process spawn, microseconds-to-milliseconds per lookup after the first open.

CLI `codemode-serve` remains only as a degraded fallback when the `.node` addon is missing (unsupported host). Official npm installs ship `ast-sgrep-codemode.node` inside each `@ast-sgrep/<platform>` package next to the CLI binary.

```bash
# Dev: build the in-process addon for this host
cargo build -p ast-sgrep-codemode-napi --release
npm run build:native -w pi-ast-sgrep
```

Measured on the sample fixture (release NAPI, warm): **20 searches ≈ few ms** vs **~60 ms for 5 cold CLI spawns**.

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
(tool-surface authority, not an OS jail).

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
| `packages/pi/extension/src/codemode/` | JS connector + AsyncFunction runner |
| `packages/pi/extension` tool `asgrep` | Pi primary Code Mode entry |
| `crates/ast-sgrep-mcp` | Unrelated MCP transport |

## Non-goals

- Linking MCP ↔ Code Mode
- Embedding Cloudflare Workers / V8 isolates in Rust (Pi uses Node `vm`; cloud hosts bring their own executor)
- Replacing the native binary with a JS search reimplementation

# Code Mode (`ast-sgrep-codemode` + Pi)

## What Code Mode is

Code Mode is a **tool-use pattern**, not a transport:

> The model writes JavaScript that calls typed methods. That code runs against
> an explicit tool surface (`asgrep.*`), can fan out work in parallel, filter
> intermediates, and return only the shaped value the model needs.

That is the same idea as:

- [Cloudflare Code Mode](https://developers.cloudflare.com/agents/tools/codemode/): one `codemode` tool, typed connector globals
- [OpenCode CodeMode](https://github.com/anomalyco/opencode/tree/dev/packages/codemode): orchestration over host-supplied tools (authority = tools you expose, not an OS jail)
- [Anthropic programmatic tool calling](https://platform.claude.com/docs/en/agents-and-tools/tool-use/programmatic-tool-calling): tools callable from code execution via `allowed_callers`
- [OpenAI programmatic tool calling](https://developers.openai.com/api/docs/guides/tools-programmatic-tool-calling): JS in a V8 runtime coordinates tools

Traditional MCP/tool calling does **one model round-trip per operation**. Code Mode
moves loops, branching, filtering, and parallel fan-out into executable code.

## MCP vs Code Mode: pick one (XOR)

**Use either Code Mode or MCP in a given client, not both.** They are sibling
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
                                      asgrep.find() / asgrep.read()
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
| Coupling | None | **Never imports MCP; MCP never imports Code Mode** |

## Pi: built on Code Mode

`pi-ast-sgrep` exposes **`asgrep`** as the primary tool:

```text
Model ──► asgrep({ code }) ──► single-use Worker (`node:vm` inside)
                                              │
                                              │  asgrep.search / find / read / edit
                                              │  Promise.all → same-tick coalesce
                                              │  in-context asgrep + JSON host bridge
                                              │       │
                                              │       ├─ in-process NAPI Session
                                              │       │     (CodeModeSession → core)
                                              │       └─ degraded: CLI sticky serve
                                              ▼
                                        shaped return + stats
```

The guest runs in a **single-use `worker_threads` isolate**, with a `node:vm`
context inside it. The host retains the native session; guest tool calls cross
a JSON message bridge. `asgrep` / `console` are built inside the context, which
hides `process` / `require`. Deadlines terminate the guest Worker, including
runaway microtasks, while aborting outstanding host calls. This is not an OS
jail; Node does not consider `vm` an adversarial-code security boundary.

Each program is limited to 256 host calls, bounded arguments, logs, and
serialized results. Raw memory and WebAssembly globals are unavailable.
The native Code Mode boundary also caps each encoded tool value at 1 MiB
and complete batch responses at 4 MiB.

One deadline covers freshness work and the Code Mode program. The soft wall
aborts the run's `AbortSignal`, so queued host calls and later `asgrep.*`
calls cannot keep using the pooled NAPI `Session` after timeout. Waiters that have not yet
taken the session mutex return `operation cancelled` instead of blocking the
pool. Read/search calls that already hold the mutex may finish their current
operation; `index_repo` polls the abort flag during walk/prepare and returns
`operation cancelled` without committing. The Pi freshness coordinator also
aborts that shared index when the last waiter cancels, so a timed-out search
cannot leave rayon workers running.

**Root jail (host duty):** `CodeModeSession` / NAPI tool `root` args are jailed
under the configured session workspace the same way MCP jails under
`ASGREP_ROOT` (`canonicalize` + containment; message
`escapes configured workspace`). NAPI has no separate resolver; it inherits
Session. Hosts must set Session root intentionally; this is policy confinement,
not an OS security boundary. `ASGREP_INDEX_PATH` remains a privileged sink
(see `docs/env-trust.md`).

### Amdahl note

Wall time ≈ serial + parallel_work / N.

| Serial cost (cut hard) | Parallel fraction |
|------------------------|-------------------|
| SQLite open once per session; single-use guest Worker | Independent `search`/`find`/`read` inside `Promise.all` |

Same-tick coalesce turns N serial spawn costs into **one** batch process. Prefer
**session-scoped sticky serve** (`codemode-serve`): one warm Searcher per project
root for the whole Pi session, shared by Code Mode programs, direct tools, and
freshness checks.
Inside a one-shot batch, Rust **Auto is always serial warm**. Unique search/find
is ~0.5 to 1 ms; N parallel SQLite opens are the serial wall. Force `Parallel`
only for an explicit experiment.

### Why no CLI spawn (Pi / Code Mode)

| Surface | Process model |
|---------|----------------|
| **MCP** (`asgrep-mcp`) | **In-process**: links `ast-sgrep-core`, warm `Searcher`. |
| **Pi / Code Mode** | **In-process NAPI**: `ast-sgrep-codemode-napi` loads `CodeModeSession` inside Node. Same retrieval core as MCP; no `asgrep` child on the hot path. |

Install **either** MCP **or** the Pi package (Code Mode); they are siblings, not a stack. Both should feel like a native grep: warm index, zero process spawn, microseconds-to-milliseconds per lookup after the first open.

CLI `codemode-serve` remains only as a degraded fallback when the `.node` addon is missing (unsupported host). Official npm installs ship `ast-sgrep-codemode.node` inside each `@ast-sgrep/<platform>` package next to the CLI binary.

```bash
# Dev: build the in-process addon for this host
cargo build -p ast-sgrep-codemode-napi --release
npm run build:native -w pi-ast-sgrep
```

The in-process path removes per-search process startup and reuses one open
searcher. This review did not retain a clean before/after benchmark fixture, so
no numeric speedup is claimed.

Direct tools (`asgrep_search`, `asgrep_edit`, `asgrep_read`, `asgrep_index`) remain for simple one-shot lookups. Prefer Code Mode whenever the task needs composition, parallel lookups, or filtering before the model sees data.

Returned `ref` strings are checkout-relative and can be passed straight back to
`read`, including from a subdirectory session. Path-form requests are anchored
to that session; paths already under its checkout-relative scope are not
prefixed twice. Symbol lookups retain the same scope as general searches.

Edits validate all replacements before writing and serialize within a native
session (or a canonical-root filesystem fallback queue). They are not a
multi-file filesystem transaction: I/O or reindex failures can leave changes,
and unrelated external writers are not locked. Inspect files before retrying
a failed edit; ambiguous transport failures are not automatically replayed.

Explicit Code Mode returns retain their selected data, including hit fields.
Model-visible output has an 8,000-character cap with a truncation notice; stale
index qualifications reserve space in that budget.

Example the model writes:

```js
async () => {
  const seed = await asgrep.search({ query: "auth refresh", limit: 5 });
  const hit = seed.hits?.[0];
  if (!hit) return { seed };
  const [defs, window] = await Promise.all([
    asgrep.find({ query: `defs:${hit.symbol}`, limit: 5 }),
    asgrep.read({ refs: [hit.ref] }),
  ]);
  return { symbol: hit.symbol, defs: defs.hits, window };
}
```

Runner capabilities: `asgrep.*`, `Promise`, `JSON`, arrays/objects/math. No
direct `require`, `process`, `fetch`, or filesystem globals. The configured wall
deadline terminates the guest Worker and aborts awaited host calls.
Call arguments, logs, serialized results, and the guest heap are capped.
Native work that already holds the session mutex follows the cancellation
limits above; Worker termination is not an OS jail.

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
| `packages/pi/extension/src/codemode/` | JS connector + restricted `node:vm` runner |
| `packages/pi/extension` tool `asgrep` | Pi primary Code Mode entry |
| `crates/ast-sgrep-mcp` | Unrelated MCP transport |

## Non-goals

- Linking MCP ↔ Code Mode
- Embedding Cloudflare Workers / V8 isolates in Rust (Pi uses Node `vm`; cloud hosts bring their own executor)
- Replacing the native binary with a JS search reimplementation

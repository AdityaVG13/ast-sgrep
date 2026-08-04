# Code Mode (`ast-sgrep-codemode`)

> **Status:** scaffold for review. Core session + tools + plan runner work; host
> wiring (MCP reuse, CLI subcommand, JS package) waits on approval of the phases
> below.

## Verdict

ast-sgrep did **not** previously have a Code Mode / programmatic tool-calling
crate. Agents reached search via MCP (one JSON-RPC call per tool), CLI JSON, Pi
tool wrappers, or LSP. Those surfaces force a **model round-trip per operation**.

`ast-sgrep-codemode` is the missing **execution** layer: typed tools, a warm
session, progressive discovery, and a JSON plan runner so multi-step search
logic runs in-process and returns only the shaped result.

Aligned with:

- [Cloudflare Code Mode](https://developers.cloudflare.com/agents/tools/codemode/)
- [Anthropic programmatic tool calling](https://platform.claude.com/docs/en/agents-and-tools/tool-use/programmatic-tool-calling)
- [OpenAI programmatic tool calling](https://developers.openai.com/api/docs/guides/tools-programmatic-tool-calling)

## Why a separate crate

| Surface | Role |
|---------|------|
| `ast-sgrep-core` | Index + retrieval |
| `ast-sgrep-mcp` | MCP **transport** (stdio JSON-RPC) |
| `ast-sgrep-cli` | Human/agent CLI |
| **`ast-sgrep-codemode`** | **Execution model**: compose tools, filter intermediates, emit host tool defs |

Dependency arrow stays toward core (same as MCP). MCP/CLI should eventually
*call into* Code Mode rather than duplicating Searcher cache / format logic.

## Architecture

```text
                    ┌─────────────────────────────┐
  Anthropic PTC ───►│ adapters::{anthropic,openai, │
  OpenAI PTC    ───►│   cloudflare}                │
  CF Code Mode  ───►└──────────────┬──────────────┘
                                   │ tool schemas
                                   ▼
  Model / host ──► catalog (progressive discovery)
                                   │
                    ┌──────────────▼──────────────┐
                    │ CodeModeSession             │
                    │  · warm Searcher cache      │
                    │  · call budget              │
                    │  · capsule-first defaults   │
                    └──────────────┬──────────────┘
                                   │
              ┌────────────────────┼────────────────────┐
              ▼                    ▼                    ▼
         tools::*            plan::run_plan        transforms
      search/chain/…      $step refs, return     filter/select
              │                    │
              └──────────┬─────────┘
                         ▼
                 ast-sgrep-core + plugins
```

### Tools (v0 catalog)

| Tool | Kind | Notes |
|------|------|-------|
| `search` | search | Hybrid + prefixes; default `format=capsule` |
| `semantic` | search | Embed pass only |
| `chain` | search | Call/import neighborhood (not on MCP today) |
| `defs` / `callers` / `imports` | search | Shorthand wrappers |
| `index_status` / `index_repo` | index | Lifecycle |
| `filter_hits` / `select` | transform | Keep intermediates out of the model |
| `catalog_search` / `catalog_describe` | catalog | Cloudflare-style progressive discovery |

### Plan language (local / deterministic)

Hosts without a JS sandbox run multi-step work as JSON:

```json
{
  "steps": [
    {"id": "seed", "tool": "search", "args": {"query": "auth refresh", "format": "capsule", "limit": 5}},
    {"id": "narrow", "tool": "filter_hits", "args": {"hits": "$seed", "path_contains": "src/", "limit": 3}},
    {"id": "graph", "tool": "chain", "args": {"query": "$narrow.hits.0.symbol", "max_depth": 2}},
    {"id": "out", "tool": "select", "args": {"value": "$graph", "fields": ["nodes", "edges", "node_count"]}}
  ],
  "return": "$out"
}
```

Hosted PTC (Claude/OpenAI) can instead generate JavaScript that calls the same
tools via `allowed_callers`; adapters emit those definitions.

## Approval gates

Please approve or adjust before the next phases land:

| Phase | Scope | Approve? |
|-------|--------|----------|
| **0 (this PR)** | Crate scaffold, catalog, session, plan runner, provider adapters, docs + tests | *landed for review* |
| **1** | Refactor `ast-sgrep-mcp` to dispatch through `CodeModeSession`; add `chain` + capsule to MCP | pending |
| **2** | CLI: `asgrep codemode tools\|plan\|run` + `capabilities` discovery | pending |
| **3** | Optional `packages/codemode` JS helpers for Claude/OpenAI request assembly | pending |
| **4** | Snippets library (saved plans) + tighter Cloudflare Agents SDK connector | pending |

### Non-goals (for now)

- Embedding a full JS/V8 sandbox in Rust (hosts already provide that for PTC)
- Replacing MCP or Pi; Code Mode composes with them
- Write tools beyond `index_repo` (keep approval boundary clear)

## Library usage

```rust
use ast_sgrep_codemode::{run_plan, parse_plan, CodeModeSession, SessionConfig};
use ast_sgrep_codemode::adapters::{anthropic_tools, openai_tools, cloudflare_connector};
use serde_json::json;

let mut session = CodeModeSession::new(SessionConfig {
    root: "/path/to/repo".into(),
    ..SessionConfig::default()
});

// Single tool
let hits = session.call("search", json!({
    "query": "credential renewal",
    "format": "capsule",
    "limit": 5
}))?;

// Multi-step plan (no model between steps)
let plan = parse_plan(&json!({ /* ... */ }))?;
let result = run_plan(&mut session, &plan)?;

// Emit host tool lists
let _ = anthropic_tools();
let _ = openai_tools();
let _ = cloudflare_connector();
```

## Validation

```bash
cargo test -p ast-sgrep-codemode
```

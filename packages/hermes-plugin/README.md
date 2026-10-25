# asgrep — native Hermes plugin

Eager repo-search tools inside the Hermes harness, backed by a persistent
`asgrep serve` NDJSON worker per repository root: zero spawn cost per call,
warm index, ambient freshness (the worker builds and incrementally refreshes
the index for its root automatically — the first search on an unindexed
checkout just works).

All tools are **read-only**. Registration does no indexing, no network, and no
filesystem writes. When the asgrep binary cannot be resolved, `check_fn`
removes the tools from the model's context entirely (fail-closed, no dead
schemas).

## Tools (toolset `asgrep`)

| tool | maps to | purpose |
|---|---|---|
| `asgrep_search` | `search` | fused keyword/AST search (`query`, `lang?`, `file_filter?`, `limit?`, `excerpt_lines?`, `semantic_only?`) — the same fused lane the CLI and MCP expose |
| `asgrep_read` | `read` | budgeted source windows for refs/hit ids (`refs`, `context_lines?`, `max_chars?`) |
| `asgrep_status` | `index_status` | index health for the session root |

### Query grammar (asgrep_search)

`query` accepts the full fused grammar — the schema description teaches this
to the model at assembly time:

- plain words / symbols / snake_case fragments: `parse_pattern`, `auth refresh`
- structural patterns with metavariables: `fn name($$$A)`
- channel prefixes: `defs:SYMBOL`, `callers:SYMBOL`, `imports:MODULE`,
  `word:TERM`, `pattern:PATTERN`

No separate tools for defs/callers/imports — the prefixes ride `asgrep_search`
and return the same hit shape. `excerpt_lines: N` widens hit excerpts to
include bodies (the capsule default returns previews only); `semantic_only`
forces the semantic channel when the hybrid answer is too broad.

### Refs (asgrep_read)

Canonical form `src/a.rs#L12-L40`; LSP-style `src/a.rs:12` is accepted as an
exact equivalent. Bare paths read the default bounded window.

Tool-level errors pass through verbatim — stale-schema errors arrive with
their exact `asgrep reindex <root> --json` recovery command.

## Install

```bash
# shipped path: the binary embeds the plugin files (self-contained)
asgrep install --target hermes --yes
hermes plugins enable asgrep   # the installer runs this when hermes is on PATH
```

Dev-live from this checkout (what you want while hacking on the plugin):

```bash
ln -s ~/Developer/ast-sgrep/packages/hermes-plugin ~/.hermes/plugins/asgrep
hermes plugins enable asgrep
```

New sessions only: Hermes freezes the tool list per session at assembly.
Requires `asgrep` on PATH (or set the plugin `binary` config); the worker is
`asgrep serve`, so the binary must include the `serve` subcommand.

## Config

| key | default | meaning |
|---|---|---|
| `binary` | `asgrep` | executable name on PATH, or absolute path |
| `timeout_s` | `120` | per-call timeout, including cold index builds |
| `eager` | `true` | keep the tools out of `tool_search` deferral (genome-pattern host seam; degrades to the bridged surface if the seams move) |
| `prewarm` | `true` | start the serve worker + ambient index build at plugin load, off the search path |
| `prewarm_timeout_s` | `600` | prewarm call budget (long: the worker keeps building even when the call returns) |

## Habit lane (`search_files` takeover, Lane B)

The tools above require the model to *choose* asgrep. The habit lane meets
the model where it already is: for the narrow eligible set — **literal
content search on the warm indexed session root** — the stock `search_files`
tool is answered by the asgrep worker, ranked (definitions first), in the
exact stock result shape. Everything else (regex patterns, file search,
context lines, pagination, count modes, cold roots) falls through to the
stock ripgrep handler verbatim.

- **Dormant by construction**: registration uses the host's sanctioned
  `register_tool(override=True)` seam, which refuses to load unless the
  operator sets `plugins.entries.asgrep.allow_tool_override: true`. The
  host — not this plugin — enforces the trust decision.
- **Ripgrep-faithful by construction**: the takeover path mirrors
  `tools.file_tools.search_tool` by importing and calling the host's own
  pipeline pieces (repeat-search guard, path resolution, read-block
  filtering, secret redaction, truncation hints). Every needed host symbol
  is probed at load; if any moved, the lane stays inert. Any exception
  during takeover falls back to the stock handler — the override can slow a
  call, never break one.
- **Warm-only**: the search path never spawns a worker on the common path;
  takeover happens only when the pool already holds one (`prewarm` supplies
  that at load). A worker that dies between the warm check and the call is
  replaced transparently inside the same budget. The takeover call runs
  inside the stock rg pipeline's own 60s budget with the pool's transparent
  retry disabled — if the worker cannot answer in what ripgrep would have
  been given (one attempt, no re-try), the call falls back to rg, which is
  the retry.
- **Prewarm is scoped**: only repo-like roots (`.git` or an existing
  `.asgrep` index) get the speculative warm-up — a session opened in
  `$HOME` never kicks off a background index of the home tree. On such
  roots there is no warm worker, so the habit lane stays inert there and
  stock rg answers; the power-lane tools still work on demand anywhere.
- **Serves only provably-identical answers**: takeover answers only when
  the complete hit set fits strictly inside one page (a full page cannot be
  proven complete — the worker clamps the limit+1 probe at its own cap, and
  a ranked top-N is not rg's walk-order first-N; rg owns every pagination
  sequence) and no hit's preview was width-elided (stock shows up to 500
  chars of a line; an elided capsule can hide the match itself). Everything
  else — regex patterns, file search, context lines, offsets, count modes,
  overflow pages, long-line matches — falls through to stock rg.
- **Guard parity**: the repeat-search guard bumps exactly once per call,
  whichever engine answers; fallback paths restore the pre-bump tracker
  snapshot, so handoffs are invisible to the anti-loop guardrail.
- Undo: plugin unload restores the stock handler (host-tracked).
- Known deltas (documented, deliberate): match paths are repo-relative
  (stock rg prefixes `./` when given the default path) and ranking is
  definitions-first within a served page.

> Minimal-PATH hosts: GUI harnesses (desktop gateways, launchd agents) often
> run with `PATH=/usr/bin:/bin:/usr/sbin:/sbin:/usr/local/bin`, which omits
> `~/.local/bin`. Resolution falls back to `~/.local/bin/<name>` when PATH
> misses, and the `binary` config accepts an absolute path for anything else.

## Tests

```bash
python3 tests/test_plugin.py        # e2e against target/release/asgrep
python3 tests/test_override.py      # Lane B contracts; host modules faked
ASGREP_TEST_BIN=/path/to/asgrep python3 tests/test_plugin.py
```

Stdlib only; no hermes-agent import (the plugin entry is exercised through a
stub context mirroring `PluginContext`; the override suite fakes the host
pipeline symbols it mirrors).

## Doctrine notes

- **Code-Mode-XOR-MCP** (docs/mcp.md): if an `asgrep` MCP server is also
  configured for the same harness, prefer one surface — this plugin and the
  MCP server expose the same engine.
- The eagerness mechanism wraps the same host seams as `hermes-genome`
  (`tools.tool_search.is_deferrable_tool_name`,
  `registry.get_tool_names_for_toolset`, `toolsets._registry_generation`),
  identity-marked `_asgrep_eager`, chainable with other plugins doing the
  same. Any seam that cannot be wrapped is skipped — the failure mode is
  benign degradation to `tool_search` bridging.
- Lane B (the built-in `search_files` takeover) is built and **dormant by
  default**: the host's `allow_tool_override` gate must be granted before it
  activates. See "Habit lane" above; `tests/test_override.py` pins the
  eligibility, guardrail, and fallback contracts with faked host modules,
  and the takeover path was live-fired against the real host pipeline.

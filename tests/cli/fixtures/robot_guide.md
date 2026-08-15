# asgrep — agent handbook (robot-docs guide)
## Agent triad (start here)
1. `asgrep capabilities --json` — authoritative command/flag/env contract (derived from clap).
2. `asgrep robot-docs guide` — this handbook.
3. `asgrep doctor --robot-triage` — health + recovery commands using the effective root.
## Quick start
1. `asgrep index . --json` — build or refresh the index (required once per checkout).
2. `asgrep --json --format compact "natural language intent" .` — ranked hits with bounded snippets.
## Indexed source / freshness
- Do not spawn `rg` on indexed source. Use `literal:<term>` for exact substring presence and unprefixed search for ranked code navigation.
- For a long-running CLI session, run `asgrep watch <root>`. A pending batch starts after the debounce quiet period or after at most three debounce windows under continuous events; indexing time still depends on the project.
- Pi and Code Mode refresh before search, with a configurable 30-second correctness lease by default. LSP applies document open/change/save/close notifications before processing the next request.
- Ripgrep remains the tool for logs and unindexed or unsupported files. ast-sgrep never spawns it as a compatibility layer.
## Subcommands
See `capabilities --json` → `commands` (complete clap catalog). Notable: `search`/`find`/`query`, `keyword`, `semantic`, `chain`, `index`/`reindex` (`--dry-run`), `status`, `bench`, `watch`, `eval`, `doctor`, `version`.
## Integrations / sibling binaries
- `asgrep-mcp` — MCP stdio server (`ASGREP_ROOT`, tools: keyword/ast/semantic search, index_repo, code_read)
- `asgrep-lsp` — Language Server Protocol server
- `ast-sgrep` — alias of the `asgrep` executable
## Root specification
- Canonical: positional `ROOT` on the subcommand (or bare-search ROOT).
- Alias: `--root ROOT`. Conflicting `--root` + positional ROOT → usage error.
## JSON / automation
- `--format` implies `--json`. Prefer `--format compact` for bounded LLM consumption.
- Machine mode emits one JSON value on stdout and no duplicate stderr diagnostics.
## Index cancel / dry-run
- `asgrep index --dry-run` / `asgrep reindex --dry-run` report planned work without mutating the index.
- Index writes are transactional; an interrupted uncommitted write is rolled back when SQLite recovers.
## Exit codes
- 0 success · 1 usage · 2 index/search failure
## Environment
See `capabilities --json` → `environment`. Common: `ASGREP_INDEX_PATH`, `ASGREP_LIMIT`, `ASGREP_NO_EMBED`, `ASGREP_DURABILITY`, `NO_COLOR`, `CI`.
## Ops footguns (privileged sinks)
- `ASGREP_INDEX_PATH` / `--index-path` is a **privileged sink**: any absolute writable path is accepted. Treat it like a database URL; do not point it at untrusted locations.
- Index rebuilds are in-place on the default `.asgrep/` DB or a pinned `ASGREP_INDEX_PATH` (SQLite transactional rollback). There is no build-then-swap generation layout. Pinning only chooses which file; it does not change atomicity.
- `ASGREP_DURABILITY=fast-unsafe` (or `--durability fast-unsafe`) opts into power-loss corruption risk during write batches. `asgrep doctor` / `status` surface it; MCP/Code Mode inherit the env.
- MCP and Code Mode / NAPI jail tool `root` under the configured workspace (`escapes configured workspace`). Host duty remains: set `ASGREP_ROOT` / Session root intentionally; NAPI inherits Session (not a free root).
## Common mistakes
- Missing or empty index: run `asgrep index <root> --json` before searching.
- Missing ROOT is an operational error; it is never reported as an empty result.
- Full rebuild: prefer `asgrep reindex --dry-run <root> --json` before `reindex`.
- Output format is not `json`: use `--json` and optionally `--format compact` (not `--format json`).
- Piping: `asgrep --json … | head` is safe (broken pipe exits cleanly); always put data flags on asgrep, not the pipe consumer.
- Watch + long-lived MCP/Code Mode on the same index: writers bump `writer_generation` beside the index home; warm Searchers poll and reopen. Prefer one shared `ASGREP_INDEX_PATH`. See `docs/index-consistency.md`.

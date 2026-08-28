# asgrep — agent handbook (robot-docs guide)
## Agent triad (start here)
1. `asgrep capabilities --json` — authoritative command/flag/env contract (derived from clap).
2. `asgrep robot-docs guide` — this handbook.
3. `asgrep --robot-triage` (alias: `asgrep doctor --robot-triage`, `asgrep --robot-next`) — health + recovery in one call.
## Quick start
1. `asgrep --json --format compact "natural language intent" .` — ranked hits with bounded snippets. Search is read-only; it does not auto-index.
2. `asgrep index . --json` — build or refresh the index. Pass `--auto-index` on search only when you explicitly want search to write.
## Indexed source / freshness
- Do not spawn `rg` on indexed source. Use `literal:<term>` for exact substring presence and unprefixed search for ranked code navigation.
- For a long-running CLI session, run `asgrep watch <root>`. A pending batch starts after the debounce quiet period or after at most three debounce windows under continuous events; indexing time still depends on the project.
- Pi and Code Mode refresh before search, with a configurable 30-second correctness lease by default. LSP applies document open/change/save/close notifications before processing the next request.
- Ripgrep remains the tool for logs and unindexed or unsupported files. ast-sgrep never spawns it as a compatibility layer.
## Subcommands
See `capabilities --json` → `commands` (complete clap catalog). Notable: `search`/`find`/`query`, `keyword`, `semantic`, `chain`, `call-path`, `index`/`reindex` (`--dry-run`), `codemod`, `status`, `bench`, `watch`, `eval`, `doctor`, `version`.
## Integrations / sibling binaries
- `asgrep-mcp` — MCP stdio server (`ASGREP_ROOT`, tools: keyword/ast/semantic search, index_repo, code_read)
- `asgrep-lsp` — Language Server Protocol server
- `ast-sgrep` — alias of the `asgrep` executable
## Root specification
- Canonical: positional `ROOT` on the subcommand (or bare-search ROOT).
- Alias: `--root ROOT`. Conflicting `--root` + positional ROOT → usage error.
## JSON / automation
- `--json` / `-j` emit one JSON value on stdout. `--format` implies `--json`. Prefer `--format compact` for bounded LLM consumption.
- Recovered spellings: `--jason`, `--machine`, `--output-json`, `--format=json` → `--json`.
- Machine mode emits no duplicate stderr diagnostics. Agent envelopes omit TTY and wall-clock fields.
## Index cancel / dry-run
- `asgrep index --dry-run` / `asgrep reindex --dry-run` report planned work without mutating the index.
- `asgrep codemod --pattern 'legacy($ARG)' --rewrite 'modern($ARG)' --dry-run .` emits a JSON edit plan without writing. Apply with `--yes` (alias `--force`): `asgrep codemod --yes --pattern 'legacy($ARG)' --rewrite 'modern($ARG)' .` commits one source transaction, then a separate index refresh. If refresh fails, source edits remain applied and the command reports `asgrep index` as recovery.
- Index writes are transactional; an interrupted uncommitted write is rolled back when SQLite recovers.
## Exit codes
- 0 success · 1 usage · 2 index/search failure
## Confirmation / first-try recovery
- Source writes: `asgrep codemod` apply requires `--yes` (alias `--force`). Always plan with `--dry-run` first.
- `--help` after-help names the triad, `-j/--json`, and the `--yes` gate.
- Typos: nearby flag/command spellings rewrite before clap (`--jsno` → `--json`, `capabilites` → `capabilities`, `docs` → `robot-docs`, `--dryrun` → `--dry-run`).
## Environment
See `capabilities --json` → `environment`. Common: `ASGREP_INDEX_PATH`, `ASGREP_LIMIT`, `ASGREP_NO_EMBED`, `ASGREP_NO_AUTO_INDEX`, `ASGREP_DURABILITY`, `NO_COLOR`, `CI`, `TERM=dumb`, `SOURCE_DATE_EPOCH` (bench history timestamps). `CI=1` and `TERM=dumb` suppress progress chatter (`asgrep: indexing …`) so logs stay quiet without `--json`.
## Ops footguns (privileged sinks)
- `ASGREP_INDEX_PATH` / `--index-path` is a **privileged sink**: any absolute writable path is accepted. Treat it like a database URL; do not point it at untrusted locations.
- Index rebuilds are in-place on the default `.asgrep/` DB or a pinned `ASGREP_INDEX_PATH` (SQLite transactional rollback). There is no build-then-swap generation layout. Pinning only chooses which file; it does not change atomicity.
- `ASGREP_DURABILITY=fast-unsafe` (or `--durability fast-unsafe`) opts into power-loss corruption risk during write batches. `asgrep doctor` / `status` surface it; MCP/Code Mode inherit the env.
- MCP and Code Mode / NAPI jail tool `root` under the configured workspace (`escapes configured workspace`). Host duty remains: set `ASGREP_ROOT` / Session root intentionally; NAPI inherits Session (not a free root).
## Common mistakes
- Empty index: run `asgrep index <root> --json`. Search does not auto-index; pass `--auto-index` only when search may write.
- Missing ROOT is an operational error; it is never reported as an empty result.
- Full rebuild: prefer `asgrep reindex --dry-run <root> --json` before `reindex`.
- Output format is not `json`: use `--json` and optionally `--format compact` (not `--format json`).
- Piping: `asgrep --json … | head` is safe (broken pipe exits cleanly); always put data flags on asgrep, not the pipe consumer.
- Watch + long-lived MCP/Code Mode on the same index: writers bump `writer_generation` beside the index home; warm Searchers poll and reopen. Prefer one shared `ASGREP_INDEX_PATH`. See `docs/index-consistency.md`.

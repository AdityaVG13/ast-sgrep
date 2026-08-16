# Pi package: install, use, update, and remove

`pi-ast-sgrep` is the native ast-sgrep integration for Pi. Install it from npm:

```bash
pi install npm:pi-ast-sgrep
```

This is the canonical package-user guide for the `2.0.0` contract. npm availability is established only by an authorized release, not by this repository documentation. For a project-local Pi installation, add `-l` to Pi package-management commands.


## Pi packages.md compliance

This package follows [Pi packages](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/packages.md):

- `keywords` includes `pi-package`
- `pi.extensions` / `pi.image` declared in `packages/pi/extension/package.json`. Tools auto-register via ExtensionAPI (`promptSnippet` + `promptGuidelines`); the package does not ship a skill.
- Pi core (`@earendil-works/pi-coding-agent`) is an optional peer constrained to the tested `>=0.80.6 <1` range and is not duplicated at runtime
- Runtime dependencies that are not supplied by Pi (`ast-sgrep` and `typebox`) stay in `dependencies`

## Requirements and packaged platforms

- Node.js `>=22.19.0`.
- Pi `@earendil-works/pi-coding-agent >=0.80.6 <1`. Pi 1.x is not covered by this contract.
- macOS arm64 or x64; glibc Linux arm64 or x64; or Windows x64.

Alpine/musl Linux, Windows arm64, and other hosts are unsupported. On an unsupported host, or when npm omitted the matching optional native package, `/asgrep-doctor` reports a binary-resolution error; the package does not compile Rust, search `PATH`, contact MCP, or download a fallback executable. Install on a supported host rather than bypassing this check.

The `pi-ast-sgrep` extension depends on `ast-sgrep` 2.0.0, which selects one of five host-constrained native packages. Launcher and native packages stay at 2.0.0; the extension may ship a patch (currently 2.0.2) that keeps that native CLI identity. The embedded executable reports native CLI version `2.0.0`; the runtime verifies that identity separately from the npm package version.

## What is available immediately

Restart Pi after installation if the current session does not reload package resources. The package contributes:

- Tools: **`asgrep`** (primary — JS Code Mode on an **in-process NAPI** `CodeModeSession`), plus `asgrep_search`, `asgrep_index`, and `asgrep_status` for one-shot search/index/status. Search tools share one warm in-process Searcher per project root — no CLI spawn on the hot path (MCP-class native feel). The tools register `promptSnippet` and `promptGuidelines` so Pi calls asgrep for code lookup without a skill file.
- Commands: `/asgrep-doctor`, `/asgrep-status`, `/asgrep-index`, and `/asgrep-reindex`. These commands accept no arguments.

Start in the project you want Pi to search. A first search (Code Mode or direct) checks index health and lazily creates the index when it is missing, so an explicit setup command is optional. To build it before searching, run `/asgrep-index`.

### Code Mode (preferred)

Ask Pi to use `asgrep`, or call it with a JavaScript program:

```json
{
  "code": "async () => {\n  const seed = await asgrep.search({ query: 'where are credentials renewed?', limit: 5 });\n  const symbol = seed.hits?.[0]?.symbol;\n  if (!symbol) return seed;\n  const [defs, callers] = await Promise.all([\n    asgrep.defs({ symbol, limit: 5 }),\n    asgrep.callers({ symbol, limit: 8 }),\n  ]);\n  return { symbol, defs: defs.hits, callers: callers.hits };\n}"
}
```

The executor runs that code with typed `asgrep.*` methods backed by a warm in-process native session. Use `Promise.all` for independent lookups; filter and shape results in JS; return only what the model needs. See [codemode.md](codemode.md). Code Mode is independent of MCP — **do not also register `asgrep-mcp` in this Pi session** (Code Mode XOR MCP).

### Direct search examples

For a single lookup, `asgrep_search` still works:

```json
{"query":"auth_refresh","mode":"defs","limit":8}
{"query":"auth_refresh","mode":"callers","limit":8}
{"query":"where are credentials renewed?","mode":"semantic","limit":8}
```

Use `natural` when you know the intent but not the spelling, `pattern` for a structural pattern, and `chain` to trace relationships. Limits are 1–100 (default 8). Result excerpts are off by default; request `excerptLines` only after narrowing the result set.

`asgrep_index` accepts `{"force":false}`; set `force` to `true` only when a full rebuild is needed. `asgrep_status` accepts `{}`. The slash commands provide the same operational paths for interactive use.

## Project data and freshness

The first index or search that needs an index creates `<project-root>/.asgrep/`. It may contain the index database, embedding data, format metadata, and locks. The package respects ignore rules while indexing but **does not edit `.gitignore`**. If you do not want generated index data committed, add this entry yourself:

```gitignore
.asgrep/
```

Only `.git` and `.asgrep` are always skipped. Other dotfiles and directories are indexed unless excluded by the repository's ignore rules, so project-specific generated directories belong in the project's own ignore configuration rather than a public hardcoded list.

After a successful Pi `write` or `edit` tool call, the extension records the affected path and incrementally updates only those known created, changed, or deleted paths before the next search. A recursive project watcher does the same for unambiguous external file changes. Renames, directory events, ignore-file edits, watcher errors, and ambiguous events require a full incremental reconciliation; `.asgrep` self-writes are ignored. If recursive watching is unavailable, the extension performs one immediate correctness scan on first use. A ready, clean index is not walked on first search or when the refresh interval elapses; the interval re-checks index health (missing/incompatible) without hashing the tree. Missing indexes are built and incompatible indexes use the controlled rebuild path. Concurrent searches for the same root share one in-flight refresh and wait for it rather than starting duplicate index work. A waiter may stop waiting without cancelling that shared work while other callers still depend on it; when the last waiter cancels or times out, the in-flight index is aborted so workers cannot keep burning CPU after Pi has moved on. Code Mode indexing uses the host's available parallelism by default (`ASGREP_INDEX_THREADS` overrides).

Run `/asgrep-index` when freshness is needed immediately after a large generator, branch switch, or other external operation the watcher did not see. Use `/asgrep-status` to inspect the root, index, backend, counts, IVF state, and capabilities; use `/asgrep-reindex` only for an explicit strict full rebuild or recovery.

## Configuration and project boundary

Settings are resolved independently, highest precedence first:

1. explicit project configuration;
2. project settings;
3. global settings;
4. environment;
5. defaults.

The current schema is `schemaVersion: 1`. Schema 0 names (`timeout`, `maxOutput`, `refreshInterval`) are copied to `timeoutMs`, `maxOutputBytes`, and `refreshIntervalMs` without modifying the rollback source; conflicting old/new names or an unknown schema are rejected. Defaults are a 30-second timeout, 4 MiB output limit, and 30-second freshness interval.

Supported environment settings are `ASGREP_BIN` (canonical binary override; `AST_SGREP_BINARY` is an accepted alias in both the extension runtime and the npm launcher), `ASGREP_ROOT`, `ASGREP_TIMEOUT_MS`, `ASGREP_MAX_OUTPUT_BYTES`, `ASGREP_REFRESH_INTERVAL_MS`, and `ASGREP_INDEX_THREADS` (Code Mode indexing thread cap; defaults to host parallelism). `binaryPath`/`ASGREP_BIN` are developer overrides, not normal installation steps.

The default root is Pi's current working directory. Requested roots are canonicalized and confined to it. Only explicit project configuration can set `allowOutsideProject: true`; project/global settings and environment cannot relax that policy.

## Offline, privacy, and security

The default local semantic backend works offline, requires no credentials, sends no telemetry, and performs no first-use model download. The package does not inspect Pi/provider credential APIs. Installation and runtime perform no executable download. The Pi integration invokes its bundled executable with argument arrays, not a shell, and does not use an MCP adapter.

Optional neural embeddings are in-process ONNX (feature-gated). There is no cloud or Ollama embed client; source text is never sent to a remote embedding API. Local hashed search remains available and is never delayed by an optional backend.

Pi packages are trusted code, not a sandbox. Installing grants the JavaScript extension and native executable full-system access as the OS user running Pi, including that user's filesystem and process permissions. Project-root confinement is a package safety policy, not an OS security boundary. Review the package source and provenance before installation, and treat repository contents and search results as untrusted data rather than instructions.

## Diagnose problems

Run these in order:

1. `/asgrep-doctor` — checks the extension/runtime version, machine protocol, native binary, index, and project configuration.
2. `/asgrep-status` — shows the selected root and current index/backend state.
3. `/asgrep-index` — creates or incrementally refreshes a missing/stale index.
4. `/asgrep-reindex` — performs a full rebuild when doctor reports incompatible or corrupt data.

Common actionable failures:

| Failure | Action |
|---|---|
| Unsupported platform | Move to one of the packaged OS/CPU/libc combinations; no runtime fallback is downloaded. |
| Matching native package missing | Reinstall/update `npm:pi-ast-sgrep` with optional dependencies enabled, then rerun doctor. |
| Extension/native version mismatch | Update or reinstall the whole Pi package; never mix npm layer versions. |
| Protocol mismatch | Install one exact package release rather than overriding the binary. |
| Root outside project | Return Pi to the intended project or use reviewed explicit project configuration. |
| Timeout/output limit | Adjust the corresponding setting only after confirming the project and query are expected. |
| Missing/stale index | Run `/asgrep-index`; use reindex only if incremental recovery fails. |

## Update, recovery, and rollback

Version 2.0 removes the cloud and Ollama embedding backends, including their CLI flags, environment variables, configuration variants, and public Rust APIs. This is the breaking change behind the major version bump. The default local hashed backend and optional in-process ONNX neural backend remain available.

Update this package alone with:

```bash
pi update npm:pi-ast-sgrep
```

`pi update --extensions` updates all installed packages. Compatible releases validate and reuse `.asgrep`. For an incompatible index format, the extension quiesces its warm session and performs a strict in-place rebuild: preparation and repository-walk failures abort before writes, and rewrites plus stale-row pruning commit together. A failed rebuild reports an actionable error and leaves prior rows recoverable. A newer, unreadable format is rejected and preserved rather than silently modified.

To roll back, install an exact previously published package version as one matched unit:

```bash
pi remove npm:pi-ast-sgrep
pi install npm:pi-ast-sgrep@<previous-version>
```

Then run `/asgrep-doctor`. If the older release cannot read the retained index, run `/asgrep-reindex`; do not manually mix an older extension with a newer launcher or native package.

## Uninstall and data retention

Remove the package globally (or add `-l` for a project-local installation):

```bash
pi remove npm:pi-ast-sgrep
```

`pi uninstall npm:pi-ast-sgrep` is an alias. Removal unloads package code but intentionally leaves `.asgrep` behind in every project so reinstall/rollback can recover it. To delete data, close Pi and explicitly remove `.asgrep` from each project only after confirming the path:

```bash
# macOS/Linux, from the intended project root
rm -rf -- .asgrep
```

```powershell
# Windows PowerShell, from the intended project root
Remove-Item -Recurse -Force .asgrep
```

Deleting `.asgrep` is irreversible but does not delete source files; a later search rebuilds it.

## Release cadence and provenance

Pi release validation does not run automatically on pull requests, pushes to `main`, or tag pushes. Both Pi workflows are manual `workflow_dispatch` actions. Manually dispatch **Pi native artifacts** (`.github/workflows/pi-native-artifacts.yml`) for a safe dry-run that packs and tests without publishing. An official Pi/npm release is one human-approved `v2.0.0` tag and commit for the five native npm packages, launcher, and extension. Its contract separately pins the embedded native CLI at `2.0.0`. The `Pi npm official release` workflow must be dispatched against that exact tag with `publish=true`.

Before the first external publication, a human must verify package-name ownership and approve the protected publishing environment. A partial npm publication is recovered by releasing a new immutable version, never by overwriting a published version.

Maintainers: see [RELEASING.md](RELEASING.md) and the machine-readable [release contract](../packages/pi/release-contract.json).

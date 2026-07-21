# Pi package: install, use, update, and remove

`pi-ast-sgrep` is the native ast-sgrep integration for Pi. Install it from npm:

```bash
pi install npm:pi-ast-sgrep
```

This is the canonical package-user guide for the `1.2.0-alpha` contract. The package is an alpha release; npm availability is established only by an authorized release, not by this repository documentation. For a project-local Pi installation, add `-l` to Pi package-management commands.

## Requirements and packaged platforms

- Node.js `>=22.19.0`.
- Pi `@earendil-works/pi-coding-agent >=0.80.6 <1`. Pi 1.x is not covered by this contract.
- macOS arm64 or x64; glibc Linux arm64 or x64; or Windows x64.

Alpine/musl Linux, Windows arm64, and other hosts are unsupported. On an unsupported host, or when npm omitted the matching optional native package, `/asgrep-doctor` reports a binary-resolution error; the package does not compile Rust, search `PATH`, contact MCP, or download a fallback executable. Install on a supported host rather than bypassing this check.

The npm layers are exact-version matched: the `pi-ast-sgrep` extension depends on `ast-sgrep`, which selects one of five host-constrained native packages. Extension, launcher, and native package manifests must all be `1.2.0-alpha`. The embedded executable reports native CLI version `1.2.0-alpha`; the runtime verifies that identity separately from the npm package version.

## What is available immediately

Restart Pi after installation if the current session does not reload package resources. The package contributes:

- Tools: `asgrep_search`, `asgrep_index`, and `asgrep_status`.
- Commands: `/asgrep-doctor`, `/asgrep-status`, `/asgrep-index`, and `/asgrep-reindex`. These commands accept no arguments.
- Skill: `ast-sgrep`, which tells Pi when to use intent, structural, definition, caller, chain, or semantic search and when exact-text search is better.

Start in the project you want Pi to search. A first `asgrep_search` checks index health and lazily creates the index when it is missing, so an explicit setup command is optional. To build it before searching, run `/asgrep-index`.

### Search examples

Ask Pi to use `asgrep_search`, or call the tool with arguments like these:

```json
{"query":"auth_refresh","mode":"defs","limit":8}
{"query":"auth_refresh","mode":"callers","limit":8}
{"query":"where are credentials renewed?","mode":"semantic","limit":8}
```

Use `natural` when you know the intent but not the spelling, `pattern` for a structural pattern, and `chain` to trace relationships. Limits are 1–100 (default 8). Result excerpts are off by default; request `excerptLines` only after narrowing the result set.

`asgrep_index` accepts `{"force":false}`; set `force` to `true` only when a full rebuild is needed. `asgrep_status` accepts `{}`. The slash commands provide the same operational paths for interactive use.

## Project data and freshness

The first index or search that needs an index creates `<project-root>/.asgrep/`. It may contain the index database, embedding data, format metadata, locks, and atomic-rebuild state. The package respects ignore rules while indexing but **does not edit `.gitignore`**. If you do not want generated index data committed, add this entry yourself:

```gitignore
.asgrep/
```

After a successful Pi `write` or `edit` tool call, the extension marks the affected path dirty and refreshes it before the next search. After the configured interval (30 seconds by default) without edits, the package rechecks index health via `status` and only rebuilds when the index is missing or incompatible—not on a pure wall-clock lease alone. Concurrent searches for the same root share one in-flight refresh and wait for it rather than starting duplicate index work. Wall-clock freshness is best-effort: large clock skew or a backward jump can delay expiry detection; prefer write/edit dirtying for correctness-critical workflows. Use `/asgrep-status` to inspect the root, index, backend, counts, IVF state, and capabilities; use `/asgrep-reindex` only for an explicit full rebuild or recovery.

## Configuration and project boundary

Settings are resolved independently, highest precedence first:

1. explicit project configuration;
2. project settings;
3. global settings;
4. environment;
5. defaults.

The current schema is `schemaVersion: 1`. Schema 0 names (`timeout`, `maxOutput`, `refreshInterval`) are copied to `timeoutMs`, `maxOutputBytes`, and `refreshIntervalMs` without modifying the rollback source; conflicting old/new names or an unknown schema are rejected. Defaults are a 30-second timeout, 4 MiB output limit, and 30-second freshness interval.

Supported environment settings are `ASGREP_BIN` (canonical binary override; `AST_SGREP_BINARY` is an accepted alias in both the extension runtime and the npm launcher), `ASGREP_ROOT`, `ASGREP_TIMEOUT_MS`, `ASGREP_MAX_OUTPUT_BYTES`, and `ASGREP_REFRESH_INTERVAL_MS`. `binaryPath`/`ASGREP_BIN` are developer overrides, not normal installation steps.

The default root is Pi's current working directory. Requested roots are canonicalized and confined to it. Only explicit project configuration can set `allowOutsideProject: true`; project/global settings and environment cannot relax that policy.

## Offline, privacy, and security

The default local semantic backend works offline, requires no credentials, sends no telemetry, and performs no first-use model download. The package does not inspect Pi/provider credential APIs. Installation and runtime perform no executable download. The Pi integration invokes its bundled executable with argument arrays, not a shell, and does not use an MCP adapter.

Cloud, Ollama, and neural embedding backends are optional and opt-in. If you select an external service, source text and queries needed for embeddings may be sent to that provider; that service's credentials, retention, and privacy policy then apply. Local search remains available and is never delayed or replaced by an optional backend.

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

Update this package alone with:

```bash
pi update npm:pi-ast-sgrep
```

`pi update --extensions` updates all installed packages. Compatible releases validate and reuse `.asgrep`. For an incompatible index format, the extension builds a replacement separately and swaps it only after validation; a failed rebuild reports an actionable error and leaves prior data recoverable. A newer, unreadable format is rejected and preserved rather than silently modified.

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

Pi release validation does not run automatically on pull requests, pushes to `main`, or tag pushes. Both Pi workflows are manual `workflow_dispatch` actions. Manually dispatch **Pi native artifacts** (`.github/workflows/pi-native-artifacts.yml`) for a safe dry-run that packs and tests without publishing. An official Pi/npm release is one human-approved `v1.2.0-alpha` tag and commit for the five native npm packages, launcher, and extension. Its contract separately pins the embedded native CLI at `1.2.0-alpha`. The `Pi npm official release` workflow must be dispatched against that exact tag with `publish=true`.

Before the first external publication, a human must verify package-name ownership and approve the protected publishing environment. A partial npm publication is recovered by releasing a new immutable version, never by overwriting a published version.

Maintainers: see [RELEASING.md](RELEASING.md) and the machine-readable [release contract](../packages/pi/release-contract.json).

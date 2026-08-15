# Environment trust model

Agent and CLI surfaces read configuration from the environment. Untrusted values
must not become silent privilege or SSRF primitives.

## Boolish flags

`ASGREP_*` boolean flags accept `1` / `true` / `yes` / `on` (case-insensitive).
Other spellings are false. CLI clap flags use `BoolishValueParser`; library
defaults use `env_flag` / the same spelling set.

## Embeddings (in-process only)

Embeddings never leave the process. There is no `ASGREP_EMBED_API_URL`, no
Ollama URL, no API key, and no SSRF allowlist. Stored indexes that still
record `cloud` or `ollama` fail closed at query time until `asgrep reindex`.

Neural (ONNX) is opt-in via `--features neural-embed` and `ASGREP_NEURAL_EMBED`.
Explicit Neural does not silently swap to hashed hits unless
`ASGREP_NEURAL_FALLBACK=1`. CoreML EP is opt-in via `ASGREP_NEURAL_COREML`.
See `docs/validation/neural-trust.md`.

## External `ast-grep` (`ASGREP_AST_GREP`)

Production `pattern:` search **never** starts `ast-grep` (see
`docs/structural-patterns.md`). Optional **bench comparison** may invoke an
external binary only when **both** are set:

- `ASGREP_ALLOW_AST_GREP=1`
- `ASGREP_AST_GREP=/absolute/path/to/ast-grep`

PATH names (`ast-grep`, `sg`) and relative paths are ignored. A timed
`--version` probe must succeed before any bench run; hung children are killed
and reaped.

## Binary overrides (`ASGREP_BIN` / `binaryPath`)

Pi launcher / extension may override the `asgrep` binary. Overrides must resolve
to an existing executable file; missing or non-executable paths raise
`BINARY_NOT_FOUND` / `BINARY_NOT_EXECUTABLE` rather than falling through to an
untrusted PATH search when an explicit override was configured.

## MCP / Code Mode / NAPI workspace root

`asgrep-mcp` canonicalizes `ASGREP_ROOT` at startup. Tool `root` arguments must
canonicalize under that workspace; escapes fail closed
(`escapes configured workspace`).

Code Mode `CodeModeSession` and NAPI (which wraps Session) apply the **same
jail** under `SessionConfig.root`. Host duty: set the session/workspace root
intentionally. Tool `root` is **not** a free absolute-path escape hatch.

## Privileged index path (`ASGREP_INDEX_PATH` / `--index-path`)

An absolute (or otherwise writable) index path is a **privileged sink**: the
process will create/open that SQLite database wherever the path points. Do not
accept untrusted values for this env/flag. Pinning also disables generation
atomic reindex (in-place rebuild crash window) — see
`docs/index-consistency.md`.

## Durability (`ASGREP_DURABILITY`)

| Value | Risk |
|-------|------|
| `balanced` (default) | Survives process crash |
| `strict` | Survives power loss; slower |
| `fast-unsafe` | `synchronous=OFF` during write batches; power loss can corrupt the index |

MCP and Code Mode inherit `Durability::from_env()` via `IndexOptions::default`.
`asgrep doctor` emits `durability_fast_unsafe` when FastUnsafe is active;
`asgrep status` reports the `durability` field.

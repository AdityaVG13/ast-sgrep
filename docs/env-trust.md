# Environment trust model

Agent and CLI surfaces read configuration from the environment. Untrusted values
must not become silent privilege or SSRF primitives.

## Boolish flags

`ASGREP_*` boolean flags accept `1` / `true` / `yes` / `on` (case-insensitive).
Other spellings are false. CLI clap flags use `BoolishValueParser`; library
defaults use `env_flag` / the same spelling set.

## Embed HTTP URLs (`ASGREP_EMBED_API_URL`, `ASGREP_OLLAMA_URL`)

Requests are allowlisted before any HTTP client call (`embed_url_is_allowed`):

| Host | Notes |
|------|-------|
| `api.openai.com`, `api.azure.com` | Default cloud hosts |
| `127.0.0.1`, `localhost`, `::1` | Default Ollama loopback |
| `ASGREP_EMBED_URL_ALLOWLIST` | Extra comma-separated hosts |

`http://` is limited to loopback unless `ASGREP_EMBED_ALLOW_INSECURE_HTTP=1`.
Non-allowlisted hosts fail closed (config ignored / request error) — never silent
fallback to a private metadata endpoint.

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

## MCP workspace root

`asgrep-mcp` canonicalizes `ASGREP_ROOT` at startup. Tool `root` arguments must
canonicalize under that workspace; escapes fail closed.

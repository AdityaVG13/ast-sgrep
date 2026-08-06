# Machine JSON schema notes (`ziij` / `0f7r`)

Shared agent envelope fields (CLI `--json`, Pi runtime, capabilities catalog):

| Field | Type | Notes |
|-------|------|-------|
| `tool` | string | Always `"asgrep"` |
| `schema_version` | string | Protocol id (`1.0.0`); Pi rejects mismatch |
| `ok` | bool | Fail-closed: hard faults set `false` |
| `version` | string? | Binary semver; conjunction with `machine_schema_version` (ls6.2) |
| `machine_schema_version` | string? | Same as `schema_version` identity for Pi triple check |
| `command` | string? | Subcommand that produced the envelope |
| `error` | object? | `{kind, message, ...}` when `ok=false` |

Capabilities: `asgrep capabilities --json` lists `machine_schema` plus boolish env spellings.
CLI and Pi both require `tool` + `schema_version` + boolean `ok`.
MCP uses JSON-RPC tool results (`isError`) rather than the CLI envelope; see surface-parity.

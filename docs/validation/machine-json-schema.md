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

Pin: `MACHINE_SCHEMA_VERSION = "1.0.0"` in `crates/ast-sgrep-cli/src/machine.rs`.

Clause IDs **MJ-xxx** (ghiw.2). Tests: `tests/cli/machine_contracts.rs`. Score is
**TBD** (clause IDs landed; do not claim MUST ≥ 0.95 until ghiw.5 counts a real
run). `assert_golden` migration of hit dumps is **nz7i.2**, not this matrix.

## MUST matrix

| ID | MUST | Evidence (`machine_contracts.rs`) | Status |
|---|---|---|---|
| MJ-001 | `tool` is `"asgrep"` on success and failure envelopes | `assert_success`, `assert_doctor_unhealthy`, operational/usage goldens | covered |
| MJ-002 | `schema_version` is `"1.0.0"` | same helpers | covered |
| MJ-003 | `ok` is boolean; success `true`, hard fault `false` | helpers + `operational_failures_are_json_and_exit_two` | covered |
| MJ-004 | Success exit 0; usage exit 1; operational/doctor exit 2 | `assert_success` (0), `bounded_arguments_are_json_usage_errors` (1), operational + doctor (2) | covered |
| MJ-005 | Operational `FailureBundle`: `error.kind=operational` | `operational_failures_are_json_and_exit_two` | covered |
| MJ-006 | Usage `FailureBundle`: `error.kind=usage` | `bounded_arguments_are_json_usage_errors`, typo/format cases in `format_aliases_typos_and_root_failures_are_unambiguous` | covered |
| MJ-007 | Capabilities dump is frozen (version scrubbed) | `capabilities_and_version_match_goldens` (`assert_golden_json_at`) | covered |
| MJ-008 | Index / reindex / status / doctor key-set freeze | `index_reindex_status_and_doctor_have_stable_shapes` vs `machine_shapes.json` | covered |
| MJ-009 | Format aliases (`search`/`find`/`query`) and compact shape | `format_aliases_typos_and_root_failures_are_unambiguous`; compact keys in `machine_shapes.json` | covered |
| MJ-010 | Doctor unhealthy: `healthy:false`, `ok:false`, exit 2 | `assert_doctor_unhealthy`; missing-root `issues[0].kind=missing_root` | covered |
| MJ-013 | `--format` alone implies machine JSON on stdout | `format_alone_implies_json_machine_output` | covered |

## Gap / DISC rows (not Pass)

| ID | Gap | Why it is not a MUST here | Pointer |
|---|---|---|---|
| MJ-011 | Full **search hit-array** golden freeze | Envelope/shape is frozen; ranked hit bodies are nz7i.2 | `COVERAGE` S6 partial; golden program |
| MJ-012 | MCP does **not** use this CLI envelope | JSON-RPC `isError`; not a second copy of `--json` | `DISC-mcp-not-full-suite`, `surface-parity.md` |

Do not force MCP onto MJ-001…010. Multi-consumer (CLI + Pi + MCP) is not one
suite.

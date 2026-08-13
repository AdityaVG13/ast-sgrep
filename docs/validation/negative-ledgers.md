# Negative ledgers (`6lmt`)

**Naming bridge:** this file is the **product fail-closed case table** (CLI/MCP
must error, not return empty hits). It is **not** the gauntlet campaign
rejection ledger. Campaign Open/Closed/Retired rows live under
[`docs/progress/`](../progress/README.md)
(`perf-negative-results.md`, `conformance-negative-results.md`,
`surface-deferrals.md`). Do not copy fail-closed rows into those files as
"measured rejects," and do not treat a campaign Open pointer as a product
error contract.

Cases that must **not** succeed as silent empty hits:

| Case | Expected |
|------|----------|
| Missing project root | exit 2 / operational error containing `does not exist` |
| Empty index (0 files) | exit 2 / `index is empty` |
| Doctor on missing root | `healthy:false`, `ok:false`, triage `missing_root` |
| MCP root outside workspace | tool `isError`, message `escapes configured workspace` |
| Embed URL to metadata IP | `embed_url_is_allowed` Err |
| Empty native package binary | `ASGREP_EXECUTABLE_EMPTY` even if checksum is empty-SHA256 |
| Regex worker panic | `StoreError` (not empty hit list) |

Harness stubs live under `tests/fixtures/ranking/` and `docs/validation/`.

Clause IDs **NL-xxx** (ghiw.2). Score TBD; do not claim MUST ≥ 0.95 from this
table. Compact omitting provenance is **DISC-compact-drops-provenance**, not a
fail-closed bug.

## MUST-not matrix

| ID | MUST-not | Expected | Test / gap |
|---|---|---|---|
| NL-001 | Missing project root must not return hits | exit 2, operational, message contains `does not exist` | `format_aliases_typos_and_root_failures_are_unambiguous` |
| NL-002 | Empty index must not return hits | exit 2, `index is empty` (search and chain) | same |
| NL-003 | Doctor on missing root must not look healthy | `healthy:false`, `ok:false`, `issues[0].kind=missing_root` | `agent_discovery_defaults_and_boolish_envs_are_round_trip_free` |
| NL-004 | Usage vs operational exits stay distinct | usage=1 / operational=2; never swap | `bounded_arguments_are_json_usage_errors`, `operational_failures_are_json_and_exit_two` |
| NL-005 | MCP root outside workspace | `isError`, `escapes configured workspace` | **gap** for this bead (MCP suite is `DISC-mcp-not-full-suite`; do not invent CLI-envelope coverage) |
| NL-006 | Embed URL to metadata IP | `embed_url_is_allowed` Err | **partial** / core embed allowlist tests; not CLI envelope |
| NL-007 | Empty native package binary | `ASGREP_EXECUTABLE_EMPTY` | **gap** here (packaging path; not machine_contracts) |
| NL-008 | Compact must not be treated as native JSON equality | Compact key set is `h/p/q/v/…`; no native `hits` array / excerpt blobs | `compact_omits_native_hit_array_and_excerpt_blobs` + `DISC-compact-drops-provenance` |
| NL-009 | Regex worker panic must not become empty hits | `StoreError` | **gap** in CLI machine suite (core store path) |

NL-008 is the new machine-visible negative: compact is a different shape, and
dropping provenance is intentional (`DISC-compact-drops-provenance`).

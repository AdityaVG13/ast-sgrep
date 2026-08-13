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

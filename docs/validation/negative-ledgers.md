# Negative ledgers (`6lmt`)

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

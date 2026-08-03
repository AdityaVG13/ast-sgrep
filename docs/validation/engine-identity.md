# Engine identity and failure bundles (`djo7`)

## EngineIdentity

| Field | Meaning |
|-------|---------|
| `tool` | Always `asgrep` on machine envelopes |
| `schema_version` | Machine JSON protocol (`1.0.0`) |
| `version` | `CARGO_PKG_VERSION` / Pi `RUNTIME_VERSION` (must match) |
| `embed_backend` | Stored meta: `semantic` / `neural` / `cloud` / `ollama` |
| `index_format` | SQLite user_version / Pi `INDEX_FORMAT_VERSION` |

## FailureBundle

| Kind | Exit | Envelope |
|------|------|----------|
| `usage` | 1 | `ok:false`, `error.kind=usage` |
| `operational` | 2 | `ok:false`, `error.kind=operational` (missing root, empty index, IO) |
| `doctor_unhealthy` | 2 | Doctor body with `healthy:false` and `ok:false` |
| `mcp_tool` | JSON-RPC tool result | `isError:true` text content (no panic) |

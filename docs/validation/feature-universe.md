# Feature universe (`f8qy.3`)

Canonical IDs live in the machine matrix
[`docs/contracts/supported_surface_matrix.toml`](../contracts/supported_surface_matrix.toml)
(`present|partial|missing|excluded|n/a` per host). This table is the short human index.

Weights (not certified scores): [`docs/contracts/parity_score_contract.toml`](../contracts/parity_score_contract.toml).
Conformal seed: [`tests/conformance/parity_score.json`](../../tests/conformance/parity_score.json) (`certified=false`).
Intentional deltas: [`docs/progress/surface-deferrals.md`](../progress/surface-deferrals.md).

| Feature ID | Surface | Notes |
|------------|---------|-------|
| `hybrid_search` | CLI/MCP/LSP | Default unprefixed query cascade |
| `semantic_search` | CLI `semantic` / MCP `semantic_search` | Embed channel only |
| `keyword_search` | CLI `keyword` / MCP `keyword_search` | Lexical FTS |
| `pattern_search` | `pattern:` / MCP `ast_search` | Native tree-sitter + index signatures |
| `defs_callers_imports` | Query prefixes | Graph modes |
| `chain` | CLI `chain` | Call-chain traversal |
| `compact_output` | `--format compact` | Token-budgeted agent output |
| `doctor` | CLI `doctor` | Fail-closed triage envelope |
| `mcp_index_repo` | MCP | Single-flight + deadline |
| `forbid_soundness` | CI | First-party unsafe ban |

Negative ledgers (fail-closed product cases): `docs/validation/negative-ledgers.md`.
Campaign deferrals: `docs/progress/surface-deferrals.md`.
Engine identity: `docs/validation/engine-identity.md`.

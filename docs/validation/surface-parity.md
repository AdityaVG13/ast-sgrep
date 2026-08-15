# Surface parity table (`k7l8.9`)

| Capability | CLI | MCP | LSP | Pi |
|------------|-----|-----|-----|----|
| Hybrid search | yes | via keyword/ast/semantic channels (no auto-fusion) | `asgrep.search` | extension tools |
| Semantic-only | `--semantic-only` / `semantic` | `semantic_search` | `asgrep.search.semantic` | yes |
| Limit clamp | `MAX_OUTPUT_RESULTS` | `clamp_agent_limit` (100) | default_limit | timeout/bytes caps |
| Index | `index`/`reindex` | `index_repo` (single-flight) | background index | rebuild helpers |
| Doctor/triage | `doctor` | — | — | `/asgrep-doctor` |
| Boolish env | clap Boolish + core `env_flag` | NO_EMBED boolish | settings | env aliases |

Intentional deltas: MCP does not auto-fuse channels (`excluded`, not a bug);
LSP focuses on IDE navigation. Formal statuses:
[`docs/contracts/supported_surface_matrix.toml`](../contracts/supported_surface_matrix.toml).

# Search signal provenance and margins

Every returned hit carries three required ranking fields:

```json
{ "signal": "structural", "score": 4.25, "margin": 0.75 }
```

`signal` records the evidence channel that produced the hit and is never inferred from its final rank:

| Signal | Producers |
|---|---|
| `exact` | lexical, literal, word, and regex text hits |
| `structural` | definitions, callers, graph edges, anchors, imports, and AST patterns |
| `semantic` | embedding similarity hits |

A high semantic score remains labeled `semantic`; fusion and reranking cannot present it as exact or structural evidence. `kind` remains the more specific producer such as `def`, `pattern`, or `embed`.

`margin` is the finite, non-negative score separation from the next lower candidate in the same signal channel after deduplication and file filtering. A mathematical difference beyond the finite `f64` range saturates at `f64::MAX`. The final candidate in a channel has margin `0`. Every member of a score tie also has margin `0`, avoiding false confidence. Margins compare candidates within one channel only; they are not probabilities and must not be compared across signals.

Native, GitHub, GitLab, agent, agent-capsule, MCP, and LSP JSON surfaces preserve `signal`, `score`, and `margin`. Human line formatting remains compact; consumers that need confidence metadata should request JSON.

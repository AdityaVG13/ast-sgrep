# Search signal provenance and margins

Every returned hit carries four required provenance/ranking fields:

```json
{ "signal": "structural", "contributors": ["def", "embed"], "score": 4.25, "margin": 0.75 }
```

`signal` records the evidence channel that produced the hit and is never inferred from its final rank:

| Signal | Producers |
|---|---|
| `exact` | lexical, literal, word, and regex text hits |
| `structural` | definitions, callers, graph edges, anchors, imports, and AST patterns |
| `semantic` | embedding similarity hits |

A high semantic score remains labeled `semantic`; fusion and reranking cannot present it as exact or structural evidence. `kind` remains the canonical producer such as `def`, `pattern`, or `embed`. For hybrid results, `contributors` records every positive producer kind fused at the same file/start-line identity. Suppressed evidence is omitted.

`margin` is the finite, non-negative score separation from the next lower candidate in the same signal channel after deduplication and file filtering. A mathematical difference beyond the finite `f64` range saturates at `f64::MAX`. The final candidate in a channel has margin `0`. Every member of a score tie also has margin `0`, avoiding false confidence. Margins compare candidates within one channel only; they are not probabilities and must not be compared across signals.

Native, GitHub, GitLab, agent, agent-capsule, MCP, and LSP JSON surfaces preserve `signal`, `contributors`, `score`, and `margin`. The opt-in `compact` format preserves signal as a one-byte code and rank order while intentionally omitting contributor, score, and margin decoration. Human line formatting remains compact; consumers that need full confidence metadata should request another JSON format.

Legacy JSON without `signal`, `contributors`, or `margin` remains decodable. Supplied provenance is untrusted: decoding re-derives the signal and initial contributor from `kind`. Runtime fusion replaces that initial value with the sorted positive contributor set.

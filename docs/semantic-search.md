# Semantic search — the “S” in ast-sgrep

ast-sgrep’s semantic layer answers **intent queries** when the words in your question do not appear in the code — *“credential renewal”* finding `auth_refresh`, *“sanitize user input”* finding `validate_input`. It is **on by default** and works **offline without an API key**.

## Why symbol chunks, not lines

Line-level embeddings treat every line equally. Code navigation needs **symbols in context**:

```
symbol: auth_refresh kind: function
called_by: main
calls: fetch_token store_token
excerpt: fn auth_refresh() { ... }
```

Each extracted symbol becomes an enriched text chunk, embedded into a vector stored in `semantic_chunks`. At search time, the query is embedded with the same provider and compared via cosine similarity (or IVF-ANN at scale).

This is what makes ast-sgrep semantic search **code-aware**: similarity reflects naming, neighborhood in the call graph, and excerpt content — not just adjacent lines in a file.

## Concept expansion

Before embedding, chunks are expanded with **code-domain concept groups** — synonym clusters tuned for software vocabulary:

| Concept group | Related terms |
|---------------|---------------|
| Auth / credentials | auth, credential, token, session, login |
| Refresh / renewal | refresh, renew, rotate, update |
| Validation / sanitization | validate, sanitize, check, verify |
| Persistence / storage | persist, store, save, cache |

Expansion is applied in the offline **semantic local** embedder (char n-grams + concept tokens). Neural backends (Ollama, cloud) rely on model semantics but still index the enriched chunk text.

## Provider chain

At **index** and **search** time, the same chain is used:

```
1. Cloud     — if ASGREP_EMBED_API_KEY is set
2. Ollama    — if Ollama is reachable (ASGREP_OLLAMA_URL)
3. Semantic local — always available (256-dim, offline)
```

| Backend | Flag | Env |
|---------|------|-----|
| Auto (chain) | (default) | — |
| Cloud | `--cloud-embed` | `ASGREP_EMBED_API_KEY`, `ASGREP_CLOUD_EMBED=1` |
| Ollama | `--ollama-embed` | `ASGREP_OLLAMA_URL`, `ASGREP_OLLAMA_EMBED=1` |
| Semantic only | `--semantic-only` | `ASGREP_SEMANTIC_ONLY=1` |
| Disabled | `--no-embed` | `ASGREP_NO_EMBED=1` |

`asgrep status` reports the stored `embed_backend` and `embed_dim`. For best results, query with the same backend used at index time.

### Semantic local (default, no API key)

- 256-dimensional vectors
- Char n-gram features + concept expansion
- Deterministic, offline, fast
- Regression-tested: zero token-overlap queries must rank the correct symbol

### Ollama (optional)

```bash
asgrep --ollama-embed index .
# Default model: nomic-embed-text via ASGREP_OLLAMA_URL
```

### Cloud (optional)

```bash
export ASGREP_EMBED_API_KEY=sk-...
asgrep --cloud-embed index .
```

OpenAI-compatible embedding API. Dimension depends on model; stored in index metadata.

## Search passes

### Hybrid (default)

Semantic is one pass among lexical, symbol, graph, and anchor. Semantic hits appear as kind `EMBED` in output.

```bash
asgrep "credential renewal"
```

### Semantic-only

Skips lexical/symbol/graph passes; useful for pure synonym or NL probes.

```bash
asgrep semantic "credential renewal" --json
```

With `--json`, defaults to **agent** format.

## Scale: brute force vs IVF-ANN

| Corpus | Strategy | Latency |
|--------|----------|---------|
| &lt; `ann_threshold` symbols (default 2000) | Brute-force cosine over all vectors | Sub-millisecond |
| ≥ threshold | IVF-ANN with persisted `.asgrep/semantic.ivf` | Fast approximate NN; no k-means rebuild on restart |

Tune threshold:

```bash
asgrep --ann-threshold 5000 index .
# or ASGREP_ANN_THRESHOLD=5000
```

The IVF sidecar stores cluster centroids and vector layout. On reindex, a **fingerprint** mismatch invalidates the sidecar and triggers rebuild.

LSP `initializationOptions` also accepts `annThreshold` — see [use-cases.md](use-cases.md).

## Disabling semantic

```bash
asgrep --no-embed index .
asgrep --no-embed "auth refresh"    # no EMBED hits
```

Useful for lexical-only workflows or comparing behavior.

## Verification

The regression suite includes zero token-overlap cases:

```bash
cargo test -p ast-sgrep-core --test semantic
```

Manual smoke:

```bash
asgrep index tests/fixtures/sample
asgrep "credential renewal" tests/fixtures/sample
# Expect auth_refresh in results (EMBED and/or ANCHOR/DEF)
```

## JSON: semantic metadata

Agent format exposes semantic signal explicitly:

```json
{
  "has_semantic_hits": true,
  "hits": [{
    "kind": "embed",
    "semantic": true,
    "symbol": "auth_refresh",
    "score": 3.42,
    "follow_up_queries": ["defs:auth_refresh", "callers:auth_refresh"]
  }]
}
```

LSP `workspace/symbol` includes `detail: "semantic · score 3.42"` and `data.semantic: true` for embed hits.

## Related docs

- [Getting started](getting-started.md) — flags and first queries
- [How it works](how-it-works.md) — full pipeline and schema
- [Use cases](use-cases.md) — agent loops and LSP semantic commands

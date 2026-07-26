# Semantic search, the “S” in ast-sgrep

ast-sgrep’s semantic layer answers **intent queries** when the words in your question do not appear in the code, *“credential renewal”* finding `auth_refresh`, *“sanitize user input”* finding `validate_input`. It is **on by default** and works **offline without an API key**.

## Why child chunks mapped to symbols

Whole-file or independent line embeddings lose code structure. ast-sgrep embeds bounded AST-derived child excerpts while keeping each child mapped to its enclosing function or method:

```
symbol: auth_refresh kind: function
called_by: main
calls: fetch_token store_token
excerpt: fn auth_refresh() { ... }
```

Each function or method contributes up to 32 distinct child spans. One-line functions retain their best nested child, and top-level nodes map to a bounded file parent. If extraction yields no function child, the parent excerpt is the fallback. Every child is enriched and embedded into `semantic_chunks`, but retains its parent symbol or file range.

At search time, child vectors are compared by cosine similarity (or IVF-ANN at scale), grouped by parent, and ranked by the maximum child score. One parent result is returned with up to three highest-scoring raw source children as its snippet; enrichment text is used only to produce vectors and is never exposed as source. This gives fine-grained matching without losing a meaningful read unit or letting a large function consume multiple result slots.

Schema version 6 clears legacy whole-symbol vectors, cached vectors, backend/model identity, and stored file fingerprints. The next index refresh rebuilds every file into the child-to-parent layout, so old and new layouts cannot mix. Backend model identity is persisted for semantic, neural, cloud, and Ollama vectors; indexing refreshes and search refuses stale vectors after a configured model change.

## Concept expansion

Before embedding, chunks are expanded with **code-domain concept groups**, synonym clusters tuned for software vocabulary:

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
1. Cloud    , if ASGREP_EMBED_API_KEY is set
2. Ollama   , if Ollama is reachable (ASGREP_OLLAMA_URL)
3. Semantic local, always available (256-dim, offline)
```

| Backend | Flag | Env |
|---------|------|-----|
| Auto (chain) | (default) |, |
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

Default search is a constraint cascade: lexical candidates must survive AST-derived symbol, graph, anchor, or pattern evidence before semantic chunks are ranked. Semantic hits appear as kind `EMBED`, but they cannot widen the survivor file set.

```bash
asgrep "auth refresh"
```

### Semantic-only

Skips lexical and structural gates; use this for pure synonym or zero-token-overlap NL probes.

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

LSP `initializationOptions` also accepts `annThreshold`, see [use-cases.md](use-cases.md).

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
    "signal": "semantic",
    "margin": 0.18,
    "semantic": true,
    "symbol": "auth_refresh",
    "score": 3.42,
    "follow_up_queries": ["defs:auth_refresh", "callers:auth_refresh"]
  }]
}
```

LSP `workspace/symbol` includes `detail: "semantic · score 3.42 · margin 0.18"` and `data.signal`, `data.score`, and `data.margin` for every hit. `data.semantic` remains available for compatibility.

## Related docs

- [Getting started](getting-started.md), flags and first queries
- [How it works](how-it-works.md), full pipeline and schema
- [Use cases](use-cases.md), agent loops and LSP semantic commands

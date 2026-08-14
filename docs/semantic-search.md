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

Schema version 6 clears legacy whole-symbol vectors, cached vectors, backend/model identity, and stored file fingerprints. The next index refresh rebuilds every file into the child-to-parent layout, so old and new layouts cannot mix. Backend model identity is persisted for hashed semantic and in-process neural vectors; indexing refreshes and search refuses stale vectors after a configured model change. Indexes that still record `cloud` or `ollama` hard-error until `asgrep reindex`.

## Concept expansion

Before embedding, chunks are expanded with **code-domain concept groups**, synonym clusters tuned for software vocabulary:

| Concept group | Related terms |
|---------------|---------------|
| Auth / credentials | auth, credential, token, session, login |
| Refresh / renewal | refresh, renew, rotate, update |
| Validation / sanitization | validate, sanitize, check, verify |
| Persistence / storage | persist, store, save, cache |

Expansion is applied in the offline **semantic local** embedder (char n-grams + concept tokens). In-process neural still indexes the enriched chunk text; the model supplies the similarity geometry.

## Provider chain

At **index** and **search** time, the same in-process chain is used:

```
1. Neural, if built with --features neural-embed and ASGREP_NEURAL_EMBED is set
2. Semantic local, always available (offline hashed embedder; see dimension note below)
```

There is no cloud or Ollama embed client. Source text never leaves the process for embeddings.

| Backend | Flag | Env |
|---------|------|-----|
| Auto (chain) | (default) | |
| Neural | `--neural-embed` | `ASGREP_NEURAL_EMBED=1` |
| Semantic only | `--semantic-only` | `ASGREP_SEMANTIC_ONLY=1` |
| Disabled | `--no-embed` | `ASGREP_NO_EMBED=1` |

Concurrent backend flags (CLI `--neural-embed --semantic-only`, LSP `neuralEmbed` + `semanticOnly`, or several `SearchOptions::use_*` trues) collapse to one backend: **Neural > Semantic > Auto**. Explicit Neural does not silently swap to hashed unless `ASGREP_NEURAL_FALLBACK=1`.

`asgrep status` reports the stored `embed_backend` and `embed_dim`. For best results, query with the same backend used at index time.

### Semantic local (default, no API key)

- Vectors are stored as length-`SEMANTIC_DIM` (**256**) `f32` arrays
- **Honesty note:** sign bits come from a 32-byte BLAKE3 digest (`hash_feature` walks `i % 32`), so the independent sign pattern has period 32 until a denser feature hash lands. Treat “256-dim” as the storage width, not 256 independent random projections.
- Char n-gram features + concept expansion
- Deterministic, offline, fast
- Regression-tested: zero token-overlap queries must rank the correct symbol on the fixture suite (not a statistical guarantee on arbitrary corpora)

### Neural (optional, in-process)

```bash
# Requires a build with --features neural-embed
export ASGREP_NEURAL_EMBED=1
asgrep --neural-embed index .
```

ONNX MiniLM / BGE via `fastembed` (default `all-minilm-l6-v2-q`, 384-d). First load may download weights into `ASGREP_NEURAL_CACHE_DIR` unless the model is already cached. This is not an HTTP embedding API: inference stays in-process.

## Search passes

### Hybrid (default)

Default search is a constraint cascade: lexical candidates bound the file set; structural evidence narrows it when present. If structural is empty, hybrid continues on lexical survivors and may still rank semantic chunks inside that set (see `docs/cascade-query-planner.md`). Semantic hits appear as kind `EMBED`, but they cannot widen beyond the working-file set.

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

Adaptive search probes at most 90% of populated clusters by default. The bound
is deliberate: the 2048-vector quality fixture misses the 0.99 recall target at
75%, while 90% restores exact top-10 recall and remains below the 95% candidate
ceiling.

Release-mode RCH measurements use 64 deterministic queries at dimension 32:

| vectors | probes | recall@10 | average query | candidate fraction |
|--------:|-------:|----------:|--------------:|-------------------:|
| 2,048 | 50% | 0.931250 | 276.794 µs | 0.511459 |
| 2,048 | 75% | 0.989062 | 296.533 µs | 0.754547 |
| 2,048 | default ≤90% | 0.998437 | 325.768 µs | 0.888893 |
| 10,000 | 50% | 0.982812 | 565.221 µs | 0.499580 |
| 10,000 | 75% | 0.996875 | 694.175 µs | 0.749023 |
| 10,000 | default ≤90% | 1.000000 | 780.488 µs | 0.899686 |

Full-cluster reference latency was 323.250 µs at 2,048 vectors and 849.215 µs
at 10,000 vectors on the same run. Timings are comparative within that run;
the enforced invariant is recall@10 at least 0.99 with no more than 95% of
candidates. `--ann-probes` can still request an explicit probe count.

Those µs columns are **host-comparative / `UNREPRODUCIBLE` as a universal SLO**.
The fail-closed gate is recall@10 ≥ 0.99 and candidate fraction ≤ 0.95 at the
default ≤90% probe, for both 2,048 and 10,000 vectors:

```bash
cargo test -p ast-sgrep-core --release --test semantic_ivf_roundtrip \
  adaptive_ivf_tradeoff_at_2048_and_10000_vectors -- --ignored --nocapture
```

PR CI already runs `adaptive_ivf_recall_at_10_stays_within_quality_error_budget`
(2,048 vectors, un-ignored). The 10k tradeoff stays `#[ignore]` on PRs and runs
hard-fail on the `ann-ivf-scale` `workflow_dispatch` job (`lbx1.7`).

Tune threshold:

```bash
asgrep --ann-threshold 5000 index .
# or ASGREP_ANN_THRESHOLD=5000
```

The version-2 IVF sidecar stores a bounded cluster index followed by 4096-byte-aligned vectors. Open validates and decodes the cluster metadata, then retains the vector payload as a read-only mmap; it does not deserialize vectors into heap memory. Atomic temp-file publication keeps existing mappings valid, and a **fingerprint** mismatch triggers rebuild. Language-filtered searches use their filtered in-memory vectors and never overwrite the shared global sidecar.

On a 10,000-vector medium fixture, measured p99 was 0.963 ms cold, 0.135 ms for a fresh inode under normal cache policy, and 0.037 ms warm. Methodology and byte accounting are recorded in [semantic IVF mmap validation](validation/semantic-ivf-mmap.md).

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

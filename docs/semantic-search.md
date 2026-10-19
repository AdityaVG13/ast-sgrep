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

Each chunk also stores separate vectors for its name metadata, documentation, body, graph neighborhood, and tests or usage examples. Test/example text is recognized from conventional test/example paths and symbols, plus example-bearing documentation. Conceptual queries weight docs, body, and examples; symbol queries weight names; structural behavior queries weight body, graph, and examples. Search ranks concatenated chunk vectors first (from the IVF mmap at scale), then fetches and rescores only the top-N survivors, and only the intent-weighted field columns. JSON embed hits expose those weighted similarities in `embed_fields`, and human-readable evidence includes `embed_field:<field>=<score>` for weighted fields only. Literal intent keeps the concatenated score and omits field terms.

Schema version 6 clears legacy whole-symbol vectors, cached vectors, backend/model identity, and stored file fingerprints. The next index refresh rebuilds every file into the child-to-parent layout, so old and new layouts cannot mix. Model identity is persisted (`embed_backend=static`, `embed_model=static:...:256`); indexing refreshes and search refuses stale vectors after a model change. Indexes written by retired backends (hashed `semantic-v2`, neural, `cloud`, `ollama`) are detected on open: the next index run re-embeds every file from the stored chunk text, and search hard-errors until then rather than serving stale vectors.

## Concept expansion

Before embedding, chunks are expanded with **code-domain concept groups**, synonym clusters tuned for software vocabulary:

| Concept group | Related terms |
|---------------|---------------|
| Auth / credentials | auth, credential, token, session, login |
| Refresh / renewal | refresh, renew, rotate, update |
| Validation / sanitization | validate, sanitize, check, verify |
| Persistence / storage | persist, store, save, cache |
| Throttle / rate limit | throttle, rate_limit, quota, inbound |
| Debounce / coalesce | debounce, coalesce, noisy, watch |
| Retry / backoff | retry, backoff, transient, attempt |

Expansion is applied to both chunk enrichment text at index time and the query at search time, so a query about “rate-limit inbound requests” also activates throttle/quota vocabulary in the embedding space.

## The static engine

There is exactly one embedding backend: a **static (Model2Vec-style) engine**
compiled into the binary. No model download, no ONNX runtime, no network call.
Source text never leaves the process for embeddings.

- Vocab: 63k-entry WordPiece (BERT normalization + PascalCase/camelCase
  segmentation pass tuned for code identifiers).
- Embedding input per chunk field: enriched text (symbol name, docs, excerpt,
  graph neighborhood) run through concept expansion, then WordPiece-tokenized
  by the engine.
- Pooling: token table lookup with SIF weighting, mean-pooled, L2-normalized
  to 256-d `f32`. Deterministic bit-for-bit across machines; indexes are
  portable without re-embedding.
- Model identity: `static:potion-base-32m:256` (weights: MIT
  `minishlab/potion-base-32M`, hash-pinned at build; provenance in
  `crates/ast-sgrep-embed/assets/potion-base-32m.README.md`).

| Mode | Flag | Env |
|------|------|-----|
| On (default) | (default) | |
| Semantic only | `--semantic-only` | `ASGREP_SEMANTIC_ONLY=1` |
| Disabled | `--no-embed` | `ASGREP_NO_EMBED=1` |

`asgrep status` reports the stored `embed_backend` and `embed_dim`.

Retired surface: `--neural-embed`, `--rerank`, `--rerank-top-k`,
`ASGREP_EMBED_BACKEND`, and `ASGREP_NEURAL_EMBED` are rejected with an explicit
error naming the removal (they are kept as hidden parse stubs so the error is
actionable, not “unknown flag”).

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

Adaptive search probes at most 90% of populated clusters by default on corpora
up to 10,000 vectors. The bound is deliberate: the 2048-vector quality fixture
misses the 0.99 recall target at 75%, while 90% restores exact top-10 recall and
remains below the 95% candidate ceiling. Above 10,000 vectors, nprobe is capped
at 8 so unique-query scoring stays under 1 ms (16 probes was p90 1.2 ms on the
54k-chunk corpus). The IVF payload is prefaulted on first load so unique-query
p90 is not a cold page-fault walk. Hybrid search scores only mmap rows whose
files survived the lexical/structural cascade, then SQLite-fetches those top-N
survivors, not every concatenated vector in the candidate files. Hybrid cascade
prefilter skips 1-2 character tokens (they cannot use trigrams and would
full-table `LIKE` scan). `--ann-probes` still requests an explicit probe count.

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

Delta `asgrep index` after a file edit reassigns every current vector to the existing IVF centroids and rewrites cluster postings. It does not rerun k-means. Centroids stay frozen until `asgrep reindex` (or an embedding-identity rewrite) rebuilds them. Search still refuses a sidecar whose fingerprint no longer matches the store.

The 2026-10-11 candidate refresh separates writer-no-cache/fresh-process,
fresh-inode/fresh-process, page-cached/fresh-process and in-process warm opens
on the existing 10,000-vector fixture. Current numbers and probe hashes are in
[the canonical matrix](../benchmarks/results/baselines.md#260-candidate-refresh-2026-10-11);
[semantic IVF mmap validation](validation/semantic-ivf-mmap.md) retains the old
protocol for history. Fresh-process opens are not in-process warm opens.

**Historical / UNREPRODUCIBLE (`idx_big`, not the current candidate):** the
54,732-chunk corpus, original 85-query battery and fingerprint are unavailable;
[the current refresh](../benchmarks/results/baselines.md#260-candidate-refresh-2026-10-11)
does not replace them with invented inputs. In that historical run, unique-query
`asgrep semantic` was
**p50 0.51 ms / p90 0.74 ms** (n=85, `codemode-serve`, limit 8). Default hybrid
on the same unique-query set is **p50 1.27 ms / p90 8.4 ms**: the IVF mmap path
is tens of microseconds; remaining hybrid time is lexical discovery plus
finish/fanout, not nprobe. High-df conceptual terms (for example `encode
payload`) still sit in the p90 tail. `pi-ast-sgrep` Code Mode `asgrep.search`
uses the hybrid path; `asgrep.semantic` uses the pure semantic path. The
historical sub-millisecond figure is not a current latency guarantee.

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

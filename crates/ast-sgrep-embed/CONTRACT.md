# `ast-sgrep-embed` contract

The one embedding engine for ast-sgrep: a native static (Model2Vec-style)
embedder — trained token table baked into the binary, WordPiece tokenize,
SIF-weighted mean-pool, L2-normalized 256-d output. No backend selection, no
model download, no feature flags.

- Purpose/layer: leaf library. Core calls `embed_text` / `embed_texts` for
  chunk fields and queries; the crate knows nothing about indexes or search.
- Public surface: `embed_text`, `embed_texts`, `StaticEmbedder`,
  `STATIC_DIM` (256), `STATIC_MODEL_ID` (`static:potion-base-32m:256`),
  shared vector math (`cosine_similarity`, `top_by_similarity`, …), text
  utilities (`tokenize`, `split_ident`, `expand_concepts`), and little-endian
  `embed_to_bytes` / `embed_from_bytes` for storage.
- Invariants: output is deterministic bit-for-bit across machines (same
  input text → same vector bytes), so indexes are portable without
  re-embedding; embedding is infallible (unknown tokens pool to zero
  contribution, never an error); the baked table and vocab are hash-pinned
  at build (`build.rs`) — a silent asset swap fails the build.
- Error model: none on the embed path. The only fallible call is
  `embed_from_bytes` (storage decode), returning `&'static str`.
- Determinism class: fully deterministic, offline, no cancellation surface
  (single-shot pure functions).
- Unsafe boundary: none — `#![forbid(unsafe_code)]`.
- Model identity changes: any change to the table, vocab, tokenizer, or
  pooling must bump `STATIC_MODEL_ID`; core keys stored vectors and cache
  rows by that id and re-embeds on mismatch.
- No-claim: weights are currently MIT `minishlab/potion-base-32M`
  (provenance: `assets/potion-base-32m.README.md`). Retrieval-quality claims
  live only in `benchmarks/results/`; this crate makes none.

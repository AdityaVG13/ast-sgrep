# `ast-sgrep-embed` contract

The one embedding engine for ast-sgrep: a native static (Model2Vec-style)
embedder — trained token table baked into the binary, WordPiece tokenize,
plain mean-pool, L2-normalized 256-d output. No backend selection, no
model download, no feature flags.

- Purpose/layer: leaf library. Core calls `embed_text` / `embed_texts` for
  chunk fields and queries; the crate knows nothing about indexes or search.
- Public surface: `embed_text`, `embed_texts`, `StaticEmbedder`,
  `STATIC_DIM` (256), `STATIC_MODEL_ID` (`static:potion-code-16M-v2:256`),
  shared vector math (`cosine_similarity`, `top_by_similarity`, …), text
  utilities (`tokenize`, `split_ident`, `expand_concepts`), and little-endian
  `embed_to_bytes` / `embed_from_bytes` for storage.
- Invariants: output is deterministic bit-for-bit across machines (same
  input text → same vector bytes), so indexes are portable without
  re-embedding; embedding is infallible (unknown tokens pool to zero
  contribution, never an error); the baked table and vocab ride in the
  binary via `include_bytes!` (the crate has no `build.rs`) — identity is
  pinned by `STATIC_MODEL_ID`, asserted in `tests/embed/static_engine.rs`,
  with asset bytes hashed in `assets/README.md`.
- Error model: none on the embed path. The only fallible call is
  `embed_from_bytes` (storage decode), returning `&'static str`.
- Determinism class: fully deterministic, offline, no cancellation surface
  (single-shot pure functions).
- Unsafe boundary: none — `#![forbid(unsafe_code)]`.
- Model identity changes: any change to the table, vocab, tokenizer, or
  pooling must bump `STATIC_MODEL_ID`; core keys stored vectors and cache
  rows by that id and re-embeds on mismatch.
- No-claim: weights are currently MIT `minishlab/potion-code-16M-v2`
  (provenance: `assets/README.md`; assets `potion-code-16m-v2.f16.bin`,
  `potion-code-16m-v2.vocab.bin`). Retrieval-quality claims
  live only in `benchmarks/results/`; this crate makes none.

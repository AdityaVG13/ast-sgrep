# Neural / ORT trust (`l115`)

Local neural embeddings (`--features neural-embed`) pull **ONNX Runtime** via
`ort` / `fastembed`. Trust boundary:

- Model weights download into `ASGREP_NEURAL_CACHE_DIR` (or XDG cache).
- No first-party `unsafe`; ORT internals are dependency code.
- Failures are not silent hashed swaps for explicit Neural preference unless
  `ASGREP_NEURAL_FALLBACK=1` (see `docs/env-trust.md`).
- CoreML EP is opt-in via `ASGREP_NEURAL_COREML`.

Dependency `unsafe` is inventoried by running cargo-geiger (no checked-in
baseline file); the `forbid(unsafe_code)` gate is authoritative for
first-party crates.

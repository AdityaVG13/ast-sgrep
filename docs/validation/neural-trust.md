# Neural / ORT trust (`l115`) — retired

The neural embedding backend (`--features neural-embed`, ORT via `ort` /
`fastembed`) was removed in Phase 2 of the native embed engine migration
(`docs/plans/native-embed-engine.md`). `ort` and `fastembed` are no longer in
the dependency tree; there is no model download and no ORT trust surface left
to audit. Model-weight provenance for the single static engine is documented
in `crates/ast-sgrep-embed/assets/README.md` and hash-pinned
at build time in `crates/ast-sgrep-embed/build.rs`.

The `forbid(unsafe_code)` gate remains authoritative for first-party crates.

# `ast-sgrep-watch` contract

Helper binary (`asgrep-watch`) implementing `asgrep watch`. Exists for one
reason: `notify` links CoreFoundation/CoreServices on macOS (~1.5ms dyld per
process), so the watcher lives here and the main binary never links it.

- Purpose/layer: leaf binary crate. `asgrep watch` re-execs this binary with
  verbatim argv; `main` re-parses via `ast_sgrep_cli::parse_watch_launch`
  (same parser as the shim, so index options cannot drift) and runs the
  moved `watch` loop unchanged.
- Public surface: none (bin-only, no lib). Deps: `ast-sgrep-cli` (arg parse),
  `ast-sgrep-core` (indexer), `notify` (file events, owned here).
- Invariants: never linked into `asgrep` (verify: `otool -L asgrep` shows no
  CoreFoundation/CoreServices); argv handling identical to the shim's parse;
  typo warnings not reprinted (the shim already printed them).
- Error model: `anyhow`; parse failures mirror main-binary clap rendering
  (direct hand-invocation only); non-`watch` commands bail with a pointer to
  `asgrep`.
- Determinism: same as `asgrep watch` (event-driven daemon; JSON responses
  from the indexer are unchanged).
- No-claim: install/packaging must ship this binary next to `asgrep`
  (release bundling, cargo-install docs, Homebrew); the shim fails with an
  actionable error when it is absent.

# Pass 3 — files changed

| Path | What changed |
|---|---|
| [`packages/pi/launcher/src/index.js`](../../../../packages/pi/launcher/src/index.js) | Guard/early-return flatten for `resolveHost`, `resolveBinary`, `resolveCodemodeAddon`; private helpers `isPathFallbackError`, `isOptionalHostMiss`, `readJsonFile`, `assertPackageFileChecksum`; Set membership for fallback/soft-null codes; shared checksum ladder |
| [`crates/ast-sgrep-core/src/index.rs`](../../../../crates/ast-sgrep-core/src/index.rs) | `update_paths` calls `should_skip_watch_path`; empty-rel/dir continues stay separate (no `files_skipped` bump); predicate preserves prior `\|\|` short-circuit order |

No public API signature changes. No commit performed (orchestrator constraint).

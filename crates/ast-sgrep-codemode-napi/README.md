# In-process Node-API bindings for ast-sgrep Code Mode.
#
# Build (from repo root):
#   cargo build -p ast-sgrep-codemode-napi --release
#   npm run build:native -w pi-ast-sgrep
#
# Pi loads the resulting `.node` and calls `Session` / `batch` with zero CLI spawn.

See `src/lib.rs` and `docs/codemode.md`.

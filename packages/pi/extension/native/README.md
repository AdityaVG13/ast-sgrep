# In-process Code Mode addon (NAPI)

This directory holds the platform `.node` binary built from
`crates/ast-sgrep-codemode-napi`.

```bash
# from repo root
cargo build -p ast-sgrep-codemode-napi --release
npm run build:native -w pi-ast-sgrep
```

Produces e.g. `ast-sgrep-codemode.linux-x64-gnu.node`. The Pi extension loads it
automatically and runs `CodeModeSession` in-process (no CLI spawn).

Binaries are **not** committed; release CI should attach them to the extension
or platform packages the same way `asgrep` CLI artifacts are shipped today.

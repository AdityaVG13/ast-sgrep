# ast-sgrep

Native command launcher used by `pi-ast-sgrep`. Most Pi users should install the extension instead:

```bash
pi install npm:pi-ast-sgrep
```

This package exposes the equivalent `asgrep` and `ast-sgrep` commands and selects one exact-version native package for the current host. Supported hosts at `1.1.0-alpha` are macOS arm64/x64, glibc Linux arm64/x64, and Windows x64. Alpine/musl Linux, Windows arm64, and other hosts are rejected explicitly. Installation and execution never download a binary, build Rust source, or search `PATH`.

The launcher validates native package identity, OS/CPU/libc metadata, exact version, executable state, and SHA-256 checksum before returning the binary path. `AST_SGREP_BINARY` is a developer override and is not needed for a packaged installation.

See the [Pi package guide](https://github.com/AdityaVG13/ast-sgrep/blob/main/docs/pi-package.md) for install/use/update/remove instructions, offline and security behavior, and recovery. MIT.

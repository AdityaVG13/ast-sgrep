# ast-sgrep

Native command launcher used by `pi-ast-sgrep`. Most Pi users should install the extension instead:

```bash
pi install npm:pi-ast-sgrep
```

This package exposes the equivalent `asgrep` and `ast-sgrep` commands and selects one exact-version native package for the current host. Supported hosts at `2.5.2` are macOS arm64/x64, glibc Linux arm64/x64, and Windows x64. Alpine/musl Linux, Windows arm64, and other hosts are rejected explicitly. Installation and execution never download a binary, build Rust source, or search `PATH`.

The launcher validates native package identity, OS/CPU/libc metadata, exact version, executable state, and SHA-256 checksums (CLI + Code Mode NAPI addon) before returning paths. `AST_SGREP_BINARY` / `ASGREP_CODEMODE_NAPI_PATH` are developer overrides and are not needed for a packaged installation.

The native artifact inventory also includes `asgrep-watch` (`asgrep-watch.exe` on Windows) next to the CLI. Artifact and prepack verification require a nonempty helper with its own checksum; `asgrep watch` discovers that sibling without `ASGREP_WATCH_BIN` or a Cargo-installed helper. This is a packaging change for the next native-family release; already-published `2.5.4` packages do not contain the helper.

To verify real target-local artifacts without publishing, run `ASGREP_TEST_NATIVE_DIR=/absolute/path/to/native-artifacts node --test tests/pi/launcher/npm-watch-e2e.test.mjs` from the repository. The directory must contain the CLI, watch helper, and NAPI addon for the current host. The test packs and installs local tarballs offline, disables PATH and auto-index fallback, observes a file-change query result, stops the watcher, and retains `watch-evidence.json` with the tarballs and fixture.

See the [Pi package guide](https://github.com/AdityaVG13/ast-sgrep/blob/main/docs/pi-package.md) for install/use/update/remove instructions, offline and security behavior, and recovery. MIT.

# Publishing to crates.io

## Install from crates.io

```bash
cargo install ast-sgrep-cli
cargo install ast-sgrep-lsp
```

Binaries: `asgrep`, `ast-sgrep`, `asgrep-lsp`

## Publish a release (maintainers)

1. Bump version in root `Cargo.toml` `[workspace.package]`
2. Publish in dependency order:

```bash
./scripts/publish.sh
```

Or manually:

```bash
cargo publish -p ast-sgrep-lang
cargo publish -p ast-sgrep-embed
cargo publish -p ast-sgrep-core
cargo publish -p ast-sgrep-cli
cargo publish -p ast-sgrep-lsp
```

## Crate dependency order

1. `ast-sgrep-lang`
2. `ast-sgrep-embed`
3. `ast-sgrep-core`
4. `ast-sgrep-cli`
5. `ast-sgrep-lsp`

Requires `cargo login <token>`.

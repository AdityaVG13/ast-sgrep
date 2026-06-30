# Publishing to crates.io

## Install from crates.io

```bash
cargo install ast-sgrep-cli
```

Binaries: `asgrep`, `ast-sgrep`

Optional LSP server:

```bash
cargo install ast-sgrep-lsp
```

## Publish a release (maintainers)

1. Bump version in root `Cargo.toml` `[workspace.package]`
2. Tag: `git tag v1.0.0 && git push origin v1.0.0`
3. GitHub Actions `publish.yml` publishes all crates in order
4. Or manually: `./scripts/publish.sh`

## Crate dependency order

1. `ast-sgrep-lang`
2. `ast-sgrep-embed`
3. `ast-sgrep-core`
4. `ast-sgrep-cli`
5. `ast-sgrep-lsp`

## Required secret

Set `CARGO_REGISTRY_TOKEN` in GitHub repository secrets.

# Releasing to crates.io

This repository publishes its public crates in dependency order. Publishing is irreversible: prepare and verify locally, then wait for explicit human approval before running any `cargo publish` command.

## Version policy

- All public `ast-sgrep-*` crates use one lockstep version from `[workspace.package]`. A release must not mix versions.
- Versions follow Semantic Versioning. While the current release is a `1.0.0-alpha.N` prerelease, incompatible changes require a new prerelease version and release notes; once `1.0.0` is stable, incompatible public API changes require a major version bump.
- Additive, backward-compatible functionality increments the minor version after 1.0; backward-compatible fixes increment the patch version. Prerelease iterations increment the prerelease identifier (for example, `alpha.0` to `alpha.1`).
- Every path dependency between publishable workspace crates must also specify the same explicit version, so packaged manifests resolve from crates.io.
- `ast-sgrep-testkit` is internal (`publish = false`) and is never published. Dev-dependencies on it are excluded from published dependency resolution.

## Preparation

1. Confirm the working tree contains the intended release and the workspace version is consistent:

   ```sh
   cargo metadata --no-deps --format-version 1
   ```

2. Package each public crate in leaf order. This verifies the crate archive and compiles from the archive without publishing anything:

   ```sh
   CARGO_BUILD_JOBS=1 cargo package --locked -p ast-sgrep-lang
   CARGO_BUILD_JOBS=1 cargo package --locked -p ast-sgrep-embed
   CARGO_BUILD_JOBS=1 cargo package --locked -p ast-sgrep-core \
     --config 'patch.crates-io.ast-sgrep-lang.path="crates/ast-sgrep-lang"' \
     --config 'patch.crates-io.ast-sgrep-embed.path="crates/ast-sgrep-embed"'
   CARGO_BUILD_JOBS=1 cargo package --locked -p ast-sgrep-plugins \
     --config 'patch.crates-io.ast-sgrep-lang.path="crates/ast-sgrep-lang"' \
     --config 'patch.crates-io.ast-sgrep-embed.path="crates/ast-sgrep-embed"' \
     --config 'patch.crates-io.ast-sgrep-core.path="crates/ast-sgrep-core"'
   CARGO_BUILD_JOBS=1 cargo package --locked -p ast-sgrep-lsp \
     --config 'patch.crates-io.ast-sgrep-lang.path="crates/ast-sgrep-lang"' \
     --config 'patch.crates-io.ast-sgrep-embed.path="crates/ast-sgrep-embed"' \
     --config 'patch.crates-io.ast-sgrep-core.path="crates/ast-sgrep-core"'
   CARGO_BUILD_JOBS=1 cargo package --locked -p ast-sgrep-cli \
     --config 'patch.crates-io.ast-sgrep-lang.path="crates/ast-sgrep-lang"' \
     --config 'patch.crates-io.ast-sgrep-embed.path="crates/ast-sgrep-embed"' \
     --config 'patch.crates-io.ast-sgrep-core.path="crates/ast-sgrep-core"' \
     --config 'patch.crates-io.ast-sgrep-plugins.path="crates/ast-sgrep-plugins"'
   CARGO_BUILD_JOBS=1 cargo package --locked -p ast-sgrep-mcp \
     --config 'patch.crates-io.ast-sgrep-lang.path="crates/ast-sgrep-lang"' \
     --config 'patch.crates-io.ast-sgrep-embed.path="crates/ast-sgrep-embed"' \
     --config 'patch.crates-io.ast-sgrep-core.path="crates/ast-sgrep-core"' \
     --config 'patch.crates-io.ast-sgrep-plugins.path="crates/ast-sgrep-plugins"' 
   ```

   The temporary `patch.crates-io` overrides let dependent archives verify before their unpublished leaf crates exist in the crates.io index; they do not alter packaged manifests. Add `--allow-dirty` only during local preparation when reviewing intentional, uncommitted release changes. Do not use it for the approved release commit.

3. Inspect each archive with `cargo package --list -p <crate>`. Confirm the root README and license metadata are present, no credentials or generated data are included, and docs.rs links point to the matching crate.

## Publish after explicit approval

Only a human release operator may run these commands. Publish exactly in this order, waiting for each command to succeed and for its version to become available from the crates.io index before continuing:

```sh
cargo publish --locked -p ast-sgrep-lang
cargo publish --locked -p ast-sgrep-embed
cargo publish --locked -p ast-sgrep-core
cargo publish --locked -p ast-sgrep-plugins
cargo publish --locked -p ast-sgrep-lsp
cargo publish --locked -p ast-sgrep-cli
cargo publish --locked -p ast-sgrep-mcp
```

Do not publish `ast-sgrep-testkit`. If any publish fails, stop; crates.io releases cannot be replaced or overwritten. Fix the problem, bump the lockstep version, repeat packaging, and resume only with a newly approved release.

## Post-publish verification

1. Confirm every published version on crates.io and docs.rs:
   - `https://crates.io/crates/<crate>`
   - `https://docs.rs/<crate>/<version>`
2. Install from crates.io into a clean temporary root and verify both CLI entry points:

   ```sh
   install_root="$(mktemp -d)"
   CARGO_INSTALL_ROOT="$install_root" cargo install ast-sgrep-cli --locked
   "$install_root/bin/asgrep" --version
   "$install_root/bin/ast-sgrep" --version
   ```

3. Run the GitHub Actions `Post-publish install smoke` workflow manually. It repeats `cargo install ast-sgrep-cli --locked` on Linux and macOS. This workflow is post-publish verification only; it must not be used before the release is visible on crates.io.

## Homebrew formula

The standalone source formula lives at `packaging/homebrew/ast-sgrep.rb`. It is pinned to `1.0.0-alpha.0` and intentionally contains a SHA-256 placeholder until the matching GitHub tag is published. After publishing the tag, calculate the archive digest and replace the all-zero `sha256` placeholder in the formula:

```sh
version="1.0.0-alpha.0"
url="https://github.com/AdityaVG13/ast-sgrep/archive/refs/tags/v${version}.tar.gz"
curl --fail --location --silent --show-error "$url" --output "ast-sgrep-v${version}.tar.gz"
shasum -a 256 "ast-sgrep-v${version}.tar.gz"
```

The placeholder prevents an accidental install against an unverified archive. Do not publish the formula to a tap until the digest has been replaced and the formula passes validation.

For local source installation after replacing the digest, run:

```sh
brew install --build-from-source ./packaging/homebrew/ast-sgrep.rb
```

The formula invokes `cargo install --locked` through Homebrew's `std_cargo_args` helper for the `crates/ast-sgrep-cli` package, installs `asgrep`, and checks `asgrep --version` in its test block.

### Homebrew tap follow-ups

After the formula is validated in this repository, publishing it to a public
Homebrew tap (and any umbrella formulas that depend on `ast-sgrep`) is a
separate human release step. This repository does not push external taps.

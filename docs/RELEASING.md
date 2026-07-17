# Releasing

This repository publishes npm packages and public crates from one source commit with separate, explicit release identities. Publishing is irreversible: prepare and verify locally, then wait for explicit human approval before any external registry command or protected release-environment approval.

## Pi npm package family

The npm release is one atomic versioned family at `1.1.0-alpha.1`, published only from the human-approved official `v1.1.0-alpha.1` tag and commit:

1. the five host-constrained native packages: `ast-sgrep-darwin-arm64`, `ast-sgrep-darwin-x64`, `ast-sgrep-linux-arm64-gnu`, `ast-sgrep-linux-x64-gnu`, and `ast-sgrep-win32-x64-msvc`;
2. the `ast-sgrep` launcher, whose optional native dependencies use that exact version;
3. the `pi-ast-sgrep` extension, whose launcher dependency uses that exact version.

The extension, launcher, and five native npm packages share npm version `1.1.0-alpha.1`. The packaged executable is built from this PR base and reports native CLI version `1.1.0-alpha`; that expected CLI identity is recorded separately in [the release contract](../packages/pi/release-contract.json). All artifacts still share one source commit and recorded checksums. Pi validation does not run automatically on pull requests, pushes to `main`, or tag pushes; both Pi workflows are manual `workflow_dispatch` actions. npm and crates.io are independently approved registry operations over the same source release; neither waits for or proves completion of the other.

Local preparation is side-effect free:

```bash
npm run check:pi-contract
npm run check:pi-release
npm run test:pi-release-gate
npm run test:pi-e2e
```

The release-gate and E2E commands exercise packed artifacts and the official Pi loader without publishing. Package-level `npm pack --dry-run`/`npm pack` preparation is allowed; do not run `npm publish` locally. The manual **Pi native artifacts** workflow (`.github/workflows/pi-native-artifacts.yml`) is dry-run only. The tag-only **Pi npm official release** workflow (`.github/workflows/pi-npm-release.yml`) is the canonical publisher. External npm publication requires explicit human approval of its protected `npm-production` environment and trusted-publishing OIDC/provenance. Before the first publication, re-verify every npm name and publisher ownership; a prior 404 is not a reservation. Publish native packages before the launcher and the launcher before the extension.

If publication stops after an immutable npm version becomes visible, do not overwrite it or finish a mixed family. Correct the cause, advance every coupled component to a new version, repeat all checks, obtain new approval, and publish the complete family in order.

## Version policy

- All public `ast-sgrep-*` crates use one lockstep version from `[workspace.package]`. A release must not mix versions.
- Versions follow Semantic Versioning. While the project is in alpha (currently `1.1.0-alpha.1`), incompatible changes require a new alpha version and release notes; once `1.0.0` stable ships, incompatible public API changes require a major version bump.
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

Only a human release operator may run this block. It is noninteractive and publishes the seven public crates in the only valid leaf order. It waits until each immutable version is resolvable from the crates.io index before publishing a dependent crate.

```bash
set -euo pipefail
release_version='1.1.0-alpha.1'
release_crates=(
  ast-sgrep-lang
  ast-sgrep-embed
  ast-sgrep-core
  ast-sgrep-plugins
  ast-sgrep-lsp
  ast-sgrep-cli
  ast-sgrep-mcp
)
start_at="${START_AT:-${release_crates[0]}}"

case " ${release_crates[*]} " in
  *" ${start_at} "*) ;;
  *) printf 'unknown START_AT crate: %s\n' "$start_at" >&2; exit 2 ;;
esac

publishing=false
for crate in "${release_crates[@]}"; do
  if [[ "$crate" == "$start_at" ]]; then
    publishing=true
  fi
  $publishing || continue

  cargo publish --locked -p "$crate"
  until cargo info --registry crates-io "${crate}@${release_version}" >/dev/null 2>&1; do
    sleep 15
  done
done
```

For a new release, run the block without `START_AT`; the first external command is `cargo publish --locked -p ast-sgrep-lang`. If the process stops after one or more successful publications, first identify the first crate in `release_crates` whose exact version is not on crates.io, then rerun the unchanged approved source with `START_AT=<that-crate>`. Never point `START_AT` at a version already published: crates.io versions cannot be overwritten.

Do not publish `ast-sgrep-testkit`. A transient failure before a crate is accepted may be retried from that crate with the same approved source. If any source, manifest, or lockfile must change, stop the release: bump the workspace version, update every workspace dependency and lockfile entry, repeat all preparation, and obtain new approval before publishing the new version from the beginning.

## Post-publish verification

1. Verify the exact immutable version exists for all seven crates and that docs.rs has completed each build:

   ```bash
   set -euo pipefail
   release_version='1.1.0-alpha.1'
   release_crates=(ast-sgrep-lang ast-sgrep-embed ast-sgrep-core ast-sgrep-plugins ast-sgrep-lsp ast-sgrep-cli ast-sgrep-mcp)
   for crate in "${release_crates[@]}"; do
     cargo info --registry crates-io "${crate}@${release_version}" >/dev/null
     curl --fail --location --silent --show-error --output /dev/null \
       "https://docs.rs/${crate}/${release_version}/"
   done
   ```

2. Install the exact version from crates.io into a new temporary root, independently of the checkout, and record both version outputs as clean-install evidence:

   ```bash
   set -euo pipefail
   release_version='1.1.0-alpha.1'
   install_root="$(mktemp -d)"
   cargo install ast-sgrep-cli --version "=${release_version}" --locked --root "$install_root"
   asgrep_version="$("$install_root/bin/asgrep" --version)"
   ast_sgrep_version="$("$install_root/bin/ast-sgrep" --version)"
   printf '%s\n%s\n' "$asgrep_version" "$ast_sgrep_version"
   [[ "$asgrep_version" == *" ${release_version}" ]]
   [[ "$ast_sgrep_version" == *" ${release_version}" ]]
   ```

3. Run the GitHub Actions `Post-publish install and docs smoke` workflow manually with `version` set to `1.1.0-alpha.1`. It installs the exact crates.io CLI version into an empty temporary root on Linux and macOS, checks both binaries, and verifies the exact-version docs.rs page for every published crate. Save the successful workflow URL with the release record. This workflow is post-publish evidence only; do not run it before the release is visible on crates.io.

## Homebrew formula

The standalone source formula lives at `packaging/homebrew/ast-sgrep.rb`. It is pinned to `1.1.0-alpha.1` and intentionally contains a SHA-256 placeholder until the matching GitHub tag is published. After publishing the tag, calculate the archive digest and replace the all-zero `sha256` placeholder in the formula:

```sh
version="1.1.0-alpha.1"
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

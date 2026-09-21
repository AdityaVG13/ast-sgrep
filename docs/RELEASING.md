# Releasing

This repository publishes npm packages and public crates from one source commit with separate, explicit release identities. Publishing is irreversible: prepare and verify locally, then wait for explicit human approval before any external registry command or protected release-environment approval.

## Official release happy path (copy-paste)

One terminal, five steps. Everything below the checklist is reference for when something deviates.

```bash
# 1. Mechanical bump (workspace, npm family + extension, contract, constants,
#    dist) plus the check battery, then do the human remainder it prints
#    (CHANGELOG prose, README, docs pointers, launcherRange judgment):
npm run release:prepare X.Y.Z
npm run check:pi-contract && npm run check:pi-dist && npm run check:pi-release
git commit -am "release: ast-sgrep X.Y.Z" && git push origin main

# 2. The release moment: verifies the prepared tree, runs preflight, signs
#    and pushes the tag, dispatches the family lane, and opens the run page:
npm run release -- X.Y.Z          # append --dry-run to print the plan only

# 3. Wait ~30 min for builds + verify, then approve the one
#    `npm-production` deployment gate on the run page ("Review pending
#    deployments"). Nothing else needs a click.

# 4. Verify all seven packages are live at the release versions:
for p in ast-sgrep @ast-sgrep/darwin-arm64 @ast-sgrep/darwin-x64 \
         @ast-sgrep/linux-arm64-gnu @ast-sgrep/linux-x64-gnu \
         @ast-sgrep/win32-x64-msvc pi-ast-sgrep; do
  printf '%-32s %s\n' "$p" "$(npm view "$p" version 2>/dev/null)"
done
```

Rules of the road: never dispatch from `main` (the gate requires tag == checkout == workflow commit); never move a tag after publication starts — a broken pre-publish tag may be re-signed and force-pushed again only while nothing is published, and every move must be followed by clearing that tag's unconsumed npm release assets (fresh commit SHA + fresh Windows timestamps would fail the byte-compare); reruns are idempotent (live tarballs with matching integrity are skipped, order is native → launcher → extension); to retry only the publish steps without rebuilding, dispatch the same tag with `-f mode=publish-only` (family lane; reuses the preserved release assets and refuses when they are missing or differ). Publication is OIDC-only (br-r8k): there is no token lane, so an OIDC failure means the npm trusted-publisher registration is wrong — fix it on npmjs.com and re-dispatch publish-only. Brand-new package names cannot use this flow until a human publishes them once manually (`npm publish` with 2FA) and registers the trusted publisher. After either publish job succeeds, the workflow re-pins `package-lock.json` against the live release and pushes it to `main` (br-zvh); this runs under the publish approval, not a second one.

## Pi npm package family

The npm release publishes the canonical family at the contract's canonical version, only from the human-approved official tag and commit:

1. the five host-constrained native packages: `@ast-sgrep/darwin-arm64`, `@ast-sgrep/darwin-x64`, `@ast-sgrep/linux-arm64-gnu`, `@ast-sgrep/linux-x64-gnu`, and `@ast-sgrep/win32-x64-msvc`;
2. the `ast-sgrep` launcher, whose optional native dependencies use that exact version.

The `pi-ast-sgrep` extension rides the lockstep family: `release:prepare` bumps it with everything else, and the family lane publishes it last. A `pi-v` extension lane remains for out-of-band extension revs (see below); it resolves its launcher through the declared `packages.extension.launcherRange`. The packaged executable is built from the release commit and reports the native CLI version recorded separately in [the release contract](../packages/pi/release-contract.json). Family artifacts share one source commit and recorded checksums. Pi validation does not run automatically on pull requests, pushes to `main`, or tag pushes; both Pi workflows are manual `workflow_dispatch` actions. npm and crates.io are independently approved registry operations over the same source release; neither waits for or proves completion of the other.

## Pi extension lane (pi-ast-sgrep)

`pi-ast-sgrep` is the primary dogfooding surface for the pi integration. It normally releases in lockstep with the family; the lane below covers out-of-band revs (extension-only fixes between family releases). The launcher, the five platform packages, and the embedded CLI remain lockstep; the extension does not carry binaries and resolves its launcher through the declared `packages.extension.launcherRange` (currently `>=2.0.0 <3`), so installs automatically pick up newer native families once they ship while remaining schema-guarded at runtime.

Rules for the extension lane:

- The extension version lives in two places that MUST agree: `packages/pi/extension/package.json` and `packages/pi/release-contract.json` at `packages.extension.version`. Bump both together.
- Extension releases require a signed annotated tag in the `pi-v<version>` namespace (for example `pi-v2.5.2`) on the intended commit, then a manual `pi-npm-release.yml` dispatch with `layer: extension`, `release_tag: pi-v2.5.2`, `publish: true`. The lane builds the committed `dist` of that tag (check:pi-dist), packs exactly one tarball, attests it, preserves it as a GitHub Release asset, and publishes via the same protected `npm-production` OIDC environment — the npm trusted-publisher registration is shared with the family workflow file.
- Features that need newer native envelope fields must degrade gracefully on older launchers; when a feature genuinely requires a new native floor, raise `launcherRange` and `compatibility.layers.extension.minLauncherVersion` in the contract and ship a family release first.
- Local dry-run of the lane: `node packages/pi/scripts/release-acceptance.mjs pack --lane extension --output <empty-dir> --commit $(git rev-parse HEAD)` then `... verify --artifacts <dir>`. Gate locally: `... gate --lane extension --tag pi-v2.5.2 --commit <sha> --ref-type tag` against a real signed tag.

### Dogfood publish (one command)

For everyday iteration the extension may be published locally — this is the dogfooding lane the release contract blesses:

```bash
npm run publish:pi-extension            # publish the version in the contract
npm run publish:pi-extension -- 2.1.1   # bump manifest+contract, commit them, publish
```

The script enforces the same invariants the tag lane checks: manifest and `packages.extension.version` agree, `ast-sgrep` dependency equals `launcherRange`, the extension tree and dist are fully committed (packed content = committed content, i.e. off origin/main once pushed), `check:pi-contract` passes, and the build is clean. Only then does it run `npm publish`. Provenance attestation and OIDC remain exclusive to the signed `pi-v` tag lane — use it for releases you want attestable.

Local preparation is side-effect free:

```bash
npm run check:pi-contract
npm run check:pi-dist
npm run check:pi-release
```

`check:pi-contract` remains the release-metadata/version skew gate (including a few src↔dist constant checks). `check:pi-dist` rebuilds the committed `packages/pi/extension/dist` via `tsc` and fails if `git status --porcelain` is non-empty under that tree (tracked drift or untracked emit; `npm files` ships `dist`; do not un-commit it).

The release-gate and E2E commands exercise packed artifacts and the official Pi loader without publishing. Package-level `npm pack --dry-run`/`npm pack` preparation is allowed; do not run `npm publish` locally. The manual **Pi native artifacts** workflow (`.github/workflows/pi-native-artifacts.yml`) is dry-run only. The tag-only **Pi npm official release** workflow (`.github/workflows/pi-npm-release.yml`) is the canonical publisher. Both pin Rust `1.97.1`; the official matrix packs, clean-installs, and executes each native artifact on its matching host before upload.

External npm publication requires explicit human approval of its protected `npm-production` environment, the `NPM_OWNERSHIP_APPROVED=true` secret in that environment, and trusted-publishing OIDC/provenance. Before first publication, re-verify every npm name and publisher ownership; a prior 404 is not a reservation. Publish native packages before the launcher and the launcher before the extension. GitHub Release assets are immutable: reruns download and byte-compare existing assets and never clobber them.

If publication stops after a package becomes visible, retry the same preserved family only when npm's integrity for every live package exactly matches its local release tarball. The retry skips identical packages and continues in canonical order. Any mismatch requires a new version, repeated checks, and new approval.

## Native build profiles (npm vs DSR)

Deliberate split (br-kpt option b), recorded in the contract as `releaseAutomation.nativeBuildProfile`:

- The npm matrix builds with plain `cargo build --locked --release` (default codegen, no LTO, unstripped). npm binaries therefore differ from DSR binaries.
- DSR ships the `release-ship` profile (thin LTO, `codegen-units = 1`, stripped).

Unifying the npm matrix on `release-ship` stays open pending measurement: the win32 warm baseline is ~4 min of cargo, and the cold-cache cost is unknown until br-7ie measures it. Do not change profiles without that measurement. Revisit if cold builds show headroom or install weight becomes a blocker.

Recorded 2.5.0 sizes (unpacked MiB from `release-manifest.json`; packed decimal MB from the release assets):

| platform package | CLI | NAPI addon | packed tarball |
|---|---|---|---|
| `@ast-sgrep/darwin-arm64` | 37.8 | 35.6 | 14.5 |
| `@ast-sgrep/darwin-x64` | 38.1 | 35.8 | 14.6 |
| `@ast-sgrep/linux-arm64-gnu` | 39.1 | 36.7 | 14.9 |
| `@ast-sgrep/linux-x64-gnu` | 39.7 | 37.2 | 15.1 |
| `@ast-sgrep/win32-x64-msvc` | 36.8 | 34.7 | 13.9 |

## Version policy

- All public `ast-sgrep-*` crates use one lockstep version from `[workspace.package]`. A release must not mix versions.
- Versions follow Semantic Versioning. Incompatible public API changes require a major version bump and release notes.
- Additive, backward-compatible functionality increments the minor version after 1.0; backward-compatible fixes increment the patch version. Prerelease iterations increment the prerelease identifier (for example, `alpha.0` to `alpha.1`).
- Every path dependency between publishable workspace crates must also specify the same explicit version, so packaged manifests resolve from crates.io.

## Preparation

1. Confirm the working tree contains the intended release and the workspace version is consistent:

   ```sh
   cargo metadata --no-deps --format-version 1
   ```

2. Package each public crate in leaf order. This verifies the crate archive and compiles from the archive without publishing anything:

   ```sh
   CARGO_BUILD_JOBS=1 cargo package --locked -p ast-sgrep-lang
   CARGO_BUILD_JOBS=1 cargo package --locked -p ast-sgrep-embed
   CARGO_BUILD_JOBS=1 cargo package --locked -p ast-sgrep-mmap
   CARGO_BUILD_JOBS=1 cargo package --locked -p ast-sgrep-core \
     --config 'patch.crates-io.ast-sgrep-lang.path="crates/ast-sgrep-lang"' \
     --config 'patch.crates-io.ast-sgrep-embed.path="crates/ast-sgrep-embed"' \
     --config 'patch.crates-io.ast-sgrep-mmap.path="crates/ast-sgrep-mmap"'
   CARGO_BUILD_JOBS=1 cargo package --locked -p ast-sgrep-plugins \
     --config 'patch.crates-io.ast-sgrep-lang.path="crates/ast-sgrep-lang"' \
     --config 'patch.crates-io.ast-sgrep-embed.path="crates/ast-sgrep-embed"' \
     --config 'patch.crates-io.ast-sgrep-core.path="crates/ast-sgrep-core"'
   CARGO_BUILD_JOBS=1 cargo package --locked -p ast-sgrep-codemode \
     --config 'patch.crates-io.ast-sgrep-lang.path="crates/ast-sgrep-lang"' \
     --config 'patch.crates-io.ast-sgrep-embed.path="crates/ast-sgrep-embed"' \
     --config 'patch.crates-io.ast-sgrep-core.path="crates/ast-sgrep-core"' \
     --config 'patch.crates-io.ast-sgrep-plugins.path="crates/ast-sgrep-plugins"'
   CARGO_BUILD_JOBS=1 cargo package --locked -p ast-sgrep-lsp \
     --config 'patch.crates-io.ast-sgrep-lang.path="crates/ast-sgrep-lang"' \
     --config 'patch.crates-io.ast-sgrep-embed.path="crates/ast-sgrep-embed"' \
     --config 'patch.crates-io.ast-sgrep-core.path="crates/ast-sgrep-core"'
   CARGO_BUILD_JOBS=1 cargo package --locked -p ast-sgrep-cli \
     --config 'patch.crates-io.ast-sgrep-lang.path="crates/ast-sgrep-lang"' \
     --config 'patch.crates-io.ast-sgrep-embed.path="crates/ast-sgrep-embed"' \
     --config 'patch.crates-io.ast-sgrep-core.path="crates/ast-sgrep-core"' \
     --config 'patch.crates-io.ast-sgrep-plugins.path="crates/ast-sgrep-plugins"' \
     --config 'patch.crates-io.ast-sgrep-codemode.path="crates/ast-sgrep-codemode"'
   CARGO_BUILD_JOBS=1 cargo package --locked -p ast-sgrep-mcp \
     --config 'patch.crates-io.ast-sgrep-lang.path="crates/ast-sgrep-lang"' \
     --config 'patch.crates-io.ast-sgrep-embed.path="crates/ast-sgrep-embed"' \
     --config 'patch.crates-io.ast-sgrep-core.path="crates/ast-sgrep-core"' \
     --config 'patch.crates-io.ast-sgrep-plugins.path="crates/ast-sgrep-plugins"' 
   ```

   The temporary `patch.crates-io` overrides let dependent archives verify before their unpublished leaf crates exist in the crates.io index; they do not alter packaged manifests. Add `--allow-dirty` only during local preparation when reviewing intentional, uncommitted release changes. Do not use it for the approved release commit.

3. Inspect each archive with `cargo package --list -p <crate>`. Confirm the root README and license metadata are present, no credentials or generated data are included, and docs.rs links point to the matching crate.

## Publish after explicit approval

Only a human release operator may run this block. It is noninteractive and publishes the nine public crates in the only valid leaf order. It waits until each immutable version is resolvable from the crates.io index before publishing a dependent crate.

```bash
set -euo pipefail
release_version='2.5.2'
release_crates=(
  ast-sgrep-lang
  ast-sgrep-embed
  ast-sgrep-mmap
  ast-sgrep-core
  ast-sgrep-plugins
  ast-sgrep-codemode
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

1. Verify the exact immutable version exists for all nine crates and that docs.rs has completed each build:

   ```bash
   set -euo pipefail
   release_version='2.5.2'
   release_crates=(ast-sgrep-lang ast-sgrep-embed ast-sgrep-mmap ast-sgrep-core ast-sgrep-plugins ast-sgrep-codemode ast-sgrep-lsp ast-sgrep-cli ast-sgrep-mcp)
   for crate in "${release_crates[@]}"; do
     cargo info --registry crates-io "${crate}@${release_version}" >/dev/null
     curl --fail --location --silent --show-error --output /dev/null \
       "https://docs.rs/${crate}/${release_version}/"
   done
   ```

2. Install the exact version from crates.io into a new temporary root, independently of the checkout, and record both version outputs as clean-install evidence:

   ```bash
   set -euo pipefail
   release_version='2.5.2'
   install_root="$(mktemp -d)"
   cargo install ast-sgrep-cli --version "=${release_version}" --locked --root "$install_root"
   asgrep_version="$("$install_root/bin/asgrep" --version)"
   ast_sgrep_version="$("$install_root/bin/ast-sgrep" --version)"
   printf '%s\n%s\n' "$asgrep_version" "$ast_sgrep_version"
   [[ "$asgrep_version" == *" ${release_version}" ]]
   [[ "$ast_sgrep_version" == *" ${release_version}" ]]
   ```

3. Run the GitHub Actions `Post-publish install and docs smoke` workflow manually with `version` set to `2.5.2`. It installs the exact crates.io CLI version into an empty temporary root on Linux and macOS, checks both binaries, and verifies the exact-version docs.rs page for every published crate. Save the successful workflow URL with the release record. This workflow is post-publish evidence only; do not run it before the release is visible on crates.io.

## Homebrew formula

The standalone source formula lives at `packaging/homebrew/ast-sgrep.rb`. It remains pinned to the latest verified archive until the new GitHub tag is published and its digest is known. After publishing the tag, calculate the archive digest and update both the formula URL/version and checksum:

```sh
version="2.5.2"
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

## Honesty checklist (before tagging / README bumps)

Complete before any release tag, crates.io/npm publish, or README quality/latency GATE:

1. **No unreproducible README GATE.** README and release notes must not present MRR/latency/dimension figures as current product guarantees unless `benchmarks/results/baselines.md` marks them reproducible **or** the text explicitly says historical / `UNREPRODUCIBLE` and links the canonical row.
2. **Single fingerprint per metric.** Confirm no conflicting canonical values for the same corpus+config (see `baselines.md` “Canonical fingerprint rows”). Superseded numbers stay labeled superseded.
3. **Negative ledger reviewed.** Failed or withdrawn evals still appear in `benchmarks/results/` (or are linked from baselines); do not ship a release that drops known losses.
4. **Optional harness job URL.** If a CI workflow regenerated numbers, record the successful job URL next to the baselines provenance table; absence of a URL means the row stays unreproducible from this tree.
5. **No conformal cert from a point estimate.** Do not quote matrix present-count or a single MRR as certified. `tests/conformance/parity_score.json` must show `certified=false` (or a real lower bound) before any `strict-conformant-release.v1` language. Never emit `release_certificate.json` while H-rows are red. `UNREPRODUCIBLE` MRR is not a certificate.

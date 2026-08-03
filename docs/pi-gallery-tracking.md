# TRACKING: pi-ast-sgrep gallery listing (81pi)

**Status:** TRACKING — no further local code action required for gallery appearance.

## Already verified in-repo / publish correctness

- `pi-ast-sgrep` package carries `pi-package` keyword + Pi manifest + image (see `packages/pi/extension/package.json`).
- Release contract + acceptance scripts: `packages/pi/release-contract.json`, `packages/pi/scripts/release-acceptance.mjs`, `packages/pi/launcher/test/npm-native-packages.test.mjs`.
- Official release workflow packs, clean-installs, and exercises natives before upload.

## External lag

`pi-ast-sgrep@1.3.2` may be published correctly but absent from npm search / pi.dev gallery until npm indexes the package (often hours to ~48h after first publish).

## Verify later (manual)

```bash
curl -s "https://registry.npmjs.org/-/v1/search?text=pi-ast-sgrep&size=20" | jq '.objects[].package.name'
# Expect: pi-ast-sgrep once indexed
```

Then check https://pi.dev/packages (gallery is downstream of npm search).

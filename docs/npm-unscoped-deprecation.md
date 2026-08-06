# Deprecate orphaned unscoped `ast-sgrep-*` native packages (wldi)

Pre-scope orphaned native packages remain published unscoped and should be deprecated to steer users to the scoped `@ast-sgrep/*` family (installed automatically via the `ast-sgrep` launcher).

## Packages (versions ≤1.3.1)

- `ast-sgrep-darwin-arm64`
- `ast-sgrep-darwin-x64`
- `ast-sgrep-linux-arm64-gnu`
- `ast-sgrep-linux-x64-gnu`

(`win32-x64-msvc` was never published unscoped.)

## Blocker

`npm deprecate` is a write op requiring interactive web/2FA auth; cloud agents cannot run it.

## User action checklist (run locally where npm auth/OTP works)

```bash
for p in darwin-arm64 darwin-x64 linux-arm64-gnu linux-x64-gnu; do
  npm deprecate "ast-sgrep-$p@<=1.3.1" "Deprecated: install via ast-sgrep / @ast-sgrep/$p (scoped). Unscoped packages are orphaned."
done
```

## Verify

```bash
npm view ast-sgrep-darwin-arm64 deprecated
npm view @ast-sgrep/darwin-arm64 name
```

Expected: unscoped packages show a deprecation message; scoped packages remain current.

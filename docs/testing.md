# Testing guide (v1.0.0-alpha)

Run this checklist before cutting `v1.0.0-alpha.0` on GitHub.

## Quick validation

```bash
# Full workspace tests
cargo test --workspace

# CLI smoke (fixture repo)
cargo build -p ast-sgrep-cli
./target/debug/asgrep index tests/fixtures/sample
./target/debug/asgrep status tests/fixtures/sample
./target/debug/asgrep "credential renewal" tests/fixtures/sample
./target/debug/asgrep semantic "credential renewal" --json tests/fixtures/sample
./target/debug/asgrep --json --format agent "process_request" tests/fixtures/sample
```

## Test matrix

| Area | Command |
|------|---------|
| Core + semantic | `cargo test -p ast-sgrep-core` |
| Embed provider | `cargo test -p ast-sgrep-embed` |
| LSP | `cargo test -p ast-sgrep-lsp` |
| Plugins (agent JSON) | `cargo test -p ast-sgrep-plugins` |
| CLI e2e | `cargo test -p ast-sgrep-cli` |
| IVF persistence | `cargo test -p ast-sgrep-core --test ivf_persist` |
| Semantic synonyms | `cargo test -p ast-sgrep-core --test semantic` |

## Manual checks

- [ ] `asgrep index .` on a real repo (no API key) — semantic chunks > 0 in `status`
- [ ] `"credential renewal"` returns `auth_refresh` or equivalent synonym hit
- [ ] `asgrep --no-embed` disables embed hits
- [ ] `callers:` / `defs:` return correct symbols (no false callers)
- [ ] `asgrep-lsp` starts; `workspace/symbol` returns results after index ready
- [ ] Large repo (optional): `semantic.ivf` appears when symbol count ≥ `ASGREP_ANN_THRESHOLD`

## Benchmark

```bash
asgrep bench tests/fixtures/sample --iterations 100
# Target: avg search < 20ms on fixture
```

## Release (when ready)

```bash
git tag v1.0.0-alpha.0
git push origin v1.0.0-alpha.0
gh release create v1.0.0-alpha.0 --title "v1.0.0-alpha.0" --notes "First alpha: semantic search, agent JSON, IVF sidecar" --prerelease
```

No API key required for default offline semantic path.

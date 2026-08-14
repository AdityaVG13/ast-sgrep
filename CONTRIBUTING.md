# Contributing to ast-sgrep

## Prerequisites

- Rust stable (edition 2021)
- `cargo` on `PATH`

## Local verification (default bar)

Keep this cheap and single-process. Do **not** treat full workspace test matrices as required for every change.

From the repository root:

```bash
# Forbid-soundness (first-party unsafe ban; distinct from cargo audit)
bash scripts/verify-forbid-soundness

# Typecheck
cargo check --workspace -j1

# Focused parity suite (index + defs/hybrid/chain on the real APIs)
cargo test -p ast-sgrep-core --test parity -j1 -- --test-threads=1

# CLI smoke
cargo build --release -p ast-sgrep-cli -j1
./target/release/asgrep --help
```

New workspace members **must** set `[lints] workspace = true` so they inherit
`unsafe_code = "forbid"`. The only sealed exception is `ast-sgrep-mmap`
(see [SECURITY.md](SECURITY.md)).

Before a crates/workspace release, humans may run:

```bash
bash scripts/local-release-gate.sh
```

That gate checks formatting, workspace clippy and tests, then a bounded 30-second
fuzz run. It is **local prep**, not the Pi npm publisher. Official Pi publication
uses `packages/pi/scripts/release-acceptance.mjs` (see [docs/RELEASING.md](docs/RELEASING.md)).
Ordinary changes should keep using the cheaper, targeted default bar above.

Merge honesty (optional, does not replace T0): `bash scripts/run-proof-pack.sh`
writes `tests/artifacts/compliance/COMPLIANCE_REPORT.md`. See
[docs/validation/proof-pack.md](docs/validation/proof-pack.md).

GitHub Actions on every `pull_request` runs `forbid-soundness`, `cargo-check`,
ubuntu `test` (`cargo test --workspace`, compare-only goldens), `pi`, `clippy`,
`fmt`, and `audit`. The ubuntu+macos **release** matrix (`build-and-test`),
Windows smoke, and bounded fuzz stay `workflow_dispatch` (Actions tab). Speed
and bake-off workflows execute real harnesses and fail on correctness, identity,
or latency threshold breaches.

## Golden files

CI compares frozen dumps; it never rewrites them (`ASGREP_UPDATE_GOLDENS=0`).
To refresh a freeze locally, set `ASGREP_UPDATE_GOLDENS=1`, run the targeted
test, review `git diff` file-by-file, and commit. Never commit `*.actual`.
Full SOP: [docs/validation/golden-files.md](docs/validation/golden-files.md).
Do not treat `benchmarks/results/baselines.md` as a golden.

## Pull requests

- Keep changes focused; extend `tests/core/parity.rs` (or a targeted unit test) when behavior changes.
- Review golden/fixture diffs file-by-file; do not commit `*.actual`.
- Do not commit local agent/tool caches or skill-run trees -- they are gitignored.
- Do not commit secrets, `.env`, local caches, or `fuzz/target/`.
- Prefer conventional commits: `feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `ci:`, `chore:`.
- Metric claims must cite `benchmarks/results/baselines.md` or be tagged `UNREPRODUCIBLE`.

## Crate layout

| Crate | Role |
|-------|------|
| `ast-sgrep-core` | Index, search, SQLite store, chain, semantic ANN |
| `ast-sgrep-cli` | `asgrep` / `ast-sgrep` binaries, supervisor |
| `ast-sgrep-lang` | Tree-sitter extraction |
| `ast-sgrep-embed` | Embeddings (+ optional neural/rerank features) |
| `ast-sgrep-lsp` | Language server |
| `ast-sgrep-mcp` | MCP server for agents |
| `ast-sgrep-plugins` | Output formats (native/github/gitlab/agent/capsule) |
| `ast-sgrep-testkit` | Shared fixtures for integration tests |

See [README.md](README.md) and [docs/README.md](docs/README.md) for user-facing docs.

Conformance honesty: [docs/validation/DISCREPANCIES.md](docs/validation/DISCREPANCIES.md),
[docs/validation/COVERAGE.md](docs/validation/COVERAGE.md), and
[docs/validation/conformance-verdicts.md](docs/validation/conformance-verdicts.md).
XFAIL/`#[ignore]` only with a registered DISC id. Not-run is not Pass.

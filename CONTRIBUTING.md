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

# CLI smoke (search + auto-index)
cargo test -p ast-sgrep-cli --test cli_smoke -j1 -- --test-threads=1
cargo build --release -p ast-sgrep-cli -j1
./target/release/asgrep --help

# Pi search/index behavior when touching packages/pi
npm test --workspace pi-ast-sgrep
```

New workspace members **must** set `[lints] workspace = true` so they inherit
`unsafe_code = "forbid"`. Sealed exceptions are exactly two (see
[SECURITY.md](SECURITY.md)): `ast-sgrep-mmap` (sole hand-written `unsafe`) and
`ast-sgrep-codemode-napi` (generated Node-API FFI only).

Release cuts use the same default bar, plus the targeted suites that cover the
changed surface. Do not treat a full `cargo test --workspace` as required for
ordinary work.

GitHub Actions is manual-only (`workflow_dispatch`). PR and branch pushes do
not start workflows. Dispatch **CI** from the Actions tab when you want the
GitHub matrix; use the local bar above for ordinary work.

## Golden files

CI compares frozen dumps; it never rewrites them (`ASGREP_UPDATE_GOLDENS=0`).
To refresh a freeze locally, set `ASGREP_UPDATE_GOLDENS=1`, run the targeted
test, review `git diff` file-by-file, and commit. Never commit `*.actual`.
Full SOP: [docs/validation/golden-files.md](docs/validation/golden-files.md).
Do not treat `benchmarks/results/baselines.md` as a golden.

## Pull requests

- Keep changes focused; extend an intent suite under `tests/` via `ast-sgrep-testkit` when search, index, or Pi behavior changes.
- Review golden/fixture diffs file-by-file; do not commit `*.actual`.
- Do not commit local agent/tool caches or skill-run trees -- they are gitignored.
- Do not commit secrets, `.env`, local caches.
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
| `ast-sgrep-mmap` | Sealed read-only mmap wrapper (sole hand-written unsafe boundary) |
| `ast-sgrep-codemode` | Code Mode / programmatic tool-calling |
| `ast-sgrep-codemode-napi` | Node-API bindings for in-process Code Mode |
| `ast-sgrep-plugins` | Output formats (native/github/gitlab/agent/capsule) |
| `ast-sgrep-testkit` | Shared fixtures for search, index, and Pi tests |

See [README.md](README.md) and [docs/README.md](docs/README.md) for user-facing docs.

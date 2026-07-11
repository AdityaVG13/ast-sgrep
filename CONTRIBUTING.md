# Contributing to ast-sgrep

## Prerequisites

- Rust stable (edition 2021)
- `cargo` on `PATH`

## Local verification (default bar)

Keep this cheap and single-process. Do **not** treat full workspace test matrices as required for every change.

From the repository root:

```bash
# Typecheck
cargo check --workspace -j1

# Focused parity suite (index + defs/hybrid/chain on the real APIs)
cargo test -p ast-sgrep-core --test parity -j1 -- --test-threads=1

# CLI smoke
cargo build --release -p ast-sgrep-cli -j1
./target/release/asgrep --help
```

Optional, when you intentionally want broader coverage:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace -j1 -- --test-threads=1
```

GitHub Actions (`CI`, bake-off, speed, install-smoke) are **manual-only** (`workflow_dispatch`). Trigger from the Actions tab; they do not run on every push.

## Pull requests

- Keep changes focused; extend `crates/ast-sgrep-core/tests/parity.rs` (or a targeted unit test) when behavior changes.
- Do not commit local agent/tool caches or skill-run trees -- they are gitignored.
- Do not commit secrets, `.env`, local caches, or `fuzz/target/`.
- Prefer conventional commits: `feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `ci:`, `chore:`.

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

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

Before a release, run the same gate used by official release acceptance:

```bash
bash scripts/local-release-gate.sh
```

The release gate checks formatting, workspace clippy and tests, then exercises
ranking invariants with a bounded 30-second fuzz run. It requires stable Rust,
nightly Rust, and `cargo-fuzz`. Ordinary changes should keep using the cheaper,
targeted default bar above.

GitHub Actions runs `forbid-soundness` and `cargo check` on every `pull_request`.
Full build/test/clippy/audit/fuzz matrices remain `workflow_dispatch` (Actions tab).
The speed and bake-off workflows execute real harnesses and fail on correctness,
identity, or latency threshold breaches. The official package release invokes
`scripts/local-release-gate.sh` through the release-acceptance command.

## Pull requests

- Keep changes focused; extend `tests/core/parity.rs` (or a targeted unit test) when behavior changes.
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

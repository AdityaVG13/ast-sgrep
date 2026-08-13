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
`unsafe_code = "forbid"`. Sealed exceptions are exactly two (see
[SECURITY.md](SECURITY.md)): `ast-sgrep-mmap` (sole hand-written `unsafe`) and
`ast-sgrep-codemode-napi` (generated Node-API FFI only).

Before a Rust release cut, run the local release gate manually:

```bash
bash scripts/local-release-gate.sh
```

That gate checks formatting, workspace clippy and tests, then exercises ranking
invariants with a bounded 30-second fuzz run. It requires stable Rust, nightly
Rust, and `cargo-fuzz`. It is **not** invoked by Pi `release-acceptance` (npm
pack/verify/gate/publish). Ordinary changes should keep using the cheaper,
targeted default bar above.

GitHub Actions on every `pull_request` runs `forbid-soundness`, `cargo-check`,
`test`, `pi`, `clippy`, `fmt`, and `audit`. `build-and-test`, `windows-smoke`,
and `bounded-fuzz` remain `workflow_dispatch` (Actions tab). The speed and
bake-off workflows execute real harnesses and fail on correctness, identity, or
latency threshold breaches.

## Pull requests

- Keep changes focused; extend `tests/core/parity.rs` (or a targeted unit test) when behavior changes.
- Do not commit skill-run trees (`.code-upgrade-enterprise/`, `.rotational-code-analysis/`, and similar). Those stay gitignored.
- Do commit `.skill-loop-progress.md` on the branch that ran the loop. It is the resume source of truth, not a cache.
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
| `ast-sgrep-mmap` | Sealed read-only mmap wrapper (sole hand-written unsafe boundary) |
| `ast-sgrep-codemode` | Code Mode / programmatic tool-calling |
| `ast-sgrep-codemode-napi` | Node-API bindings for in-process Code Mode |
| `ast-sgrep-plugins` | Output formats (native/github/gitlab/agent/capsule) |
| `ast-sgrep-testkit` | Shared fixtures for integration tests |

See [README.md](README.md) and [docs/README.md](docs/README.md) for user-facing docs.

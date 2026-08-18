# Contributing to ast-sgrep

## Prerequisites

- Rust stable (edition 2021)
- `cargo` on `PATH`

## Local verification

Prove the production surface by compiling it. Do not add a behavioral test suite.

From the repository root:

```bash
bash scripts/verify-forbid-soundness
cargo check --workspace --lib --bins -j1
cargo build --release -p ast-sgrep-cli -j1
./target/release/asgrep --help
```

New workspace members **must** set `[lints] workspace = true` so they inherit
`unsafe_code = "forbid"`. Sealed exceptions are exactly two (see
[SECURITY.md](SECURITY.md)): `ast-sgrep-mmap` (sole hand-written `unsafe`) and
`ast-sgrep-codemode-napi` (generated Node-API FFI only).

GitHub Actions on `pull_request` runs `forbid-soundness`, `cargo-check`, `pi`
package compile gates, `clippy`, `fmt`, and `audit`. Release-host builds stay
`workflow_dispatch`.

## Pull requests

- Keep changes focused on the shipped crates, CLI, Pi package, or MCP/LSP.
- Do not commit local agent/tool caches or skill-run trees -- they are gitignored.
- Do not commit secrets, `.env`, or local caches.
- Prefer conventional commits: `feat:`, `fix:`, `refactor:`, `docs:`, `ci:`, `chore:`.
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

See [README.md](README.md) and [docs/README.md](docs/README.md) for user-facing docs.

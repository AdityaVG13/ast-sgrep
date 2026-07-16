# Documentation index

Canonical entry points for humans and agents. Prefer this list over scavenging the repo root.

## Start here

| Doc | Audience | Contents |
|-----|----------|----------|
| [../README.md](../README.md) | Everyone | Product overview, install, quick start |
| [getting-started.md](getting-started.md) | Users | Pi-first and standalone install, index, queries, flags, troubleshooting |
| [pi-package.md](pi-package.md) | Pi users/operators | Canonical install/use/update/debug/remove guide; data, security, privacy, compatibility, and provenance |
| [comparison.md](comparison.md) | Users | When to use ast-sgrep vs ripgrep vs ast-grep |

## Product depth

| Doc | Contents |
|-----|----------|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Crates, index schema, search pipeline, fusion, agent surfaces |
| [how-it-works.md](how-it-works.md) | Pipeline narrative and incremental indexing |
| [QUERY_GRAMMAR.md](QUERY_GRAMMAR.md) | Prefixes, composition, routing contract |
| [semantic-search.md](semantic-search.md) | Symbol chunks, provider chain, IVF-ANN, tuning |
| [mcp.md](mcp.md) | `asgrep-mcp` setup for agents |
| [use-cases.md](use-cases.md) | Agents, LSP, JSON formats, CI patterns |

## Quality and operations

| Doc | Contents |
|-----|----------|
| [benchmarks.md](benchmarks.md) | Methodology, reproduction, quality numbers, known losses |
| [PERF_INVENTORY.md](PERF_INVENTORY.md) | Performance surface inventory |
| [RELEASING.md](RELEASING.md) | Release checklist |
| [../CONTRIBUTING.md](../CONTRIBUTING.md) | Local verification bar and PR hygiene |
| [../benchmarks/README.md](../benchmarks/README.md) | Benchmark docs index |
| [../benchmarks/results/head-to-head.md](../benchmarks/results/head-to-head.md) | Canonical cross-tool table |
| [../benchmarks/results/losses.md](../benchmarks/results/losses.md) | Published regressions |
| [../benchmarks/results/speed.md](../benchmarks/results/speed.md) | Lexical/structural speed notes |
| [../benchmarks/results/bakeoff.md](../benchmarks/results/bakeoff.md) | Offline bake-off narrative |

## Crate map

```text
ast-sgrep-lang   → extract symbols / calls / imports
ast-sgrep-core   → index + hybrid search + chain
ast-sgrep-embed  → embedding providers (+ optional neural/rerank)
ast-sgrep-cli    → asgrep / ast-sgrep binaries
ast-sgrep-lsp    → language server
ast-sgrep-mcp    → MCP stdio server
ast-sgrep-plugins→ JSON/output formats
ast-sgrep-testkit→ shared fixtures for tests
```

## CI note

Workflows under `.github/workflows/` are **`workflow_dispatch` only** (manual). They do not run on every push/PR. Trigger from the GitHub Actions tab when needed.

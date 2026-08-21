# Documentation index

Canonical entry points for humans and agents. Prefer this list over scavenging the repo root.

## Start here

| Doc | Audience | Contents |
|-----|----------|----------|
| [../README.md](../README.md) | Everyone | Product overview, install, quick start |
| [getting-started.md](getting-started.md) | Users | Pi-first and standalone install, index, queries, flags, troubleshooting |
| [pi-package.md](pi-package.md) | Pi users/operators | Install, use, update, debug, remove; data, security, privacy |
| [comparison.md](comparison.md) | Users | When to use ast-sgrep vs ripgrep vs ast-grep |

## Product

| Doc | Contents |
|-----|----------|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Crates, index schema, search pipeline, fusion, agent surfaces |
| [how-it-works.md](how-it-works.md) | Pipeline narrative and incremental indexing |
| [QUERY_GRAMMAR.md](QUERY_GRAMMAR.md) | Mode prefixes plus two-channel `AND` / `AND NOT` |
| [semantic-search.md](semantic-search.md) | Symbol chunks, provider chain, IVF-ANN, tuning |
| [fusion-ranking.md](fusion-ranking.md) | Weighted RRF, post-fusion critic, agent `why` |
| [cascade-query-planner.md](cascade-query-planner.md) | Retrieval cascade and causal follow-ups |
| [mcp.md](mcp.md) | `asgrep-mcp` setup for agents |
| [codemode.md](codemode.md) | Code Mode: JS program orchestration (Pi primary); XOR with MCP |
| [use-cases.md](use-cases.md) | Agents, LSP, JSON formats, CI patterns |
| [structural-patterns.md](structural-patterns.md) | Pattern syntax and language coverage |
| [symbol-normalization.md](symbol-normalization.md) | Identifier folding used by defs/callers |
| [index-consistency.md](index-consistency.md) | When the index is considered current |
| [signal-provenance.md](signal-provenance.md) | How a hit explains itself |
| [env-trust.md](env-trust.md) | Environment and binary-path trust |
| [panic-poison.md](panic-poison.md) | Mutex poison and fail-closed recovery |

## Contributor

| Doc | Contents |
|-----|----------|
| [../CONTRIBUTING.md](../CONTRIBUTING.md) | Local verification bar and PR hygiene |
| [RELEASING.md](RELEASING.md) | Release checklist |
| [validation/negative-ledgers.md](validation/negative-ledgers.md) | Product fail-closed cases (must error, not empty hits) |
| [validation/machine-json-schema.md](validation/machine-json-schema.md) | Agent JSON envelope |
| [validation/compact-output.md](validation/compact-output.md) | Compact CLI output |
| [validation/neural-trust.md](validation/neural-trust.md) | Optional in-process neural embeddings |
| [validation/semantic-ivf-mmap.md](validation/semantic-ivf-mmap.md) | IVF sidecar layout |
| [validation/golden-files.md](validation/golden-files.md) | Compare-only goldens; how to refresh locally |

Published result tables live under [`../benchmarks/results/`](../benchmarks/results/); start from [`../benchmarks/README.md`](../benchmarks/README.md).

## Crate map

```text
ast-sgrep-lang   → extract symbols / calls / imports
ast-sgrep-core   → index + hybrid search + critic + planner
ast-sgrep-embed  → in-process embedding providers (+ optional neural/rerank)
ast-sgrep-mmap   → memory-map helpers
ast-sgrep-cli    → asgrep / ast-sgrep binaries
ast-sgrep-lsp    → language server
ast-sgrep-mcp    → MCP stdio server
ast-sgrep-codemode → Code Mode / PTC tools + plan runner
ast-sgrep-plugins→ JSON/output formats
ast-sgrep-testkit→ shared fixtures for search/index/Pi tests
```

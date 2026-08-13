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
| [QUERY_GRAMMAR.md](QUERY_GRAMMAR.md) | Mode prefixes and routing (no composable AND) |
| [semantic-search.md](semantic-search.md) | Symbol chunks, provider chain, IVF-ANN, tuning |
| [mcp.md](mcp.md) | `asgrep-mcp` setup for agents |
| [codemode.md](codemode.md) | Code Mode: JS program orchestration (Pi primary); XOR with MCP — never both |
| [use-cases.md](use-cases.md) | Agents, LSP, JSON formats, CI patterns |

## Quality and operations

| Doc | Contents |
|-----|----------|
| [benchmarks.md](benchmarks.md) | Methodology reading order + local smoke |
| [PERF_INVENTORY.md](PERF_INVENTORY.md) | Hot-path cost notes + measurement caveats |
| [RELEASING.md](RELEASING.md) | Release checklist |
| [../CONTRIBUTING.md](../CONTRIBUTING.md) | Local verification bar and PR hygiene |
| [validation/DISCREPANCIES.md](validation/DISCREPANCIES.md) | Registered intentional divergences (XFAIL ids) |
| [validation/COVERAGE.md](validation/COVERAGE.md) | Conformance surface skeleton |
| [validation/conformance-verdicts.md](validation/conformance-verdicts.md) | Fail / Ignore / XFAIL / Not-run |
| [validation/proof-pack.md](validation/proof-pack.md) | Minimal reproducible ranking/honesty gates |
| [progress/README.md](progress/README.md) | Campaign negative ledgers (perf / conformance / surface) |
| [../benchmarks/README.md](../benchmarks/README.md) | Benchmark folder index and error budgets |

Published result tables (`head-to-head`, `speed`, `bakeoff`, `losses`, `baselines`)
live under [`../benchmarks/results/`](../benchmarks/results/); start from the
folder README rather than duplicating that index here.

## Crate map

```text
ast-sgrep-lang   → extract symbols / calls / imports
ast-sgrep-core   → index + hybrid search + chain
ast-sgrep-embed  → embedding providers (+ optional neural/rerank)
ast-sgrep-cli    → asgrep / ast-sgrep binaries
ast-sgrep-lsp    → language server
ast-sgrep-mcp    → MCP stdio server
ast-sgrep-codemode → Code Mode / PTC tools + plan runner
ast-sgrep-plugins→ JSON/output formats
ast-sgrep-testkit→ shared fixtures for tests
```

## CI note

Workflows under `.github/workflows/` are **`workflow_dispatch` only** (manual). They do not run on every push/PR. Trigger from the GitHub Actions tab when needed.

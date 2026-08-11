# Shard plan — stable module IDs for later rotations

Freeze: `fb932aac852f5496c0a7035cc5a0b508e05111cb`  
In-scope files: **523** (tracked after exclusion ledger).

## Stable module IDs

| ID | Path root | Files | Primary langs | Later-loop focus |
|---|---|---:|---|---|
| M-core | `crates/ast-sgrep-core/` | 78 | Rust | index/store/rank/semantic; 39 integration tests |
| M-lang | `crates/ast-sgrep-lang/` | 23 | Rust | tree-sitter extraction tables; goldens |
| M-cli | `crates/ast-sgrep-cli/` | 23 | Rust | bins asgrep/ast-sgrep; CLI contracts |
| M-embed | `crates/ast-sgrep-embed/` | 8 | Rust | embeddings; optional miri |
| M-lsp | `crates/ast-sgrep-lsp/` | 10 | Rust | asgrep-lsp protocol |
| M-mcp | `crates/ast-sgrep-mcp/` | 4 | Rust | MCP stdio protocol |
| M-codemode | `crates/ast-sgrep-codemode/` | 15 | Rust | session/catalog/batch |
| M-codemode-napi | `crates/ast-sgrep-codemode-napi/` | 5 | Rust | N-API cdylib (unsafe allow) |
| M-mmap | `crates/ast-sgrep-mmap/` | 2 | Rust | mmap (unsafe allow) |
| M-plugins | `crates/ast-sgrep-plugins/` | 5 | Rust | capsules/budget render |
| M-testkit | `crates/ast-sgrep-testkit/` | 9 | Rust | shared test helpers |
| M-pi-extension | `packages/pi/extension/` (non-dist) | 31 | TS/JS | Pi Code Mode surface |
| M-pi-launcher | `packages/pi/launcher/` | 12 | JS | npm bin wrapper |
| M-pi-platforms | `packages/pi/platforms/` (meta) | 11 | JSON/JS | platform package meta |
| M-pi-meta | `packages/pi/` scripts/contracts | 11 | JS/JSON | release gates |
| M-agent-plugin | `packages/agent-plugin/` | 8 | JSON/MD | Agent Plugins MCP pack |
| M-tests | `tests/` | 103 | mixed | fixtures, artifacts, ranking cases |
| M-fuzz | `fuzz/` | 66 | Rust/seeds | cargo-fuzz program |
| M-benchmarks | `benchmarks/` | 17 | MD/py/rs | baselines honesty |
| M-docs | `docs/` | 40 | Markdown | behavior contracts |
| M-scripts | `scripts/` | 9 | Shell/Python | forbid-soundness, gates |
| M-ci | `.github/` | 8 | YAML | workflow_dispatch CI |
| M-editors | `editors/` | 10 | mixed | VS Code pack |
| M-root | root manifests/policy | 13 | TOML/JSON/MD | workspace pins |
| M-cargo-config | `.cargo/` | 1 | TOML | cargo config |
| M-packaging | `packaging/` | 1 | — | packaging notes |

## Suggested rotation shards (token-sized)

Group for parallel attack / architecture loops without path thrash:

1. **SHARD-CORE-SEARCH** — M-core + M-lang + M-embed + M-testkit  
2. **SHARD-SURFACES** — M-cli + M-lsp + M-mcp + M-plugins  
3. **SHARD-CODEMODE** — M-codemode + M-codemode-napi + M-pi-extension + M-pi-launcher  
4. **SHARD-RELEASE-PI** — M-pi-meta + M-pi-platforms + M-agent-plugin + M-ci  
5. **SHARD-V&V** — M-tests + M-fuzz + M-benchmarks + M-scripts  
6. **SHARD-DOCS-POLICY** — M-docs + M-root + M-editors + M-packaging + M-cargo-config  
7. **SHARD-UNSAFE-ISLANDS** — M-mmap + M-codemode-napi (soundness exceptions)

## Exclusion modules (not attack shards)

| ID | Meaning |
|---|---|
| X-pi-dist | generated dist |
| X-beads | tracker runtime |
| X-native-assets | platform binaries/checksums |
| X-skill-loop | skill progress noise |
| X-cargo-target | `target/` + `target-pass*` |
| X-node-modules | npm vendor |

## Mapping rule for loop 3+

- Prefer **module ID** in findings (`module: M-core`) over raw paths.
- When a finding spans boundaries, tag **both** module IDs + boundary id (`BND-NAPI`, …).
- Do not re-open excluded modules unless evidence shows product behavior lives only there (e.g. dist-only bug → still fix sources).

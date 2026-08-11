# Pass 3 — Component graph summary

- **Freeze revision (retained):** `fb932aac852f5496c0a7035cc5a0b508e05111cb`
- **Mapped at:** 2026-08-11T01:52:59Z
- **Axes:** scale system→component; representation graph; observer architect; evidence source+build
- **Method:** `cargo metadata --format-version 1 --locked` + root/`packages/*/package.json` + pass2 module IDs
- **Note:** zerostack fszero engines unavailable; native cargo/git/rg used (B-ZS-ENGINES)

## System picture

```
                    ┌─────────────────────────────────────────┐
                    │  npm: pi-ast-sgrep (extension)          │
                    │   ├─ dep: ast-sgrep (launcher)          │
                    │   ├─ BND-NAPI → codemode-napi cdylib    │
                    │   └─ BND-CLI-JS → spawn asgrep bin      │
                    └───────────────┬─────────────────────────┘
                                    │
  agent-plugin ──BND-MCP──► asgrep-mcp ──┐
  editors/vscode ─BND-LSP─► asgrep-lsp ──┤
  asgrep/ast-sgrep CLI bins ─────────────┤
  codemode-napi ──► codemode ────────────┤
                                         ▼
                              ┌──────────────────┐
                              │  ast-sgrep-core  │  ◄── fan-in hub
                              │  search/store/   │
                              │  index/rank/ivf  │
                              └────────┬─────────┘
                    ┌──────────────────┼──────────────────┐
                    ▼                  ▼                  ▼
              ast-sgrep-lang    ast-sgrep-embed    ast-sgrep-mmap
              (tree-sitter)     (embed/rerank)     (unsafe island)
                    │
              BND-TREE-SITTER
```

## Cargo workspace (11 members, version 1.4.0)

| Crate | Layer | Module | Normal in | Normal out | Targets |
|-------|-------|--------|-----------|------------|---------|
| ast-sgrep-core | core hub | M-core | 6 | 3 (lang,embed,mmap) | lib |
| ast-sgrep-lang | core | M-lang | 2 | 0 | lib |
| ast-sgrep-embed | core | M-embed | 1 | 0 | lib |
| ast-sgrep-mmap | unsafe | M-mmap | 1 | 0 | lib |
| ast-sgrep-plugins | product-lib | M-plugins | 3 | 1 (core) | lib |
| ast-sgrep-codemode | product-lib | M-codemode | 2 | 2 (core,plugins) | lib |
| ast-sgrep-cli | surface | M-cli | 0 | 3 (core,plugins,codemode) | lib+bins |
| ast-sgrep-lsp | surface | M-lsp | 0* | 1 (core) | lib+bin |
| ast-sgrep-mcp | surface | M-mcp | 0 | 2 (core,plugins) | lib+bin |
| ast-sgrep-codemode-napi | unsafe | M-codemode-napi | 0 | 1 (codemode) | cdylib |
| ast-sgrep-testkit | test | M-testkit | 0† | 3 (core,lang,lsp) | lib |

\* testkit depends on lsp (normal) so lsp has fan-in=1 from testkit only  
† product crates depend on testkit as **dev** only

`default-members = [ast-sgrep-cli]`; workspace `exclude = [fuzz]`.

## Normal cargo edges (DAG)

```
cli → core, plugins, codemode
codemode → core, plugins
codemode-napi → codemode
mcp → core, plugins
lsp → core
plugins → core
core → lang, embed, mmap
testkit → core, lang, lsp   (test graph only for consumers)
```

## Pi / npm surface

| Package | Role | Links |
|---------|------|-------|
| `ast-sgrep-workspace` | workspaces: extension + launcher | root package.json |
| `pi-ast-sgrep` | Pi extension; Code Mode primary | dep `ast-sgrep`; build:native → napi |
| `ast-sgrep` (launcher) | bin `asgrep`/`ast-sgrep` | optionalDeps `@ast-sgrep/<plat>` |
| `@ast-sgrep/{darwin,linux,win32}-…` | platform binaries | M-pi-platforms |
| `@ast-sgrep/agent-plugin` | MCP skill pack | mcp.json → `asgrep-mcp` |

Extension runtime chooses **NAPI session** or **CLI spawn** (policy: Code Mode XOR MCP for agent hosts).

## Feature flags (propagate CLI → core → embed)

| Feature | Default | Path |
|---------|---------|------|
| cloud-embed | yes | cli → core → embed/cloud |
| neural-embed | no | cli → core → embed/neural-embed |
| rerank | no | cli → core → embed/rerank |

## Ownership (architect view)

| Owner surface | Components |
|---------------|------------|
| Index/search product | core, lang, embed, mmap |
| Agent wire/format | plugins, mcp, codemode |
| Human CLI | cli (+ watch/supervisor) |
| Editor | lsp, editors/vscode |
| Pi in-process | codemode-napi, pi-ast-sgrep |
| npm distribution | launcher, platforms, pi scripts/CI |
| V&V helpers | testkit, tests, fuzz (excluded member), benchmarks |

Machine-readable: [`component-graph.json`](./component-graph.json), [`edges.json`](./edges.json).

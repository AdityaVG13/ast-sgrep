# Pass 3 RESULT — Architecture dependency map

| Field | Value |
|-------|-------|
| Loop | 3 / architecture-dependency-and-ownership-map |
| Status | **COMPLETE** |
| Mode | audit (no product edits) |
| Freeze | `fb932aac852f5496c0a7035cc5a0b508e05111cb` (retained; HEAD may be ahead) |
| Axes | system→component · graph · architect · source+build |
| Braid | **Continue** → pass 4 entry-points / trust / privilege |

## Deliverables

| Artifact | Path |
|----------|------|
| Component graph (md) | `iterations/03-architecture/component-graph.md` |
| Component graph (json) | `iterations/03-architecture/component-graph.json` |
| Edges | `iterations/03-architecture/edges.json` |
| Hotspots | `iterations/03-architecture/hotspots.md` + `.json` |
| Cycles | `iterations/03-architecture/cycles.md` |
| Boundaries | `iterations/03-architecture/boundaries.md` |
| Slim mirror | `tests/artifacts/rotational-code-analysis/pass3-architecture/` |

## Headline findings (observations, not product bugs filed)

1. **DAG product graph** centered on `ast-sgrep-core` (fan-in 6).
2. **Dev-only cycles** via `ast-sgrep-testkit` ⇄ product crates.
3. **Six cross-boundary IDs:** BND-NAPI, BND-CLI-JS, BND-MCP, BND-LSP, BND-TREE-SITTER, BND-MMAP.
4. **Two unsafe islands:** mmap (semantic_ivf), codemode-napi (Node FFI); SECURITY.md text understates NAPI.
5. **Pi surface:** extension → launcher → platform bins; parallel NAPI load path.

## Evidence commands

```
cargo metadata --format-version 1 --locked
# workspace members + internal dep graph + cycle scan (python)
rg 'unsafe_code|memmap|napi' crates/*/Cargo.toml crates/ast-sgrep-mmap crates/ast-sgrep-codemode-napi
# package.json workspaces + extension/launcher/platforms
```

## Residuals → pass 4

- Enumerate **entry points** (bins, napi exports, MCP tools, LSP methods, Pi tools) with trust boundaries.
- Privilege / path sandbox notes for index root vs query input.
- Process model: single Searcher warm path (MCP/codemode) vs cold CLI.
- Still open: B-DIRTY-FREEZE, B-ZS-ENGINES, coverage/mutation gates.

## Counts

- Cargo members: 11
- Normal internal edges: 16
- Dev internal edges: 5
- Cross-ecosystem edges recorded: 7
- Boundaries: 6
- Cycles (normal): 0
- Cycles (with dev): 4

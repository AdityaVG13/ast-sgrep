# code-upgrade-enterprise whole-repo inventory

Campaign: dedicated-session · books `20260813-043427` · branch `refactor/de-monolithize-isomorphic` (PR 29) · PIN `4291982`.

Artifact: `.code-upgrade-enterprise/runs/20260813-043427/01-inventory.json` (gitignored).

## Scope checked
- All 11 workspace crates + fuzz (excluded) + packages/pi + packages/agent-plugin
- tests/{unit,integration dirs,fixtures,goldens,pi}
- 7 GitHub workflows + scripts/
- docs/ (+ validation demonolith leave-alone / surface-parity)
- editors/vscode, packaging/homebrew, benchmarks/

## Counts
| Area | Value |
| --- | --- |
| crates (workspace) | 11 |
| crate .rs LOC | 28077 |
| walk file_count | 542 |
| git tracked | 550 |
| tests/ tree files (sum dirs) | ~214 |
| #[ignore] found | 3 (baseline inherited ignored=4; reconcile) |
| real unsafe blocks | 1 (mmap) |
| unwrap / expect (all) | 1156 / 493 |
| product src unwrap / expect | 3 / 15 |
| todo!/unimplemented! | 0 |
| catch-all `_ =>` | 51 |
| TODO/FIXME/XXX/HACK | 2 |
| workflows | 7 |
| pi extension dist (B10) | 24 files, 204K, tracked |

## Top risks (Score next)
1. Clippy `-D warnings` historically red — re-measure before lint rehab claims
2. Committed `packages/pi/extension/dist` generated drift (B10)
3. Leave-alone hubs still >1000 LOC: `index.rs`, `mcp/lib.rs`
4. Ignored-test count drift vs inherited baseline; suite not re-run this pass
5. Docs: handbook mention without handbook dir; CONTRIBUTING crate table incomplete vs workspace

## Already healthy corners
- `unsafe_code = "forbid"` + SECURITY.md sealed exceptions (mmap, napi)
- No `todo!`/`unimplemented!`; no TS `as any`
- MCP/LSP/lang product src free of unwrap/expect
- PR CI always runs forbid-soundness + `cargo check`
- Demonolith leave-alone documentation present for residual hubs
- Pi release-contract npm scripts present at repo root

## Out of scope this pass
No product rehab. No Score selection / implementation. Next: `02-risk-map.md` + scored `03-candidate-matrix.md`; re-measure primary gate before treating 488/0/4 as current.

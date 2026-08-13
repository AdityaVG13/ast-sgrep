# code-upgrade-enterprise selected upgrade set

Campaign: dedicated-session · books `20260813-043427` · PIN `4291982` · inventory `6c4b180` · risk `9d6ab8e` · scores `1270057` · branch `refactor/de-monolithize-isomorphic` (PR 29).

Artifact (gitignored): `.code-upgrade-enterprise/runs/20260813-043427/04-selected-upgrade-set.md` (+ `05-obligation-ledger.md`, `work-queue/`).

**Select only — no product code this pass.** Broad floor: ≥1 workstream in each of code / verification / product.

## Ordered set (safest first)

| order | WS | candidate id(s) | tag | score | next verify |
| ---: | --- | --- | --- | ---: | --- |
| 1 | WS-01 | C-CLIPPY-DWARN | verification | 36.0 | `rch exec -- cargo clippy --workspace --all-targets -- -D warnings` |
| 2 | WS-02 | C-CONTRIBUTING · C-HANDBOOK · C-DOCS-MCP | product | 45.0 / 22.5 / 10.5 | table↔11 members; no ghost handbook; mcp example path truth |
| 3 | WS-03 | C-IGNORE-DRIFT | verification | 21.0 | `rch exec -- cargo test --workspace --no-fail-fast` + ignored count |
| 4 | WS-04 | C-SQL-UNWRAP | code | 13.3 | core parity + codemode `$ref` |
| 5 | WS-05 | C-B10-DIST | product | 16.0 | `npm run check:pi-contract` skew red / rebuild green |
| 6 | WS-06 | C-WRITER-GEN | code | 7.0 | bump-fail observability without failing durable commit; **rollback required** |

## Workstream tags (coverage)

- **code:** C-SQL-UNWRAP, C-WRITER-GEN
- **verification:** C-CLIPPY-DWARN, C-IGNORE-DRIFT
- **product:** C-CONTRIBUTING, C-HANDBOOK, C-DOCS-MCP, C-B10-DIST

## Deferred leftovers

| id | score | why |
| --- | ---: | --- |
| C-HUB-SIZE | 4.0 | Risk 9; leave-alone hubs |
| C-PRODUCT-EXPECT | 3.2 | Score < 4.0 |
| C-CATCHALL-PATTERN | 2.3 | Score < 4.0; language defaults |
| C-SWALLOW-LET | 1.5 | Score < 4.0 |
| C-FORCE-SIDECAR | 1.3 | Risk 8; F-003 leave-alone (clippy is WS-01) |

## Queue

Implementer packets: `.code-upgrade-enterprise/runs/20260813-043427/work-queue/` (P0–P5). No `.beads` this campaign.

## Out of scope this pass

No implementation. No push. Next pass: WS-01 clippy const TLS.

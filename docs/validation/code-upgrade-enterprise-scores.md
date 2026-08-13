# code-upgrade-enterprise scored candidate matrix

Campaign: dedicated-session · books `20260813-043427` · PIN `4291982` · inventory `6c4b180` · risk `9d6ab8e` · branch `refactor/de-monolithize-isomorphic` (PR 29).

Artifact (gitignored): `.code-upgrade-enterprise/runs/20260813-043427/03-candidate-matrix.md`.

**Score-Or-Defer only.** Formula: Impact × Confidence ÷ Risk (1–10 each). Enter later set when Score ≥ 4.0 and Risk < 8 (or Risk ≥ 8 with filled rollback). No product code edits. No upgrade-set selection (`04` next).

## Eligible later set (Score ≥ 4.0)

| id | surface | category | I | C | R | score | verification sketch |
| --- | --- | --- | ---: | ---: | ---: | ---: | --- |
| C-CONTRIBUTING | `CONTRIBUTING.md` crate table missing mmap/codemode/napi | product | 5 | 9 | 1 | **45.0** | table ↔ 11 workspace members |
| C-CLIPPY-DWARN | `index.rs:25` `missing_const_for_thread_local` (MEASURED exit 101) | verification | 8 | 9 | 2 | **36.0** | `rch exec -- cargo clippy --workspace --all-targets -- -D warnings` |
| C-HANDBOOK | `surface-parity.md:9` Pi `handbook` ghost (no `docs/handbook/`) | product | 5 | 9 | 2 | **22.5** | parity row matches tree |
| C-IGNORE-DRIFT | **Preserve** (WS-03): measured **488/0/4** at `ebac2ad`; 3 `#[ignore]` + codemode doctest ignore fence at `lib.rs:29` -- see `code-upgrade-enterprise-ignore-drift.md` | verification | 6 | 7 | 2 | **21.0** | measured gate; no product change |
| C-B10-DIST | `packages/pi/extension/dist` 24 tracked; prefer freshness gate over un-commit | product | 6 | 8 | 3 | **16.0** | `npm run check:pi-contract` red on src/dist skew |
| C-SQL-UNWRAP | `sql.rs:179`, `queries.rs:480`, `codemode/plan.rs:116` | code | 5 | 8 | 3 | **13.3** | core parity + codemode `$ref` tests |
| C-DOCS-MCP | `docs/mcp.md` hybrid lead + example `crates/core/...` path | product | 3 | 7 | 2 | **10.5** | example path real or marked illustrative |
| C-WRITER-GEN | `advertise_writer_generation` fail-open; read `unwrap_or(0)` | code | 7 | 6 | 6 | **7.0** | bump-fail observability without failing durable commit; MCP reopen |

Broad later select can span **code** + **verification** + **product** from this list.

## Deferred

| id | score | why |
| --- | ---: | --- |
| C-HUB-SIZE | 4.0 | Risk **9** ≥ 8, no rollback; leave-alone hubs (`demonolith-leave-alone.md`) |
| C-PRODUCT-EXPECT | 3.2 | Score < 4.0; ~15 product expects mostly post-invariant |
| C-CATCHALL-PATTERN | 2.3 | Score < 4.0; `pattern.rs` language defaults -- high blast |
| C-SWALLOW-LET | 1.5 | Score < 4.0; cleanup swallows mostly intentional |
| C-FORCE-SIDECAR | 1.3 | Risk **8**; not leftover -- F-003 leave-alone test inject; clippy touch is C-CLIPPY-DWARN |

## Crevice coverage

Pi dist · CONTRIBUTING · handbook · writer_generation · sql unwrap · catch-all `pattern.rs` · ignore drift · clippy · `docs/mcp.md` · leftover FORCE_SIDECAR -- all scored above (select or defer with reason).

## Out of scope this pass

No implementation. No `04-selected-upgrade-set.md`. No push. No `.beads`.

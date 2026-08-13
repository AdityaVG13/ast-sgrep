# code-upgrade-enterprise — campaign convergence

Branch `refactor/de-monolithize-isomorphic` (PR 29). Skill-loop stop: **two consecutive ZERO-CHANGE residual scans** (passes 19 and 20). Empty `git diff` after each.

Campaign books (gitignored): `.code-upgrade-enterprise/runs/20260813-043427/` plus `20260813-zerochange-pass20/`.

## Standing gates (last measured)

- Suite: **488 passed / 0 failed / 4 ignored** (`rch exec -- cargo test --workspace --no-fail-fast`)
- Clippy: `cargo clippy --workspace --all-targets -- -D warnings` green after WS-01 (`955d04a`)
- Public API: empty or additions-only vs main (demonolith contract; not re-run on docs-only residual scans)

## Selected set (WS-01..06) — Lift / Preserve

| WS | candidate | resolve | commit |
| --- | --- | --- | --- |
| WS-01 | C-CLIPPY-DWARN | Lift | `955d04a` |
| WS-02 | C-CONTRIBUTING · C-HANDBOOK · C-DOCS-MCP | Lift | `ebac2ad` |
| WS-03 | C-IGNORE-DRIFT | Preserve (4 ignored = 3 `#[ignore]` + 1 doctest fence) | `f82e4fd` |
| WS-04 | C-SQL-UNWRAP | Lift | `2f7d937` |
| WS-05 | C-B10-DIST | Lift; adversary closed untracked-emit hole | `ba12cdb`, `4d885d9` |
| WS-06 | C-WRITER-GEN | Preserve fail-open after durable SQLite; advertise after partial watch-batch error | `2442661`, `f06d0e6` |

Follow-on lifts in the same campaign: `check:agent-plugin` in CI (`1d9f14e`); Homebrew formula v1.4.0 + VS Code langs = `Language::all()` (`ec5871f`); CONTRIBUTING CI / release-gate / sealed-unsafe truth (`08d895a`).

## Deferred (do not reopen without new Score ≥ 4.0 evidence)

| id | score | why |
| --- | ---: | --- |
| C-HUB-SIZE | 4.0 | Risk 9; leave-alone hubs (`index.rs` ~1034, MCP `lib.rs` ~1005) |
| C-PRODUCT-EXPECT | 3.2 | Score < 4.0; remaining `expect` after proven invariants |
| C-CATCHALL-PATTERN | 2.3 | Score < 4.0; language defaults |
| C-SWALLOW-LET | 1.5 | Score < 4.0 |
| C-FORCE-SIDECAR | 1.3 | Risk 8; F-003 test inject |
| C-LEX-DIRTY-OR-TRUE | 3.0 | below enter floor |
| C-INDEX-DB-PATH-FAILOPEN | 3.4 | below enter floor |
| LSP busy mutex | — | Risk 9; no rollback |
| embed infallible expects / runtime swallow | — | scored out or Risk ≥ 8 |

Writer-generation: **Refuse** fail-closed advertise after successful SQLite commit without explicit user approval.

## Residual scans that wrote nothing

Pass 17 (post-Homebrew), pass 19 (post-CONTRIBUTING), pass 20 (README crate/bin names, `docs/index-consistency.md` ↔ writer-gen fail-open, `package.json` `check:pi-dist` + `check:agent-plugin` vs CI, no `TODO`/`FIXME` in `crates/*/src`). Pass 20 named a completeness nit (README omits internal mmap/napi crates) and refused it as not a false claim.

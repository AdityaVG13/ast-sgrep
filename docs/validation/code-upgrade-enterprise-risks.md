# code-upgrade-enterprise risk map

Campaign: dedicated-session · books `20260813-043427` · PIN `4291982` · inventory `6c4b180` · branch `refactor/de-monolithize-isomorphic` (PR 29).

Artifact (gitignored): `.code-upgrade-enterprise/runs/20260813-043427/02-risk-map.md`.

**This pass names risks with evidence. No product fixes. No upgrade-set selection (Score later).**

## Clippy re-measure

```bash
PATH=$HOME/.local/bin:$PATH rch exec -- cargo clippy --workspace --all-targets -- -D warnings
```

| | |
| --- | --- |
| Result | **exit 101** (~58s remote on Spark) |
| Blocking | `clippy::missing_const_for_thread_local` at `crates/ast-sgrep-core/src/index.rs:25` |
| Hint | use `const { Cell::new(false) }` for the `thread_local!` initializer |
| CI | `.github/workflows/ci.yml` clippy job uses `-D warnings` (with `--release`) |

Historically red on main; now confirmed red on this worktree. Residual warnings after this fix are unknown (compile aborted on `ast-sgrep-core`).

## Top risks (evidence)

| ID | Risk | Evidence |
| --- | --- | --- |
| R-CLIPPY-DWARN | Clippy `-D warnings` fails | `index.rs:25`; CI `ci.yml` clippy job |
| R-WRITER-GEN-FAILOPEN | Writer stamp advertise fail-open after commit | `index.rs:782-789` swallows bump Err; `writer_generation.rs:61-66` read → `unwrap_or(0)` |
| R-B10-DIST | Committed Pi `extension/dist` drift | 24 tracked paths under `packages/pi/extension/dist`; not gitignored |
| R-PRODUCT-UNWRAP | Product panic paths | unwrap: `store/sql.rs:179`, `sqlite/queries.rs:480`, `codemode/plan.rs:116`; product expect ≈15 |
| R-DOCS-LIES | Docs claim missing surfaces | `surface-parity.md:9` → Pi `handbook` but no `docs/handbook/`; `CONTRIBUTING.md` crate table omits mmap/codemode/codemode-napi |
| R-HUB-SIZE | Oversized leave-alone hubs | `index.rs` ~1034, `mcp/lib.rs` ~1005 — documented leave-alone, not free split debt |
| R-IGNORE-DRIFT | Ignored-test count vs baseline | 3 `#[ignore]` found vs inherited baseline 4; suite not re-run on PIN |
| R-CATCHALL | Dense default match arms | 51 `_ =>` arms (hotspot `lang/pattern.rs`) |

## Already mitigated (preserve)

| Corner | Evidence |
| --- | --- |
| Schema newer than binary refuse | `store/sqlite/mod.rs:166-169` |
| Symlink non-follow + watch jail | `index.rs:521` `.follow_links(false)`; `index_watch.rs:10-19` |
| MCP sandbox path checks | `mcp/src/sandbox.rs:74-95`; `lib.rs:586-608` `sandbox_root` |
| Forbid-soundness | `SECURITY.md`; sole hand-written unsafe `ast-sgrep-mmap/src/lib.rs`; PR `verify-forbid-soundness` |
| Clean product panic hygiene (MCP/LSP/lang) | unwrap/expect = 0 in those crates' `src/` |
| Pi TypeBox + output budgets | `packages/pi/extension/src/index.ts` TypeBox + char caps |

## Out of scope this pass

No Score / selected upgrade set / implementation. Next books: `03-candidate-matrix.md` then select. Re-run primary test gate before treating 488/0/4 as current HEAD truth.

# Adversarial review — code-upgrade-enterprise product delta

Branch `refactor/de-monolithize-isomorphic` · FAILURE-MODES pass on upgrade commits after demonolith (`955d04a`, `ebac2ad`, `f82e4fd`, `2f7d937`, `ba12cdb`, `2442661`), not the isomorphic extract unless regressed.

Artifact: `.code-upgrade-enterprise/runs/20260813-043427/07-diff-review.md` (gitignored).

## Finding fixed

| id | class | issue | fix |
| --- | --- | --- | --- |
| C-B10-DIST-HOLE | correctness / test | `check:pi-dist` only ran `git diff --exit-code`, so **untracked** `tsc` emit under `packages/pi/extension/dist` did not fail CI | `package.json` now requires empty `git status --porcelain` on that tree; `check-contract.mjs` locks the script; `docs/RELEASING.md` updated |

Evidence: `touch packages/pi/extension/dist/__adversary_untracked.js` → old gate diff exit 0; new gate exit 1. Clean tree: `npm run check:pi-dist` exit 0.

## Checks that already pass (no product change)

1. **Clippy `-D warnings`** — `rch exec -- cargo clippy --workspace --all-targets -- -D warnings` → exit **0** (const TLS cells, agent `?`, mcp `#[cfg(test)]` import for `force_sidecar_rebuild_err`).
2. **Pi contract lock** — `npm run check:pi-contract` → **Pi release contract is consistent at 1.4.0** (includes `check:pi-dist` script pin).
3. **CONTRIBUTING ↔ workspace members** — table lists all **11** `Cargo.toml` members including `ast-sgrep-mmap`, `ast-sgrep-codemode`, `ast-sgrep-codemode-napi`.
4. **Docs truth** — `docs/mcp.md` no-auto-fusion lead matches MCP tool copy; example path `crates/ast-sgrep-core/src/search/mod.rs` exists; `surface-parity.md` Doctor row is `/asgrep-doctor` (present in Pi release-contract commands).
5. **Ignore-drift preserve** — three `#[ignore]` sites + codemode doctest ` ```ignore ` at `lib.rs:29` reconcile suite **4** ignored; see `code-upgrade-enterprise-ignore-drift.md`.
6. **Unwrap delta** — `query_imports` `Option` filter is isomorphic to prior `is_none_or` + `unwrap`; empty-`$ref` still errors before `split`.
7. **Writer-generation fail-open** — `advertise_writer_generation` still logs and skips on stamp I/O Err; `read_writer_generation` still returns `0` on absence; docs/log match code.

## Verdict

Upgrade delta **not rejected** after the dist-gate fix. Kill criteria (correctness / security / reporting) clean for this surface.

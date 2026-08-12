# RESULT — Wave 2 / Pass 3 (HARDEN R-CM-ROOT-POLICY Option A)

```text
SPIN_THE_BLOCK_RESULT:
status: complete
mode: harden
target_root: /Users/aditya/Developer/ast-sgrep
prior_state_leveraged: true
wave: 2
campaign_pass: 3
iteration: 15
product_safe: true
product_source_edits: 4
residual_closed: R-CM-ROOT-POLICY
product_decision: Option A (jail CM/NAPI root like MCP)
bead: ast-sgrep-rca-residuals-sp6p.1
bead_also: ast-sgrep-of45
technique: Session sandbox_root (canonicalize + Path::starts_with) + catalog + NAPI rustdoc cascade
axes_changed: 3
axes: representation:policy-lattice | observer:attacker | scale:boundary
frozen_revision_pass1: 62ee4b4595ad2433bd16b0ac14747dada612b4d6
head_at_verify: 2c76d8b12fd594af2f2ab1801b85aa80d6c98819
dirty: true
dirty_note: product CM/NAPI root jail only; no xproc/docs-bundle/Pi runtime
zerostack: unavailable-fszero-codemode
independent: n/a-this-pass (originator harden; pass 11 dual-evidence CONFIRMED)
braid_resolve: Continue
NEXT_PASS: Harden R-XPROC-MULTIWRITER (smallest closed-fail)
PRODUCTIVE: true
```

## Product decision (recorded)

**Option A accepted:** Code Mode / NAPI tool `root` is jailed under the configured session/workspace root the same way MCP does (`canonicalize` + `Path::starts_with` / contained-in-root). Message: `escapes configured workspace`.

## Gate

- [x] `root_arg` fail-closed outside workspace
- [x] NAPI inherits Session (no bypass; rustdoc records contract)
- [x] Foreign root + shared `index_path` refused; pinned DB size unchanged
- [x] Nested root under workspace still allowed
- [x] Catalog `root` schema text updated
- [x] RCH verify `--lib` + catalog/session_plan
- [x] Axes ≥2 vs pass 2
- [x] No xproc / docs-bundle / Pi runtime edits

## Diff summary (product)

| File | Change |
|------|--------|
| `crates/ast-sgrep-codemode/src/session.rs` | `sandbox_root` + `root_arg` → `Result`; foreign-root unit test |
| `crates/ast-sgrep-codemode/src/catalog.rs` | `ROOT_ARG_DESC` on all tool `root` props |
| `crates/ast-sgrep-codemode/src/lib.rs` | crate rustdoc Option A |
| `crates/ast-sgrep-codemode-napi/src/lib.rs` | NAPI cascade rustdoc (inherits Session) |

## Verify

```text
rch exec -- cargo test -p ast-sgrep-codemode --lib
  ok. 2 passed (incl. foreign_root_is_rejected_under_session_workspace)
rch exec -- cargo test -p ast-sgrep-codemode --test catalog --test session_plan
  ok. catalog 3 + session_plan 4
```

## Braid

**Freeze(retained) → Axis(policy-lattice+attacker+boundary) → Enact(Session jail Option A) → Independent n/a → Residual(R-CM-ROOT closed; R-XPROC/OPS open) → Resolve Continue**

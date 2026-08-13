# RESULT — Wave 2 / Pass 5 (HARDEN R-OPS-DOCS-FOOTGUNS)

```text
SPIN_THE_BLOCK_RESULT:
status: complete
mode: harden
target_root: /Users/aditya/Developer/ast-sgrep
prior_state_leveraged: true
wave: 2
campaign_pass: 5
iteration: 17
product_safe: true
product_source_edits: yes
residual_closed: R-OPS-DOCS-FOOTGUNS
bead: ast-sgrep-rca-residuals-sp6p.4
technique: doctor FastUnsafe issue + status durability print + ops/docs honesty (index_path pin, CM/NAPI jail, C1 cascade, ESC-3 deadline)
axes_changed: 3
axes: observer:operator | representation:lifecycle-runbook | evidence:docs+source
vs_pass4: observer:scheduler→operator; representation:interleaving→lifecycle-runbook; evidence:stamp/tests→docs+source
frozen_revision_pass1: 62ee4b4595ad2433bd16b0ac14747dada612b4d6
head_at_verify: (dirty working tree; product edits uncommitted)
dirty: true
dirty_note: ops/docs footgun packet only; no Searcher/root/xproc reopen; no Pi leftover
zerostack: unavailable-fszero-codemode
independent: n/a-this-pass (originator harden; pass 11 dual-evidence not required for medium/low docs)
braid_resolve: Continue
NEXT_PASS: Seal wave-2 residuals / optional Pi leftover only if authorized; else campaign close
PRODUCTIVE: true
void_fixture_outcome: n/a mid-wave harden
north_star_probe_outcome: n/a product harden
independent_loop27: n/a
```

## Gate

- [x] Doctor surfaces FastUnsafe (`durability_fast_unsafe`) when status or `--durability`/`ASGREP_DURABILITY` is fast-unsafe
- [x] Status human print includes Durability (JSON already had field)
- [x] Docs: privileged `ASGREP_INDEX_PATH`, pin disables gen reindex, NAPI/CM jailed host duty (not free root)
- [x] C1 cascade docs aligned with lexical-fallback-on-empty-structural (+ historical mismatch note)
- [x] ESC-3: post-mutation deadline error notes index may have committed
- [x] Axes ≥2 vs pass 4
- [x] RCH verify doctor unit tests + MCP lib compile/tests

## Diff summary (product)

| File | Change |
|------|--------|
| `crates/ast-sgrep-cli/src/agent.rs` | `doctor_fast_unsafe_issue` + robot-docs ops footguns + 3 unit tests |
| `crates/ast-sgrep-cli/src/index_cmd.rs` | human status prints Durability |
| `crates/ast-sgrep-mcp/src/lib.rs` | ESC-3 deadline error honesty |
| `docs/cascade-query-planner.md` | align stop table with code; historical C1 note |
| `docs/semantic-search.md` | hybrid cascade wording |
| `docs/env-trust.md` | INDEX_PATH privilege, durability, CM/NAPI jail |
| `docs/index-consistency.md` | pinned path disables gen reindex; durability |
| `docs/mcp.md` | env table + deadline commit honesty |
| `docs/codemode.md` | NAPI/Session root jail host duty |
| `docs/getting-started.md` | INDEX_PATH privileged sink note |

## Verify

```text
RCH_CANONICAL_PROJECT_ROOT=/Users/aditya \
rch exec -- env CARGO_TARGET_DIR=… cargo test -p ast-sgrep-cli --lib doctor_surfaces
  ok. 3 passed (FastUnsafe status/cli + balanced silent)

rch exec -- … cargo test -p ast-sgrep-mcp --lib
  ok. 5 passed (compile confirms ESC-3 string change)
```

## Braid

**Freeze(retained) → Axis(operator+lifecycle-runbook+docs/source) → Enact(doctor+docs+ESC-3) → Independent n/a → Residual(R-OPS-DOCS closed; optional B-ZS-ENGINES tooling host-local) → Resolve Continue**

## Failure modes (named)

1. Doctor marks FastUnsafe as unhealthy (exit 2) even when index is otherwise fine — intentional ops gate.
2. Pre-start deadline error unchanged (no mutation yet) — only post-mutation path notes commit.
3. Docs-only residual B-ZS-ENGINES (tokenzero missing) remains host tooling, not product.

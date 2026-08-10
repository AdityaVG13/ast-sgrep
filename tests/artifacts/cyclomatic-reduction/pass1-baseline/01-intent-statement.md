# Intent Statement

Run ID: `2026-08-10T235424Z-baseline`
Date: `2026-08-10T23:56:42Z`
Mode gate: **baseline-only** (Pass 1 of 12)
Branch: `perf/software-optimization` (open PR #27)
Target root: `/Users/aditya/Developer/ast-sgrep`

## Campaign

Repo-sweep multipass campaign for **ΣCC reduction** on the PR branch.
North star: lower total decision-point density (ΣCC) while preserving exact behavior
(differential parity). Do **not** redesign APIs, rename public surfaces, or game the metric
via helper-fan-out dumps without a justified displacement bill.

## This pass (Pass 1)

- Preflight (lizard present; no whole-workspace suite)
- Full product-scope complexity census
- Baseline report + ranked target ledger
- No product code edits

## Scope (product code)

| Included | Notes |
|---|---|
| `crates/**/*.rs` | All Rust crates (cli, core, lang, lsp, mcp, plugins, embed, codemode, mmap, testkit, napi) |
| `packages/pi/extension/src/**` | Pi extension TypeScript sources |
| `packages/pi/launcher/src/**` | Pi launcher JS sources |

| Excluded | Reason |
|---|---|
| `target/`, `target-pass*` | Build artifacts (~1.6G); would dominate walk |
| `node_modules/` | Vendored deps |
| `packages/**/dist` | Generated/compiled output |
| Packages test trees | Deferred; crates tests remain inside crates measure |
| Skill folder / `.cyclomatic-reduction` | Not product |

## Success criteria (campaign, not this pass)

- ΣCC of scoped surface declines across waves without unjustified displacement
- Hotspots above ceiling (CC > 10) reduced via named techniques
- Parity proven per transform wave
- Remaining hotspots queued (beads or work-queue packets)

## Explicit non-goals this pass

- No transforms, no refactors, no API changes
- No scorecard grading
- No inventing performance numbers beyond lizard/measure_complexity output

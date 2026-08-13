# Demonolith census — Phase 1

**Run:** `2026-08-13-ast-sgrep-wt-demonolith-1`  
**Target:** `/Users/aditya/AI/ast-sgrep-wt-demonolith` @ `origin/main` (accb010)  
**Mode:** Standard · inventory-only toolchain (no auto-install)  
**Workspace:** `../ast-sgrep-wt-demonolith__demonolith_workspace/`

## Thresholds

| Knob | Value | Notes |
|---|---|---|
| Soft (RS / TS / JS / TSX) | **1000** code LOC | Override via `DEMONOLITH_SOFT_RS=1000` (and TS/JS/TSX) to match the repo 1k product-file rule. Skill defaults are 5000 / 2500. |
| Hard | 10000 LOC | Unchanged |
| LOC source | tokei (high confidence) | scc missing — degraded |
| Complexity | lizard density | |
| Files scanned | 274 | |

## Over-threshold unified table

| file | LOC | complexity | churn (all / 180d) | buckets | severity-prior |
|---|---:|---:|---|---|---|
| `crates/ast-sgrep-core/src/store/sqlite.rs` | 1607 | 0.286 | 55 / 55 | B1, B3, B5(test) | must-split (57.0) |
| `crates/ast-sgrep-core/src/index.rs` | 1357 | 0.231 | 44 / 44 | B1, B3, B5(test) | must-split (48.5) |
| `crates/ast-sgrep-core/src/search/mod.rs` | 1111 | 0.115 | 62 / 62 | B4, B5 | should-split (42.1) |
| `tests/cli/machine_contracts.rs` | 1023 | 0.055 | 20 / 1 | B9 | borderline (19.4) |
| `tests/core/metamorphic.rs` | 1166 | 0.075 | 4 / 1 | B9 | borderline (19.1) |

No file hit the hard 10k trigger. No Rust `expand_candidate` (>2000 LOC).

## Top offenders (product)

1. **`store/sqlite.rs`** — IndexStore persistence: schema/tx, upserts, query surface, semantic/embed IO cohabiting (B1+B3). Highest severity. Behind `store/mod.rs` façade but fails B11 stable-churn.
2. **`index.rs`** — Indexer pipeline + corrupt recovery + watch + sidecars (B1+B3).
3. **`search/mod.rs`** — Searcher hub over already-split passes; Mutex caches (B4+B5); highest churn (62).

## Generated (B10)

`packages/pi/extension/dist/**` — `tsc` emit (`outDir: dist`). Exclude from hand-splitting; fix at generator/build if ever a compile bottleneck. Census marker sweep did not flag them (`generated: false`); build-system evidence overrides.

## B11

No confirmed justified monolith among over-threshold rows this pass. `sqlite.rs` is storage-layer-shaped (candidate only). `metamorphic.rs` is cohesive but remains B9.

## Watchlist (under soft)

| file | LOC | severity_prior | note |
|---|---:|---:|---|
| `crates/ast-sgrep-mcp/src/lib.rs` | 972 | 39.9 | Prior ~1140 was `wc -l`; would be B4+B5 if promoted |
| `packages/pi/extension/src/runtime.ts` | 775 | 45.9 | High density×churn despite under LOC |
| `crates/ast-sgrep-lang/src/extract.rs` | 649 | 31.3 | |
| `crates/ast-sgrep-core/src/semantic_ann.rs` | 631 | 42.7 | |

## Toolchain inventory

**Present:** tokei, cloc, ast-grep, semgrep, lizard, hyperfine, jq, rust-nightly, cargo-modules, cargo-llvm-lines, cargo-public-api, cargo-semver-checks, cargo-llvm-cov, cargo-expand, cargo-depgraph, cargo-udeps.

**Skipped / degraded:**

- `scc` — missing (`go install github.com/boyter/scc/v3@latest`); LOC via tokei, complexity via lizard.
- TypeScript graph tools (madge / dependency-cruiser / knip) — not probed; inventory detected `languages: [rust]` only (phase0 also lists typescript for `packages/pi`).
- Host note: macOS `/bin/realpath` lacks `-m`; census ran with coreutils `grealpath` shim.

## Census command

```bash
export DEMONOLITH_SOFT_RS=1000 DEMONOLITH_SOFT_TS=1000 DEMONOLITH_SOFT_JS=1000 DEMONOLITH_SOFT_TSX=1000
# PATH shim: grealpath as realpath (macOS)
bash "$SKILL_DIR/scripts/census.sh" "$PROJECT" "$WORKSPACE"
```

**Result:** exit 0 · 5 over-threshold · artifacts `census.json`, `census_notes.json`, `churn_coupling.json`, `phase1_monolith_census.md`.

Full slice-by-slice idiom + bucket evidence: workspace `phase1_monolith_census.md`.

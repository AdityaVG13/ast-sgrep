# Named checks — Pass 12 absolute convergence

Scope: `crates/` · `packages/pi/extension/src` · `packages/pi/launcher/src`  
Analyzer: lizard 1.23.0 via `measure_complexity.py --threshold 10`  
Bill anchor: `2026-08-11Tpass10-bill` / `bill-summary.json`  
This run: `2026-08-11Tpass12-convergence`

| # | Check | Expected | Observed | Outcome |
|---|---|---|---|---|
| N1 | Full-scope ΣCC equals pass 10 bill | 5994 | **5994** | **PASS** (Δ 0) |
| N2 | Max CC / median / mean match pass 10 | 26 / 2 / 3.07 | **26 / 2 / 3.07** | **PASS** |
| N3 | Functions + hotspots CC>10 match pass 10 | 1953 / 83 | **1953 / 83** | **PASS** |
| N4 | Parts ΣCC match (crates / extension / launcher) | 5090 / 802 / 102 | **5090 / 802 / 102** | **PASS** |
| N5 | Product source diff empty for authorized trees | no changes under `crates/`, `packages/pi/extension/src`, `packages/pi/launcher/src` this pass | `git diff --stat -- crates packages/pi/extension/src packages/pi/launcher/src` → empty | **PASS** (ZERO product source edit) |
| N6 | Top residual head stable vs pass 10 top-20 | same head functions/CC | resolveHost 26, read_header 25, readLineWindow 25, run_bench_suite 24, … identical top-20 | **PASS** |
| N7 | `resolveHost` pure extract still non-fundable | would raise file/package ΣCC | Pass 9 measured **+6** on launcher `index.js` for assert/addon pure extract; residual is PATH/platform/checksum taxonomy (requisite error codes). D1 gate: file ΣCC ≤ 102 | **REFUSE / Keep residual** |
| N8 | `run_bench_suite` / `run_process` pure extract non-fundable | case-loop identity / error ladder | Pass 9: `run_process` extract **+3 Refuse**; `measure_suite_case` pure extract **+3 Refuse**. Suite residual is case-enumeration + machine-contract packing (D2) | **REFUSE / Defer-only** |
| N9 | `regex_pass` multi-helper dump non-fundable | no +Σ helper fan-out | Pass 8 refused walk/regex multi-helper dumps; residual 18 is budget/trigram/context domain (D3). Only shared-collapse allowed | **REFUSE pure extract / Keep residual** |
| N10 | Essential Keep ledger untouched for score | read_header, readLineWindow, classify_native, apply_weighted_rrf, … | Still top residual; no edits; domain parsers/validators | **PASS Keep** |
| N11 | Displacement vs campaign baseline still PASS | ΣCC ≤ 6022 | 5994 ≤ 6022 (−28) | **PASS** |
| N12 | Two consecutive product-zero passes before this one | pass 10 + pass 11 ZERO product | documented; this pass also ZERO | **PASS → CONVERGED chain** |

## Refuse / Keep reasons (≥3 named residual accidental heads)

### 1. `resolveHost` (CC 26) — Refuse pure extract

- **History:** Pass 3 guards applied; pass 9 pure extract of `assertHostManifestMatches` + addon helpers measured **file ΣCC +6** → Refuse (not kept).
- **Structure now:** Platform/arch/libc mapping, optional-host miss taxonomy, sequential metadata mismatch guards, version mismatch — requisite host-resolution domain; PATH fallback codes fixed contract.
- **Why not fundable:** Any pure extract moves decisions without eliminating them → base cost of new functions raises ΣCC. Shared-collapse only if **duplicate** trees across resolveHost / resolveBinary / resolveCodemodeAddon prove removable; none newly discovered this scan.
- **Disposition:** **Defer** (D1) / permanent Keep residual until bill-negative shared-collapse appears.

### 2. `run_bench_suite` (CC 24) — Refuse ladder extract / case-loop vanity

- **History:** Pass 9 shared collapse on bench ratchet + human print reduced suite 29→24 and package −2; pure extract of measure/print helpers **+3 Refuse**.
- **Structure now:** Case loop over suite identity contracts, skip/index paths, CV/envelope packing — largely case-loop identity (Ashby Keep-lean).
- **Why not fundable:** Further pure extract of loop body reintroduces helper base cost without eliminating decisions; machine envelope / ratchet semantics are contract-sensitive.
- **Disposition:** **Defer** (D2) residual case-loop identity.

### 3. `run_process` (CC 16) — Refuse error-path extract

- **History:** Pass 9 extract to exit_clap/exit_run → **lib.rs +3 Refuse**.
- **Structure now:** Clap parse error ladder + colour/color tips + machine failure vs agent footer — agent UX domain + short-circuit order sensitive.
- **Why not fundable:** Extract relocates branches; does not remove decision points.
- **Disposition:** **Defer** (D2); do not re-attempt without measured −ΣCC plan.

### 4. `regex_pass` (CC 18) — Refuse multi-helper fan-out

- **History:** Pass 8 multi-helper walk/regex dumps refused (+Σ class); successful wave was shared collapse elsewhere (literal/IVF write linear displacement only).
- **Structure now:** Pattern length/budget, case flag, trigram candidate gate, thread/chunk plan, context file map — search correctness domain.
- **Why not fundable:** Fan-out adds helper CC without net Σ reduction; search/regex budget tests pin behavior.
- **Disposition:** **Defer** (D3); shared-collapse only if duplicate gates with sibling passes proven.

### 5. Keep domain heads (not accidental fundable)

- `read_header` 25 (IVF on-disk format), `readLineWindow` 25 (protocol), `read_clusters_bounded` 22, `apply_weighted_rrf` 21, `classify_native` 20 — **frozen Keep**; cutting for score would smash domain.

## Productive-cut scan result

**No fundable accidental cut found** that would lower ΣCC without raising it or smashing domain.  
Verdict input: **CONVERGED** (not PRODUCTIVE).

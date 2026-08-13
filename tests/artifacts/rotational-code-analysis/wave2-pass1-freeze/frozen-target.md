# Frozen target — Wave 2 / Pass 1 (HARDEN authorize + re-freeze)

| Field | Value |
|---|---|
| **target_root** | `/Users/aditya/Developer/ast-sgrep` |
| **git_revision (NEW FREEZE)** | `62ee4b4595ad2433bd16b0ac14747dada612b4d6` |
| **prior_wave1_freeze** | `fb932aac852f5496c0a7035cc5a0b508e05111cb` |
| **branch** | `perf/software-optimization` (PR #27) |
| **upstream** | `## perf/software-optimization...origin/perf/software-optimization` |
| **dirty** | **true** (38 short status lines) |
| **action_mode** | `harden` (user-authorized product fixes on PR #27) |
| **campaign** | rotational-code-analysis **wave 2** pass 1 of 12 — Freeze + harden authorize |
| **skill** | rotational-code-analysis 2.0.0 (skill folder read-only) |
| **frozen_at** | `2026-08-12T16:24:44Z` |
| **wave1_snapshot_sha256 (retained)** | `c7b14742a308e688ced488c9b7828b27de13703ffd9785c8835cf3b0cb24d9fb` |
| **snapshot_policy** | **V-STATE-IGNORE** — do not re-init/re-census; freeze identity = `git rev-parse HEAD` + dirty note |
| **state_file** | `.rotational-code-analysis/state.json` |
| **zerostack** | unavailable (`fszero-codemode` missing); shell/`rg` used |

## Axes this rotation (≥2 vs wave-1 last)

Wave-1 last rotation (pass 11/12): observer skeptic · evidence reread/tests · scale residual-only · representation dual-evidence/coverage-check · time retained `fb932aac852f`.

| Axis | This pass | Changed? |
|---|---|---|
| time | **new-freeze** (`62ee4b4595ad`) | **yes** |
| observer | **operator-harden** | **yes** |
| evidence | **git+state** | **yes** |
| scale | campaign-reentry | yes (extra) |

`axes_changed`: **3** (time, observer, evidence)

## Scope (operator freeze)

- **In product (authorized harden later):** Rust `crates/*`, relevant docs/tests for residual R-* packets.
- **This pass only:** Freeze + Axis record + load prior state. **No product code edits.**
- **Out of scope / do not touch:** unrelated dirty Pi files (`packages/pi/extension/src/runtime.ts`, `index.ts` rg/freshness work) and their dist/tests leftovers.
- **V-STATE-IGNORE:** do **not** re-do census/architecture; retain wave-1 coverage books.

## Dirty tree summary (at freeze)

### Beads / tracker (local runtime leftover)

```
 D .beads/.br_history/issues.20260806_020353_251237000.jsonl
 D .beads/.br_history/issues.20260806_020353_251237000.jsonl.meta.json
 D .beads/.br_history/issues.20260806_020404_459224000.jsonl
 D .beads/.br_history/issues.20260806_020404_459224000.jsonl.meta.json
 D .beads/.br_history/issues.20260806_020451_346418000.jsonl
 D .beads/.br_history/issues.20260806_020451_346418000.jsonl.meta.json
 D .beads/.br_history/issues.20260806_020457_174453000.jsonl
 D .beads/.br_history/issues.20260806_020457_174453000.jsonl.meta.json
 D .beads/.br_history/issues.20260806_032753_121249000.jsonl
 D .beads/.br_history/issues.20260806_032753_121249000.jsonl.meta.json
 M .beads/beads.db
 M .beads/beads.db-wal
 M .beads/issues.jsonl
 M .beads/last-touched
```

### Pi extension leftover (OUT OF SCOPE for this mission)

```
 M packages/pi/extension/dist/code-mode.js
 M packages/pi/extension/dist/codemode/dispatch.d.ts
 M packages/pi/extension/dist/codemode/dispatch.js
 M packages/pi/extension/dist/index.js
 M packages/pi/extension/dist/runtime.js
 M packages/pi/extension/src/index.ts
 M packages/pi/extension/src/runtime.ts
 M packages/pi/extension/test/runtime.test.ts
 M packages/pi/extension/test/skill-workflow.test.ts
 M packages/pi/extension/test/tools.test.ts
```

### Other modified / untracked

```
 M .papercuts.jsonl
 M Cargo.lock
?? .skill-loop-progress-conformance.md
?? .skill-loop-progress-fuzzing.md
?? .skill-loop-progress-gauntlet.md
?? .skill-loop-progress-golden-artifacts.md
?? .skill-loop-progress-rotational-code-analysis-harden.md
?? target-pass11/
?? target-pass13/
?? target-pass14/
?? target-pass15/
?? target-pass4/
?? target-pass8/
?? tests/artifacts/bug-hunt/
```

**Short status count:** 38 lines from `git status --porcelain=v1`.

## Prior state leveraged (mandatory)

Loaded `.rotational-code-analysis/state.json` first:

| Field | Prior value |
|---|---|
| run.iteration | 11 (campaign books; pass 12 seal in artifacts) |
| run.action | audit |
| run.status | IN_PROGRESS |
| campaign.mode | audit-12-pass |
| campaign.freeze_revision | `fb932aac852f5496c0a7035cc5a0b508e05111cb` |
| coverage.files | 48991 (spin-bloated); census_pass2 in_scope 523 tracked |
| residuals (pass 11/12) | R-CM-ROOT-POLICY, R-INDEX-ERR-CACHE-SYNC, R-XPROC-MULTIWRITER, R-OPS-DOCS-FOOTGUNS |

## Commits since wave-1 freeze

`git log --oneline fb932aac852f..HEAD` includes RCA passes 1–12 seal commits ending at `62ee4b4 skill-loop pass 12/12: RCA absolute convergence seal (audit)`.

## Workspace (not re-probed this pass)

Wave-1 freeze established 11 cargo members @ 1.4.0. Not re-running `cargo metadata` / census (V-STATE-IGNORE).

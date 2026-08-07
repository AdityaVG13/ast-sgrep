# Pass 6 — CI & Review Workflow for Goldens

**Date:** 2026-08-07  
**Branch:** `perf/software-optimization`  
**Skill:** `testing-golden-artifacts` + `references/CI-GOLDENS.md`  
**Prior:** PASS1–PASS5 under `tests/artifacts/golden-audit/`  
**Scope:** CI inventory, auto-update risk, failure artifact upload, developer/agent update SOP, PR review discipline, interaction with Agents.md honesty rules. **Audit only** -- no workflow edits, no beads, no commit.

---

## Executive summary

| Dimension | Status |
|-----------|--------|
| Compare-only in CI today | **Yes by accident** -- no `assert_golden` write path, no `ASGREP_UPDATE_GOLDENS` consumer yet |
| Explicit no-update guard in CI | **No** |
| Upload `*.actual` on test failure | **No** (`ci.yml` uploads nothing) |
| PR-triggered golden / test gate | **No** -- all workflows are `workflow_dispatch` only |
| Developer golden update SOP | **No written process** (env name decided in PASS2 only) |
| PR template golden checkboxes | **Absent** (no PR template at all) |
| Honesty baselines as CI goldens | **Correctly separate** -- ledger docs, not assert targets |
| CONTRIBUTING vs real CI triggers | **Drift** -- docs claim PR-run soundness/check; YAML does not |

**Maturity (CI golden hygiene):** ~2/10 infrastructure process, ~5/10 de-facto fail-closed. Harden **before or with** PASS2 P0 `assert_golden` write path.

---

## 1. Current CI inventory for test jobs

### 1.1 Workflow map

All seven workflows under [`.github/workflows/`](../../../.github/workflows/) trigger on **`workflow_dispatch` only**. None run on `pull_request` or `push`. No Makefile / justfile; local gates are shell + `cargo` + npm scripts.

| Workflow file | Purpose | Test / gate commands | Artifacts uploaded | Golden-related |
|---------------|---------|----------------------|--------------------|----------------|
| **`ci.yml`** (`CI`) | Main Rust quality matrix (manual) | See jobs below | **None** | Indirect: `cargo test` fails on fixture `assert_eq!` |
| **`speed.yml`** | Fixed speed harness + latency floor | `asgrep bench …` → `scripts/check-bench-output.py` | `speed-results.json` (`always()`) | Bench threshold, **not** output goldens |
| **`bakeoff.yml`** | Self-repo retrieval bake-off | `asgrep bench . --suite self` → check script | `bakeoff-results.json` (`always()`) | Same |
| **`install-smoke.yml`** | Post-crates.io install + docs.rs | `cargo install` version check; curl docs.rs | None | N/A |
| **`pi-cross-smoke.yml`** | macOS dual-arch build smoke | Build + `asgrep version` / `doctor` | None | N/A |
| **`pi-native-artifacts.yml`** | Multi-target native pack + install smoke + e2e | `release-artifact.mjs`, `ci-install-smoke.mjs`, `test:pi-e2e` | Per-target `dist/…`, `pi-npm-dry-run-*` | Structural release contract, not file goldens |
| **`pi-npm-release.yml`** | Official npm publish path | release gate, native matrix, pack/verify, `test:pi-e2e`, attest, publish | Native dists, `pi-npm-*` tarballs | Same; protected env secrets only |

### 1.2 `ci.yml` jobs (the golden-relevant surface)

| Job | OS | Command(s) | Notes |
|-----|-----|------------|-------|
| `forbid-soundness` | ubuntu | `bash scripts/verify-forbid-soundness` | No `if:` guard (redundant; workflow is dispatch-only) |
| `cargo-check` | ubuntu | `cargo check --workspace -j1` | Same |
| `build-and-test` | ubuntu + macos matrix | `cargo build --workspace --release` then **`cargo test --workspace --release`** | **Only job that runs full golden-bearing tests** |
| `windows-smoke` | windows | `cargo test -p ast-sgrep-cli --lib --release` + CLI smoke + cancel | Partial tests; not full workspace goldens |
| `clippy` | ubuntu | `cargo clippy --workspace --release --all-targets -- -D warnings` | |
| `fmt` | ubuntu | `cargo fmt --check` | |
| `audit` | ubuntu | `cargo audit` | |
| `bounded-fuzz` | ubuntu | cargo-fuzz `parsed_query`, `rank` (30s each) | |

Heavy jobs use `if: github.event_name == 'workflow_dispatch'` (leftover from a possible prior PR trigger; currently always true when the workflow runs).

**Env vars on test steps:** none related to goldens. No `ASGREP_UPDATE_GOLDENS`, `UPDATE_GOLDENS`, `INSTA_UPDATE`, or explicit `CI=true` on the Rust test job. Pi/npm workflows set build/publish envs only (`RUSTC_WRAPPER`, `ASGREP_NPM_*`).

### 1.3 Local / script gates (not GHA)

| Entry | What runs | Golden role |
|-------|-----------|-------------|
| [`scripts/local-release-gate.sh`](../../../scripts/local-release-gate.sh) | `fmt --check`, clippy `-D warnings`, **`cargo test --workspace --locked`**, 30s rank fuzz | Same compare-only fail as CI when used |
| [`CONTRIBUTING.md`](../../../CONTRIBUTING.md) default bar | forbid-soundness, `cargo check`, focused parity, CLI smoke | Does **not** require machine_contracts or full workspace |
| [`docs/validation/proof-pack.md`](../../../docs/validation/proof-pack.md) | Targeted oracle + `machine_contracts` + mcp protocol | Closest “merge bar” doc for contract freezes |
| `package.json` `test:pi-e2e` / `test:pi-release-gate` | Pi release scripts | Structural, not dump goldens |
| `scripts/check-bench-output.py` | JSON identity + avg latency max | Used by speed/bakeoff CI |

### 1.4 What runs golden-like comparisons today

Per PASS1–3, true file-backed freezes exercised by `cargo test` include:

- `crates/ast-sgrep-cli/tests/machine_contracts.rs` ↔ fixtures under `crates/ast-sgrep-cli/tests/fixtures/`
- Ranking / graph oracles + `tests/fixtures/ranking/cases.json`
- Dense hand asserts (plugins, mcp, extraction tuples) -- fail in CI the same way

There is **still no** `assert_golden` / update write path (PASS2 P0 unimplemented). CI cannot auto-rewrite goldens until that lands -- then hygiene becomes mandatory.

### 1.5 Already good / related hygiene

| Item | Status |
|------|--------|
| `.gitignore` `*.actual` | **Yes** (PASS2) under `# Tests` |
| Bench result upload on speed/bakeoff | **Yes** -- good pattern to mirror for `*.actual` |
| Baselines honesty (Agents.md + RELEASING) | **Yes** -- separate from test goldens |
| Auto-update in CI | **No path exists** (good) |

---

## 2. Gap analysis vs skill `CI-GOLDENS.md` checklist

Skill iron rule: **CI = compare only; local = update + human review + commit.**

| Skill expectation | Repo today | Gap severity |
|-------------------|------------|--------------|
| CI runs tests in strict compare mode | `cargo test --workspace --release` when **manually** dispatched | **Medium** -- no PR automation; goldens only checked if someone runs the workflow |
| Explicit forbid update env (`INSTA_UPDATE=no` / empty `UPDATE_*`) | Not set; no consumer of `ASGREP_UPDATE_GOLDENS` yet | **Low now / High once P0 lands** |
| Post-test scan for `*.snap.new` / `*.actual` and fail | Absent | **Medium** after assert_golden |
| Upload golden diffs (`*.actual`, `.snap.new`) on **failure** | Absent on `ci.yml` | **High** for remote triage once dumps exist |
| PR comment listing changed golden files | Absent | **Low** (nice-to-have; manual `git diff` ok for small set) |
| Developer workflow documented (update → diff → commit) | PASS2 audit only; not CONTRIBUTING / Agents | **High** for agents + humans |
| Orphan golden weekly cleanup job | Absent | **Low** (few freezes today) |
| Vitest/Jest snapshot `CI=true` strict mode | N/A (no snapshot runner for product goldens) | N/A |
| Never auto-update in CI | De facto OK | Guard still missing |

### 2.1 Extra gaps specific to this repo

1. **CONTRIBUTING drift:** claims GHA runs forbid-soundness + cargo check **on every `pull_request`**. [`ci.yml`](../../../.github/workflows/ci.yml) is **`workflow_dispatch` only**. Reviewers may believe goldens/tests are PR-gated when they are not.
2. **No PR template** under `.github/` -- no checkboxes for fixture/golden review or baselines honesty.
3. **No Makefile golden target** -- agents must invent commands; SOP should be the single source of truth.
4. **Windows matrix** does not re-run full `machine_contracts` (lib + smoke only) -- cross-platform golden risk deferred to PASS2 P3 / PASS5 canonicalize.
5. **Honesty ledgers vs goldens:** speed/bakeoff enforce **latency thresholds** and upload JSON; they must never be confused with blessing `benchmarks/results/baselines.md` quality figures (Agents.md).

### 2.2 Risk of auto-update in CI

| Scenario | Risk today | Risk after `assert_golden` |
|----------|------------|----------------------------|
| Default job env rewrites goldens | **None** (no write API) | **High** if `ASGREP_UPDATE_GOLDENS` set in workflow `env:` or org secrets by mistake |
| `cargo test` with ambient developer env leaked | N/A on GHA clean runners | Still clean runners; risk is **explicit bad workflow edit** |
| Commit step after tests | No git write in `build-and-test` | Keep it that way; never `git add` goldens in CI |
| Pi release packing rewriting contract | Scripts verify/pack; contract is source-controlled | Out of golden-update scope; keep pack read-only w.r.t. contract |

**Mitigation (proposed, not implemented):** pin on `build-and-test`:

```yaml
env:
  ASGREP_UPDATE_GOLDENS: "0"   # compare-only; never rewrite freezes in CI
```

plus failure upload of `**/*.actual` and optional `find` fail if any `.actual` remains after a **green** run (pollution check).

---

## 3. Proposed workflow YAML snippet (audit only -- do not land yet)

Prefer attaching golden hygiene to the **existing** `build-and-test` job rather than a parallel full matrix (minutes budget is already the reason for manual dispatch). When PR triggers are re-enabled (if ever), the same block applies.

```yaml
# Proposed addition to .github/workflows/ci.yml — build-and-test job
# DO NOT apply in this pass; proposal for a future bead.

  build-and-test:
    # ... existing strategy / checkout / toolchain / cache ...
    env:
      # Iron rule: CI never rewrites golden files (PASS2 env name).
      ASGREP_UPDATE_GOLDENS: "0"
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2

      - name: Build (release)
        run: cargo build --workspace --release

      - name: Test (release, golden compare-only)
        run: cargo test --workspace --release

      - name: Fail if golden mismatch dumps left behind
        if: success()
        shell: bash
        run: |
          # Once assert_golden writes *.actual on mismatch, a green suite
          # must not leave dumps (would mean silent update or partial run).
          mapfile -t actuals < <(find . -name '*.actual' -not -path './target/*' 2>/dev/null || true)
          if ((${#actuals[@]})); then
            echo "::error::Found unexpected *.actual files after a green test run:"
            printf '  %s\n' "${actuals[@]}"
            exit 1
          fi

      - name: Upload golden mismatch dumps
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: golden-actuals-${{ matrix.os }}-${{ github.run_id }}
          path: |
            **/*.actual
          if-no-files-found: ignore
          retention-days: 7
```

**Optional later (when freezes grow):** lightweight path-only job on `pull_request` that only runs `cargo test -p ast-sgrep-cli --test machine_contracts` + ranking oracle -- not full workspace -- if minutes become available. Out of scope to re-enable PR CI without product owner sign-off (current design is intentional cost control).

**Do not** add `ASGREP_UPDATE_GOLDENS=1` anywhere under `.github/`.

---

## 4. Developer / agent golden update SOP

**Update env name (PASS2 decision, non-negotiable):** `ASGREP_UPDATE_GOLDENS`  
Truthy: `1`, `true`, `yes`, `on` (same family as other `ASGREP_*` flags).  
**Reject:** bare `UPDATE_GOLDENS`, `AST_SGREP_UPDATE_GOLDENS`, `INSTA_UPDATE=always`.

Until `assert_golden` exists, “update” for file freezes is still **manual edit or regenerate + copy** into fixture paths; the SOP below is written for the **target** workflow and remains valid for hand edits today.

### 4.1 When to update a golden

Update only when product behavior **intentionally** changed and the new output is correct. Do **not** re-bless to silence a flake (scrub instead -- PASS5). Do **not** update `benchmarks/results/baselines.md` via this SOP (honesty ledger -- §5).

### 4.2 Steps (local)

```bash
# 1. Reproduce failure (compare mode -- env unset)
cargo test -p ast-sgrep-cli --test machine_contracts -j1 -- --test-threads=1
# or, once assert_golden lands:
# cargo test -p <crate> --test <name> -- <filter>

# 2. Inspect failure
# - assert_eq! Debug dump today
# - future: *.actual next to golden + unified diff + hint in panic

# 3. If intentional, regenerate (future assert_golden path)
ASGREP_UPDATE_GOLDENS=1 cargo test -p <crate> --test <name> -- <filter>

# Today (no helper): edit or overwrite the fixture under
#   crates/<crate>/tests/fixtures/…
#   tests/fixtures/ranking/cases.json
# then re-run step 1 without the env.

# 4. Review EVERY changed freeze file
git diff -- crates/*/tests/fixtures tests/fixtures
# Ask: is each delta explained by the product change?
# Reject path/temp/version noise -- scrub instead of freezes that encode them.

# 5. Stage explicit paths only (never git add .)
git add crates/ast-sgrep-cli/tests/fixtures/<file>.json   # example
git commit -m "test: update machine envelope golden for <reason>"

# 6. Confirm compare mode still green without update env
unset ASGREP_UPDATE_GOLDENS
cargo test -p <crate> --test <name> -j1 -- --test-threads=1
```

### 4.3 Agent rules (bead loops / multi-agent)

| Rule | Detail |
|------|--------|
| Never set update env in CI YAML | Local shells only |
| Never commit `*.actual` | Gitignored; use as review aid then delete |
| One behavior change → one golden commit message reason | No bulk re-bless without per-file review |
| Prefer focused package/test filters | Full workspace only for release gate |
| If mismatch is non-deterministic | Do not update; file scrub/canonicalize work (PASS5) |
| Baselines metrics | Separate process (§5); never `ASGREP_UPDATE_GOLDENS` on benches |

### 4.4 Reviewer checklist (PR)

- [ ] Diff of golden/fixture files is **intentional** and matches the code change description  
- [ ] No temp paths, absolute home dirs, timestamps, or host-specific noise introduced  
- [ ] Version fields remain scrubbed or pinned per existing contract (`<version>` pattern)  
- [ ] No `ASGREP_UPDATE_GOLDENS` documented as required for normal `cargo test`  
- [ ] If numbers in README/PR body: provenance row in `baselines.md` or explicit `UNREPRODUCIBLE`  
- [ ] Extraction “goldens” that are Rust tuples: review as code, not as dump files  

---

## 5. Interaction with Agents.md honesty rules (baselines)

| Concern | Policy |
|---------|--------|
| Is `benchmarks/results/baselines.md` a CI golden? | **No.** Historical / honesty ledger; many rows `UNREPRODUCIBLE`. |
| May CI rewrite baselines? | **No.** |
| May agents restate MRR/Recall/nDCG/latency from memory? | **No** -- only cite a baselines row or tag `UNREPRODUCIBLE` ([Agents.md](../../../Agents.md) “Benchmark and published-number claims”). |
| Speed / bakeoff workflows | Gate **identity + latency threshold** on harness JSON; upload results. That is a **correctness/perf gate**, not a blessing of quality tables. Do not copy threshold numbers into README without baselines provenance. |
| Negative ledger | Failures stay documented under `benchmarks/results/` / validation docs; do not “pass” honesty by deleting misses ([docs/RELEASING.md](../../../docs/RELEASING.md)). |
| Overlap with golden SOP | Updating a machine JSON fixture ≠ updating a published metric. Keep commit messages and PR checklists separate. |

---

## 6. Aggregated findings for beads (max 4)

Deep items only; implement later. **Do not file beads in this pass.**

### B1 — P1: CI golden hygiene on `build-and-test` (upload + no-update guard)

| | |
|--|--|
| **Problem** | Manual CI runs full `cargo test` with no `ASGREP_UPDATE_GOLDENS=0` pin, no `*.actual` upload on failure, no post-success pollution check. When PASS2 assert_golden lands, misconfiguration can rewrite freezes or bury mismatch dumps on the runner. |
| **Why** | Skill CI iron rule; remote triage; agent loops need downloadable `.actual`. |
| **Acceptance** | `ci.yml` `build-and-test`: env `ASGREP_UPDATE_GOLDENS=0`; on `failure()`, `actions/upload-artifact` for `**/*.actual` (ignore if none); optional success-time `find` fail if `.actual` present. **No** update mode. Document in workflow comment. |
| **Depends** | Soft-depends on assert_golden for dumps to exist; guard env can land first. |

### B2 — P1: Written golden update SOP in tree (CONTRIBUTING or `docs/validation/`)

| | |
|--|--|
| **Problem** | Env name and process live only in audit markdown; CONTRIBUTING has no golden section; agents invent re-bless procedures. |
| **Why** | Without SOP, bulk fixture commits skip review; wrong env names proliferate. |
| **Acceptance** | Short section (can mirror §4 of this pass): env `ASGREP_UPDATE_GOLDENS`, compare vs update, `git diff` review, no CI update, link PASS5 scrub policy in one line. Fix CONTRIBUTING line that claims PR-triggered soundness/check **or** restore `pull_request` triggers deliberately (product decision). |

### B3 — P2: PR template checkboxes for freezes + honesty

| | |
|--|--|
| **Problem** | No `.github/pull_request_template.md` (or `PULL_REQUEST_TEMPLATE`). Reviewers lack a prompt for fixture diffs and baselines claims. |
| **Why** | Skill PR review workflow; Agents.md bare-quote ban needs a human gate on PR bodies. |
| **Acceptance** | Minimal template with checkboxes: behavior tests updated; golden/fixture files reviewed file-by-file; no committed `*.actual`; any metric claims cite `baselines.md` or `UNREPRODUCIBLE`; no `ASGREP_UPDATE_GOLDENS` in CI. |

### B4 — P2: Optional PR-path contract slice (minutes-aware)

| | |
|--|--|
| **Problem** | Zero automatic PR signal for machine contracts / ranking oracle; freezes can merge broken until someone dispatches full CI. |
| **Why** | Highest-value freezes are cheap tests (`machine_contracts`, `ranking_oracle`) relative to full workspace release matrix. |
| **Acceptance** | Product decision recorded: either (a) keep all-manual and rely on proof-pack in PR description, or (b) add a **small** `pull_request` workflow running forbid-soundness + `machine_contracts` + `ranking_oracle` only, with same no-update + artifact rules as B1. Do not silently re-enable full `build-and-test` on every PR without cost sign-off. |

**Explicitly not bead-worthy here:** orphan weekly cleanup; PR auto-comment of golden paths; adopting insta; treating baselines as goldens; implementing assert_golden (PASS2 P0).

---

## 7. Cross-pass linkage

| Pass | Input to this audit |
|------|---------------------|
| **PASS1** | Inventory: few true freezes; CI golden upload missing |
| **PASS2** | `ASGREP_UPDATE_GOLDENS`; P2 CI hygiene; `*.actual` gitignore landed |
| **PASS3–4** | Which fixtures matter if CI only runs a slice (machine + ranking first) |
| **PASS5** | Do not re-bless flakes; scrub; never update goldens in CI for nondeterminism |

---

## 8. Evidence / method

- Read skill `references/CI-GOLDENS.md` and SKILL.md CI-related sections  
- Catalogued all `.github/workflows/*.yml` (triggers, jobs, `cargo test`, upload-artifact, env)  
- Checked `.gitignore` for `*.actual`; absence of PR template / Makefile / justfile  
- Compared `CONTRIBUTING.md` PR claims to `ci.yml` triggers  
- Cross-read PASS2 § CI / env naming and Agents.md honesty block  
- No CI YAML modified; no beads; no commit  

---

## 9. Bottom line

**Compare-only is safe today only because update mode does not exist.** CI does not upload golden dumps, does not pin the future update env, does not run on PRs, and developer SOP is audit-only. Ship B1+B2 before or with `assert_golden`; use B3 for review discipline; treat B4 as a product cost decision. Keep baselines honesty orthogonal to golden file workflows.

# Pass 4/16 — Keep-gate / Ratchet / Bench-history Audit

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Date:** 2026-08-07  
**Skill:** `running-the-gauntlet-on-your-rust-port` → KEEP-GATE-RULES + pattern 155  
**Priors:** `PASS1_PROJECT_CLASS_REFERENCES.md`, `PASS2_THREE_PILLAR_GAPS.md`, `PASS3_EVIDENCE_HONESTY.md`  
**Mode:** audit-only · read/rg only · **no** cargo test/bench/workspace · **no** beads · **no** commit · **no** invented latency/quality numbers  

---

## 0. Executive summary

Product has a **thin optional mean-latency tripwire** (`BENCH_RATCHET_PCT = 50`, env-gated) plus **absolute ms CI ceilings** (`--max-average-ms`). That is **not** the skill keep-gate (committed multi-scenario `.bench-history/<bench>.latest.json`, primary −3% / geomean −5% / …, cv_pct>5 ineligible, same-window dual gates, host fingerprint on keep decisions).

| Layer | Present? | Skill match? |
|-------|:--------:|:------------:|
| `cv_pct` **reported** on single-query + suite | Yes | Partial (report only; no >5% hard reject for keep) |
| Pass-over-pass **history file** | Yes (single JSON map) | No (gitignored, not multi-bench dir, not full report v3) |
| Ratchet **fail-on-regression** | Optional `ASGREP_BENCH_RATCHET=1` | No (50% only; default off) |
| Absolute latency gate | Yes (`check-bench-output.py`) | Opposite of skill pass-over-pass primary |
| Host fingerprint on keep | Partial (`check-error-budget.py` only) | Not product default; CI absolute ms ignore host |
| Weighted primary / geomean / p90 / throughput ratio gates | **Absent** | Required by skill |
| Same-window focused+broad | **Absent** | Required (K-4) |

**Verdict:** Skill keep-gate **absent**. Product = coarse regression tripwire + absolute ceilings. Aligns with PASS3 **T2** / PASS2 residual **#1**.

---

## 1. Current keep-gate mechanics

### 1.1 Product bench CLI (`crates/ast-sgrep-cli/src/bench.rs`)

| Constant / control | Value / behavior |
|--------------------|------------------|
| `BENCH_HISTORY_PATH` | `".bench-history.json"` (cwd-relative) |
| `BENCH_RATCHET_PCT` | **`50.0`** (hard-coded; not env-overridable) |
| `ASGREP_BENCH_HISTORY` | Default **on**; set `0` to disable write |
| `ASGREP_BENCH_HISTORY_PATH` | Override history path |
| `ASGREP_BENCH_RATCHET` | Must be exactly `"1"` to **fail** process on regression; otherwise history is advisory only |

**History write path** (`update_bench_history`):

1. Load or create root `{"schema_version": "1", "entries": {}}`.
2. Read prior `entries[label].avg_search_ms` if any.
3. Overwrite entry: `{ avg_search_ms, cv_pct, updated_unix_ms }`.
4. Persist pretty JSON to path.
5. Return meta: `path`, `label`, `avg_search_ms`, `cv_pct`, `prior_avg_search_ms`, `ratchet_pct`, optional `regression_pct`, `ratchet_ok`.

**Regression formula** (latency-up = worse):

```text
regression_pct = ((avg_ms - prior) / prior) * 100   # prior > 0
ratchet_ok     = regression_pct <= 50.0
```

- No prior → `ratchet_ok = true` (first run always passes).
- Fail only if `bench_ratchet_enabled()` **and** `ratchet_ok == false`.
- Labels:
  - single query: `query:{query}`
  - suite: `suite:{fixture_name}:{selected}` (one aggregate mean of case avgs; **not** per-case history)

**Paths that use history/ratchet:** `run_bench`, `run_bench_suite`.  
**Path that does not:** `run_bench_batch` (queries file) — no `cv_pct`, no history, no ratchet.

### 1.2 History file shape (observed)

Local (gitignored) sample at `crates/ast-sgrep-cli/.bench-history.json`:

```json
{
  "entries": {
    "query:process_request": {
      "avg_search_ms": 1.835667,
      "cv_pct": 0.0,
      "updated_unix_ms": 1786088275677
    }
  },
  "schema_version": "1"
}
```

- **One key**, no host, no git SHA, no `target/` mtime, no primary/geomean/p90, no `previous_ratchet`, no `ratchet_decision`, no full JSON v3 report.
- Schema version string is product `"1"`, not skill `fsqlite-e2e.comprehensive-bench-report.v3`.

### 1.3 Absolute CI / script gates

| Artifact | Mechanism | Threshold style |
|----------|-----------|-----------------|
| `scripts/check-bench-output.py` | Suite JSON: every case `ok` + `identity_ok` + `avg_search_ms ≤ --max-average-ms` | **Absolute ms** (required CLI arg) |
| `.github/workflows/speed.yml` | Manual `workflow_dispatch`; sample suite → `--max-average-ms **15**` | Absolute; **no** history/ratchet env |
| `.github/workflows/bakeoff.yml` | Manual; self suite → `--max-average-ms **100**` | Absolute; no history/ratchet |
| `scripts/check-error-budget.py` | Hyperfine `times` → p95, burn rate, optional same-host drift | Hard p95 + SLO; drift only if fingerprints match |
| `scripts/run-benchmarks.sh` | Invokes error-budget helper for cold-index style rows | Documented in `benchmarks/README.md` |
| `docs/benchmarks.md` | Documents history env + **50%** optional ratchet as "intentionally coarse" | Policy: tripwire ≠ microbench SLA |

**Default drift envelope** in error-budget: `max_drift_fraction = 0.10` (10% p95), **only when** both fingerprints present and equal. Missing/different fingerprint → drift **not evaluated** (does not fail on variance).

### 1.4 `.gitignore` for bench-history

```gitignore
# Local bench history ratchet (written by `asgrep bench`)
.bench-history.json
**/.bench-history.json
# ...
# CLI benchmark-history state (regenerated on runs)
**/.bench-history.json
```

Skill pattern 155 pitfall: **“.bench-history/ in .gitignore kills the whole pattern.”** Product deliberately ignores the product history file. There is **no** committed `.bench-history/` directory of `*.latest.json` files.

### 1.5 Tests (contract only)

`crates/ast-sgrep-cli/tests/machine_contracts.rs`:

- Asserts JSON emits `cv_pct` and writes history when `ASGREP_BENCH_HISTORY_PATH` set.
- Suite test disables history (`ASGREP_BENCH_HISTORY=0`).
- **No** test that enables `ASGREP_BENCH_RATCHET=1` and asserts fail at −3% or +50%.

### 1.6 Related: `release-perf` profile

`Cargo.toml` defines `[profile.release-perf]` with opt-level=3, thin LTO, codegen-units=1, line-tables-only, strip=false — **skill-shaped profile exists**. CI speed/bakeoff workflows use **`--release`**, not `release-perf` (skill rule 3 gap for “kept” claims).

---

## 2. Diff vs skill keep-gate

Skill sources: `SKILL.md` Keep-Gate table; `references/methodology/KEEP-GATE-RULES.md` (rules 1–10); `references/patterns/155-BENCH-HISTORY-RATCHET.md`.

### 2.1 Numeric thresholds

| Metric (skill) | Skill gate | Product today |
|----------------|------------|---------------|
| Primary score regression | **−3%** | **N/A** (no primary score) |
| Geomean regression | **−5%** | **N/A** |
| Per-category geomean | **−10%** | **N/A** |
| p90 regression | **−15%** | Error-budget p95 is **absolute threshold**, not pass-over-pass −15% |
| Throughput / ratio drop | **−5%** pass-over-pass | **N/A** |
| Mean search latency | (not the primary skill gate) | Optional fail if **>+50%** vs prior mean |

Order-of-magnitude: skill primary band is ~3%; product mean band is **50%** and **opt-in**.

### 2.2 History contract

| Skill (pattern 155) | Product |
|---------------------|---------|
| `.bench-history/<bench>.latest.json` **committed** | Single `.bench-history.json` **gitignored** |
| Full report JSON v3 + `previous_ratchet` + `ratchet_decision` | 3-field entry map only |
| Multi-bench (broad + focused files) | At most one aggregate label per run type; local sample has **one** key |
| Auto-stage / CI greps gitignore for `.bench-history` | Opposite: gitignore **requires** ignore |
| Environment block for host class | None in product history |

### 2.3 Keep eligibility rules (skill ALL-of)

| # | Skill rule | Product status |
|---|------------|----------------|
| 1 | Profile-first ≥0.1% self-time before source touch | Campaign dumps under `tests/artifacts/perf/*`; **not** enforced as merge gate |
| 2 | Focused + broad same run window (git, `target/`, host, ~same minute) | **Absent** |
| 3 | Measure under `release-perf` | Profile exists; **CI uses `--release`** |
| 4 | Mode-default proof file | **Absent** (no concurrent_mode-style keep proof) |
| 5–7 | Symmetric retry / identical config / selections byte-identical | N/A for pure latency suites; identity is hit-list on suite cases only |
| 8 | `cv_pct` reported; **>5% ineligible for keep** | Reported; **no gate** on >5 |
| 9 | MT8 / profile frame attribution for wins | Not wired to bench history |
| 10 | Pass-over-pass thresholds (−3/−5/−10/−15/−5) | **50% mean only**, optional |

### 2.4 CI philosophy conflict

Skill: **file-committed pass-over-pass** is the gate; absolute host ms is a lie vector.  
Product CI (manual workflows): **absolute `max-average-ms`** is the release-shaped latency check. PASS3 already labeled this **V3**.

---

## 3. cv_pct / noise handling

### 3.1 Computation (present)

```text
cv_pct = (sample_stdev / mean) * 100
```

- Sample stdev: Bessel (n−1).
- `< 2` samples or `mean == 0` → `0.0` (silent; can mask single-iteration “clean” CV).

Emitted on:

- Single-query JSON / human line.
- Per suite case + suite-level field (suite `cv_pct` = **mean of case CVs**, not CV of suite mean).

### 3.2 Skill noise rule

- `cv_pct > 5` → result is **noise**, not eligible for keep (KEEP-GATE-RULES rule 8).
- “Within noise” ≈ ±3–5% band for rejections.

### 3.3 Product gaps

| Behavior | Present? |
|----------|:--------:|
| Report `cv_pct` | Yes |
| Store `cv_pct` in history entry | Yes |
| Fail keep / ratchet when `cv_pct > 5` | **No** |
| Fail CI when noisy | **No** |
| MAD / median robust detector (pattern 170) | **No** |
| Quarantine path for cv flakes | **No** |
| Batch bench CV | **No** |

Contract tests only assert `cv_pct` **is some f64**, not a bound.

**Conclusion:** Noise is **observable**, not **operational**. A high-CV run can still update history and, with ratchet off (default), never fail.

---

## 4. Host fingerprint / same-window requirements

### 4.1 Skill requirement

Same run window = **same git state, same `target/`, same machine, same minute** (timestamps within ~60s for dual gates). Host class mismatch invalidates comparison (pattern 155 pitfall: “beefier machine” baselines).

### 4.2 Product reality

| Mechanism | Host / window awareness |
|-----------|-------------------------|
| `update_bench_history` | **None** — no hostname, CPU, rustc, git SHA, or binary hash |
| `ASGREP_BENCH_RATCHET` | Compares means across whatever host last wrote the gitignored file |
| `check-bench-output.py` | Absolute ms only; **no** fingerprint args |
| `speed.yml` / `bakeoff.yml` | `ubuntu-latest` vs local M-series hosts → gate can flip without product change (PASS3 V10 / flip table) |
| `check-error-budget.py` | **Has** `fingerprint` / `prior_fingerprint`; drift only if equal; **optional** and not wired as default keep for `asgrep bench` |
| Perf campaign `20260702` | Had `fingerprint.json` (PASS3); `20260806` did **not** — inconsistent discipline |

### 4.3 Same-window dual gate

- No pairing of focused + broad JSON commits.
- Suite history collapses to one mean label; single-query history is independent.
- No enforcement that suite + query (or release-perf + CI) share SHA/`target`/host/minute.

**Conclusion:** Host coupling is **optional tooling** on hyperfine budgets only; product keep-adjacent paths are **host-agnostic** and therefore skill-dishonest as keep gates.

---

## 5. Aggregated findings for beads (max 3 deep; fold with PASS3 T2)

**Do not file beads in this pass.** Themes for Pass 11 aggregation only. Prefer **one epic** that absorbs PASS3 **T2** rather than three parallel keep-gate beads.

### F1 — Skill-grade keep-gate program (fold = PASS3 **T2** + PASS2 residual **#1**)

**Title class:** *Keep-gate that refuses to lie*

**Scope (single epic, multi-step):**

1. **Committed multi-scenario history** under a team SSoT (skill-shaped `.bench-history/<bench>.latest.json` **or** greenfield-adapted contract still ≪ 50% and **not** gitignored).
2. **Thresholds:** primary/geomean-style or explicit greenfield table still near skill (−3%/−5% class), not 50% mean-only; document any class adaptation.
3. **Default-on CI** comparison against committed prior (not optional env; not absolute-ms-as-sole-release-gate).
4. **Host fingerprint** required on keep decisions (extend `check-error-budget` idea into bench history meta).
5. **cv_pct > 5** → ineligible / quarantine (not silent history update).
6. Ban competitor latency as correctness (already honesty policy; encode in gate docs).

**Evidence anchors:** `bench.rs` `BENCH_RATCHET_PCT=50`; `.gitignore` history; `check-bench-output.py`; workflows speed/bakeoff; PASS3 V3/V4/V10; skill KEEP-GATE-RULES + pattern 155.

**Out of scope for this epic:** micro hotspot dumps as history; quality MRR regen (T1); negative ledgers (T3); cold-index budget rebaseline (T4).

### F2 — Absolute-ms CI honesty split (sub-task of F1 or tiny sibling)

Keep identity/`ok` suite gates; **demote** bare `--max-average-ms` from “the” release keep to: (a) smoke ceiling with host class labeled, **and/or** (b) secondary to pass-over-pass. Use `release-perf` for any claim labeled keep.  
**Fold into F1** if only one bead is allowed.

### F3 — History shape + batch path parity (implementation slice under F1)

- Per-case history keys for suite (not only suite mean).
- Batch path: emit `cv_pct` + history + same ratchet rules.
- Meta fields: `git_sha`, host fingerprint, `profile` (`release` vs `release-perf`), iterations.
- Contract tests for ratchet fail/pass at configured threshold and cv>5 reject.

**Do not** open separate beads for: conformal `parity_score` ratchet (conformance pillar), BOCPD soak, MT8-specific until primary keep exists.

### Mapping table

| This pass | PASS3 | PASS2 | Skill |
|-----------|-------|-------|-------|
| F1 | **T2** | residual #1 | KEEP-GATE-RULES + 155 |
| F2 | V3, V10 | CI absolute | K-2 honesty |
| F3 | V4 thin history | thin `.bench-history.json` | report + ratchet meta |

**If only one bead ships:** **F1** (= PASS3 T2). F2/F3 are implementation checklist items, not separate epics.

---

## 6. Evidence log (what this pass actually did)

Read / searched (no cargo, no numbers claimed as live SLAs):

- `crates/ast-sgrep-cli/src/bench.rs` (full keep/history/ratchet/cv paths)
- `docs/benchmarks.md` (ratchet section)
- `.gitignore` (bench-history ignore)
- `scripts/check-bench-output.py`, `scripts/check-error-budget.py`
- `.github/workflows/speed.yml`, `bakeoff.yml`
- `crates/ast-sgrep-cli/.bench-history.json` (local shape sample only)
- `crates/ast-sgrep-cli/tests/machine_contracts.rs` (cv/history contracts)
- `Cargo.toml` `[profile.release-perf]`
- `benchmarks/README.md` (executable gates + error-budget docs)
- PASS1/PASS2/PASS3 keep-gate excerpts
- Skill: `SKILL.md` Keep-Gate table; `KEEP-GATE-RULES.md`; pattern `155-BENCH-HISTORY-RATCHET.md`

**Did not:** run cargo bench/test, invent latency/quality figures, create beads, commit.

---

## 7. Verdict block

| Item | Value |
|------|--------|
| **Artifact** | `/Users/aditya/Developer/ast-sgrep/tests/artifacts/gauntlet-audit/PASS4_KEEPGATE_RATCHET.md` |
| **Product ratchet** | Optional +50% mean vs thin gitignored history |
| **Skill ratchet** | −3% primary / −5% geomean / … on committed multi-bench latest |
| **cv_pct** | Reported; not gated at 5% |
| **Host / same-window** | Not on product keep path; partial error-budget only |
| **Beads this pass** | none (F1≡T2 for Pass 11) |
| **Skill keep-gate present?** | **No** |

**DONE** — Pass 4 complete; audit-only; no cargo; no beads; no commit; no invented numbers.

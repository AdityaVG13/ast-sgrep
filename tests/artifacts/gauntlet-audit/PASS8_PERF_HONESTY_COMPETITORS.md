# Pass 8/16 — Perf Honesty vs Competitors

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Date:** 2026-08-07  
**Skill:** `running-the-gauntlet-on-your-rust-port` (Performance pillar / competitor honesty)  
**Mode:** audit-only · **no** cargo · **no** beads · **no** commit · **no** invented latency/quality numbers  

**Priors (required):**  
[`PASS1_PROJECT_CLASS_REFERENCES.md`](./PASS1_PROJECT_CLASS_REFERENCES.md) ·  
[`PASS2_THREE_PILLAR_GAPS.md`](./PASS2_THREE_PILLAR_GAPS.md) ·  
[`PASS3_EVIDENCE_HONESTY.md`](./PASS3_EVIDENCE_HONESTY.md) ·  
[`PASS4_KEEPGATE_RATCHET.md`](./PASS4_KEEPGATE_RATCHET.md) ·  
[`PASS5_NEGATIVE_LEDGERS.md`](./PASS5_NEGATIVE_LEDGERS.md) ·  
[`PASS6_FEATURE_UNIVERSE_DRAFT.md`](./PASS6_FEATURE_UNIVERSE_DRAFT.md) ·  
[`PASS7_ORACLE_DIFFERENTIAL.md`](./PASS7_ORACLE_DIFFERENTIAL.md)  

**Cross-link only (do not re-own):**  
`ast-sgrep-golden-artifacts-program-nz7i` ·  
`ast-sgrep-conformance-harness-program-ghiw` ·  
`ast-sgrep-fuzz-program-maturity-b8q3` · mock-free `lbx1` · in-tree `tests/artifacts/perf/*` campaigns  

**Honesty rule:** No new measurements. Competitor and quality figures cited only as already labeled in-tree (`UNREPRODUCIBLE` / historical / ledger). Agents.md: quote only via [`benchmarks/results/baselines.md`](../../../benchmarks/results/baselines.md) or explicit unreproducible tag.

---

## 0. Executive summary

| Lens | Verdict |
|------|---------|
| **rg / ast-grep timing role** | **Latency competitors only** -- never keep-gate correctness oracles |
| **Match-set Pattern-1 vs rg/ast-grep** | **None in CI** (`jell-deferral.md`; structural first slice owned by **ghiw.3**, not this pass) |
| **Skill keep-gate** | **Absent** (PASS4): optional +50% mean on gitignored thin history; absolute `max-average-ms` CI |
| **baselines dual-status** | **Confirmed defect** (PASS3 V1): file-level UNREPRODUCIBLE banners coexist with subset "reproducible from this tree" rows and missing named harnesses |
| **Pillar (a) maturity** | **4 / 10** (PASS2; reaffirmed -- plumbing real, gate not skill-grade) |
| **Beads this pass** | **None**. Max **3** residual themes for Pass 10/11 |

**One-line:** Competitor speed ledgers are useful **history**, dangerous when misread as **correctness** or as a **pass-over-pass keep-gate**. Product has absolute ceilings and a coarse optional ratchet; skill keep-gate (committed multi-scenario history, −3%/−5% class, host fingerprint, cv>5 ineligible) is not installed.

---

## 1. Perf pillar honesty vs competitors

### 1.1 Correct roles (class-aware)

From PASS1 product positioning and PASS7 channel matrix:

| Competitor | Legitimate gauntlet role | Illegitimate role |
|------------|--------------------------|-------------------|
| **ripgrep** (`rg`) | Lexical **latency** head-to-head; optional future **file:line subset** match-set after DISC (ghiw later-phases; may stay deferred) | Bit-identical FTS/result-set identity; sole release keep-gate; proof that hybrid "beats rg" as correctness |
| **ast-grep** / `sg` | Structural **latency** on supported native pattern subset; **ghiw.3** owns first Pattern-1 match-set | Full surface parity / rewrites / codemods; "parity clean" as match-set proof when only wall-clock was measured |
| **semgrep** | Historical **quality** bake-off only | Correctness gate; live CI oracle |
| **hyperfine** | Latency **driver** for scripts | Correctness comparator |

**Rule (skill + product):** latency ledger (`speed.md`, hyperfine, `speedup_vs_*`) is **NEVER** a correctness oracle (PASS7 §1). Comparator for keep decisions must be **self** (pass-over-pass) under fixed host fingerprint, not "faster than rg on this host."

### 1.2 What exists today (evidence map, no new numbers)

| Artifact | What it is | Correctness? | Keep-gate? |
|----------|------------|:------------:|:----------:|
| `scripts/run-benchmarks.sh` | Warm literal vs `rg`, structural vs `ast-grep` via hyperfine | No | No (latency) |
| `benchmarks/results/speed.md` | Self + competitor wall-clock history; 2026-08-05 self rows **partial in-tree** via run-benchmarks; older head-to-head / Semgrep / scale tables **UNREPRODUCIBLE** | No | No |
| `benchmarks/results/head-to-head.md` | Aggregated win/loss classes; **"parity clean"** structural language is **latency** (PASS3 V6) | **Misread risk** | No |
| `benchmarks/results/bakeoff.md` / `losses.md` | Quality narrative vs hand-patterns / semgrep | Quality honesty only | No |
| `benchmarks/results/baselines.md` | Canonical quality fingerprints + cold/NL/watch tables -- all quality fingerprints **UNREPRODUCIBLE** | Quality SSoT quotes only | No |
| `asgrep bench` + `.bench-history.json` | Self mean latency + optional 50% ratchet | No | **Thin tripwire only** (PASS4) |
| `scripts/check-bench-output.py --max-average-ms` | Absolute suite ceiling | Identity fields only | Absolute, host-flip prone |
| `scripts/check-error-budget.py` | Hyperfine p95 + optional same-host drift | No | Partial honesty plumbing |
| `.github/workflows/speed.yml` / `bakeoff.yml` | Manual absolute ms gates; `--release` not `release-perf` | No | Absolute CI |
| `tests/artifacts/perf/*` | Profile campaigns (hotspots, budgets, fingerprints inconsistent across dumps) | No | Campaign history |

### 1.3 Honesty defects that matter for competitors

| ID | Defect | Source | Why hostile readers get lied to |
|----|--------|--------|----------------------------------|
| C1 | **"parity clean"** on structural speed rows without match-set dump | PASS3 V6; head-to-head.md | Sounds like Pattern-1; is wall-clock |
| C2 | Competitor rows from **missing harnesses** still look quotable | PASS3 V2: `run-speed-headtohead.sh`, `speed-report.py`, scale corpus absent | Cannot regen; still present as tables |
| C3 | Host coupling (M5 Max provenance) + CI absolute ms on ubuntu-latest | PASS3 V10 / PASS4 §4 | Gate flips without product change |
| C4 | Warm-indexed self vs cold-scan competitor asymmetry | speed.md / product honesty (good when stated; bad when omitted in summaries) | "Wins" are not apples-to-apples scan |
| C5 | Quality fingerprints cited near speed story without UNREPRODUCIBLE discipline | PASS3 V8 README bare quality numbers | Confuses ranking gold with latency gate |
| C6 | Skill keep-gate **absent** while competitor ledgers are rich | PASS2 pillar a; PASS4 | Rich latency narrative ≠ certified perf pillar |

### 1.4 What is *not* a competitor honesty bug

- Publishing **rg wins** on small-corpus lexical cases and named quality **losses** (`losses.md`).
- Explicit **jell-deferral** of full external hit-ID identity.
- Documenting that production structural search does **not** require spawning ast-grep.
- Using `rg` / `ast-grep` versions as **pins** in provenance blocks without claiming live differential CI.

---

## 2. Keep-gate summary (from PASS4)

Full audit: [`PASS4_KEEPGATE_RATCHET.md`](./PASS4_KEEPGATE_RATCHET.md). Condensed for synthesis:

### 2.1 Product vs skill

| Layer | Product today | Skill keep-gate |
|-------|---------------|-----------------|
| History | Single gitignored `.bench-history.json` (often one key) | Committed `.bench-history/<bench>.latest.json` multi-scenario v3 |
| Fail threshold | Optional `ASGREP_BENCH_RATCHET=1` and **+50%** mean | Primary **−3%**, geomean **−5%**, category −10%, p90 −15%, throughput −5% |
| Noise | `cv_pct` **reported**; **not** gated at >5 | `cv_pct > 5` → ineligible for keep |
| Host / same-window | None on product keep path; partial in error-budget only | Same git, `target/`, host, ~minute; dual focused+broad |
| CI | Absolute `--max-average-ms` (speed 15 / bakeoff 100) | Pass-over-pass vs committed prior |
| Profile | `release-perf` exists; CI uses `--release` | Keep under `release-perf` |
| Weighted primary score | **Absent** | Required for comprehensive matrix |

**Verdict (PASS4):** Skill keep-gate **absent**. Product = coarse optional tripwire + absolute ceilings.

### 2.2 Fold mapping (already named)

| PASS4 finding | PASS3 | PASS2 |
|---------------|-------|-------|
| F1 skill-grade keep-gate program | **T2** | residual **#1** |
| F2 demote absolute-ms as sole release keep | V3, V10 | CI absolute |
| F3 history shape / batch path / meta fields | V4 thin history | thin history |

**If only one epic ships for perf gates:** **F1 ≡ T2** (F2/F3 are checklist slices, not separate epics).

### 2.3 Competitor interaction with keep-gate

Keep-gate must **ban** competitor latency as a correctness or keep justification (PASS4 F1 item 6; skill One Rule). Allowed:

1. Self pass-over-pass (primary).  
2. Optional side-channel **latency report** vs rg/ast-grep with host pin + explicit "not a keep."  
3. External match-set only under Pattern-1 suites with XFAIL/DISC (**ghiw.3** for structural; lexical ⊆ rg deferred).

---

## 3. Baselines dual-status (from PASS3)

Full inventory: [`PASS3_EVIDENCE_HONESTY.md`](./PASS3_EVIDENCE_HONESTY.md). Condensed:

### 3.1 Dual-status pattern

Every major results file opens with a file-level **"numeric rows are unreproducible / no harnesses"** banner, while **subsections** claim regen or "reproducible from this tree":

| File | Banner class | Conflicting subset |
|------|--------------|--------------------|
| `baselines.md` | UNREPRODUCIBLE quality + missing gold/eval harness | Still **canonical** fingerprint SSoT for quotes; cold/NL/watch also historical |
| `speed.md` | File UNREPRODUCIBLE | 2026-08-05 self corpus via **`scripts/run-benchmarks.sh` (exists)** |
| `head-to-head.md` | Historical dump not in tree | 2026-08-05 self rows partial; structural "parity clean" |
| `bakeoff.md` / `losses.md` | UNREPRODUCIBLE | Named losses still useful as **ledger** |

**Missing scripts still named in reproduce blocks:** `eval-bakeoff.py`, `watch-bench.py`, `speed-report.py`, `run-speed-headtohead.sh`, `corpora.lock`, raw results JSON (PASS3 §1.1).

### 3.2 Dual-status of fingerprint rows (quality -- affects perf honesty narrative)

| Fingerprint / row class | Status | Use |
|-------------------------|--------|-----|
| `self-hybrid-d3eab74`, `rg-hybrid-default-d3eab74`, `rg-neural-rerank-d3eab74` | **canonical** + **UNREPRODUCIBLE** | Sole quote authority; never "live green" |
| `self-hist-pre-29129bd` ~0.75 | **SUPERSEDED** | Do not re-canonize |
| 2026-08-05 self warm/cold speed | **reproducible-in-tree** (host-coupled) | Best live-ish speed path |
| 23k/100k / Semgrep suite / 100k cold-overhead | **historical** + **UNREPRODUCIBLE** | Narrative only |
| 110-file cold-index **BUDGETS** vs 1,107-file reality | **stale budget** (breach noted in speed.md) | Dual-canonical budget risk (PASS3 V5 / T4) |

### 3.3 Policy kernel that already works

Agents.md four rules (no bare quotes; harness path for "reproducible"; negative ledger don't delete misses; one fingerprint per metric×corpus×config) are **necessary** and **already bind** docs. They do **not** install skill keep-gates or split dual banners per table section (PASS3 T1 still residual).

---

## 4. Residual themes (max 3 deep; Pass 10/11 only)

**Do not file beads in this pass.** Prefer folding into Pass 10 packages rather than three parallel micro-epics.

### P8-1 — Keep-gate that refuses to lie (fold PASS4 F1 ≡ PASS3 T2 ≡ PASS2 #1)

| | |
|--|--|
| **Priority** | P0–P1 for pillar (a) certification |
| **Problem** | Optional +50% gitignored mean + absolute ms ≠ skill pass-over-pass keep; competitor timing can be misread as the gate |
| **Acceptance sketch** | Committed multi-scenario history (not gitignored SSoT); thresholds near skill (−3%/−5% class or documented greenfield adaptation still ≪ 50%); host fingerprint on keep decisions; `cv_pct > 5` ineligible; CI compares to committed prior; docs ban competitor latency as correctness/keep; `release-perf` for keep-labeled claims |
| **Out of scope** | Micro hotspot dumps as history; MRR gold regen; implementing ghiw.3 match-set |
| **Cross-links** | PASS4 F1–F3; PASS2 residual #1; PASS7 "keep-gate ≠ oracle" |

### P8-2 — Published speed/quality ledger provenance closure (fold PASS3 T1 + competitor dual-status)

| | |
|--|--|
| **Priority** | P1 honesty |
| **Problem** | Dual banners + missing harness names + "parity clean" latency language let agents quote unreproducible competitor rows as live |
| **Acceptance sketch** | Per-section status tags: `canonical \| historical \| UNREPRODUCIBLE \| reproducible-in-tree`; fix or delete reproduce blocks naming absent scripts; structural "parity clean" → "latency only / no match-set" wording; quality fingerprints remain UNREPRODUCIBLE until gold+harness returns **or** permanently locked historical |
| **Out of scope** | Restoring full 18-gold MRR eval in the same bead if oversized (may be phase-2 of same epic); nz7i dump freezes |
| **Cross-links** | PASS3 V1–V2, V6, V8–V9; PASS2 residual #5; Agents.md |

### P8-3 — Budget / corpus pin honesty + HotPath on keep (fold PASS3 T4 + PASS2 #7/#10)

| | |
|--|--|
| **Priority** | P2 (honesty + attribution) |
| **Problem** | 110-file BUDGETS still ship beside breached 1,107-file reality; keep decisions lack required profile/HotPath cards |
| **Acceptance sketch** | Rebaseline or archive 110-file budgets; every budget row carries corpus file-count + git SHA; when a claim cites stage timers, commit a sample profile artifact; HotPath/attribution required for "win" keep claims (not for every micro commit) |
| **Out of scope** | Full scale-corpus product work; BOCPD multi-day soak (full gauntlet, not audit residual alone) |
| **Cross-links** | PASS3 V5/T4; PASS2 residuals #7, #10; `tests/artifacts/perf/*` |

**If only one residual ships from this pass:** **P8-1**. Competitor honesty **P8-2** is the second load-bearing honesty epic and may ship first if the team prioritizes doc integrity over ratchet engineering.

---

## 5. Explicit non-ownership (do not refile)

| Theme | Owner |
|-------|--------|
| pattern: × ast-grep match-set | **ghiw.3** |
| DISC / COVERAGE / proof-pack runner | **ghiw.1 / ghiw.5** |
| Dump goldens / assert_golden | **nz7i** |
| Fuzz continuous | **b8q3** |
| Embed-on / mock-free process e2e | **lbx1** |
| Full multi-engine hit-ID | **jell-deferral.md** (non-goal) |
| Progress negative ledgers install | PASS5 B1–B3 → Pass 10 package |
| FeatureUniverse TOML | PASS6 S1 → Pass 10 package |
| Composite oracle dispatch | PASS7 R1 → Pass 10 package |

---

## 6. Evidence log

- Read PASS1–PASS7 under `tests/artifacts/gauntlet-audit/` (full residual and competitor sections).  
- Relied on PASS3 catalog for dual-status and harness presence; PASS4 for keep-gate mechanics; PASS2 for pillar scores; PASS7 for "latency ≠ correctness."  
- Skill CERTIFICATION keep-adjacent rules via PASS4 citations (KEEP-GATE-RULES / pattern 155).  
- **Did not:** cargo test/build/bench; hyperfine regen; invent numbers; file beads; commit.

---

## 7. Verdict block

| Item | Value |
|------|--------|
| **Artifact** | `/Users/aditya/Developer/ast-sgrep/tests/artifacts/gauntlet-audit/PASS8_PERF_HONESTY_COMPETITORS.md` |
| **rg/ast-grep as correctness** | **No** (timing only; match-set absent) |
| **Skill keep-gate** | **Absent** (PASS4 summary) |
| **baselines dual-status** | **Confirmed** (PASS3) |
| **Residual themes** | **3** (P8-1 keep-gate · P8-2 ledger provenance · P8-3 budget/HotPath) |
| **Beads / cargo / commit** | **none** |

**DONE** — Pass 8 complete; audit-only; no invented numbers; cross-links only to nz7i/ghiw/b8q3/lbx1.

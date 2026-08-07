# FINAL GAUNTLET REPORT — Audit Draft (Pass 13/16 style)

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Date:** 2026-08-07  
**Skill:** `running-the-gauntlet-on-your-rust-port`  
**Campaign mode:** **audit-only** (condensed PASS1–11 under `tests/artifacts/gauntlet-audit/`)  
**Hard constraints honored this campaign:** no product implementation in audit passes; no cargo full workspace test/build/bench for scoring; no invented performance/quality numbers; **no certification issued**.

**Sources (authoritative for this draft):**

| Pass | Artifact |
|------|----------|
| 1 | [`PASS1_PROJECT_CLASS_REFERENCES.md`](./PASS1_PROJECT_CLASS_REFERENCES.md) |
| 2 | [`PASS2_THREE_PILLAR_GAPS.md`](./PASS2_THREE_PILLAR_GAPS.md) |
| 3 | [`PASS3_EVIDENCE_HONESTY.md`](./PASS3_EVIDENCE_HONESTY.md) |
| 4 | [`PASS4_KEEPGATE_RATCHET.md`](./PASS4_KEEPGATE_RATCHET.md) |
| 5 | [`PASS5_NEGATIVE_LEDGERS.md`](./PASS5_NEGATIVE_LEDGERS.md) |
| 6 | [`PASS6_FEATURE_UNIVERSE_DRAFT.md`](./PASS6_FEATURE_UNIVERSE_DRAFT.md) |
| 7 | [`PASS7_ORACLE_DIFFERENTIAL.md`](./PASS7_ORACLE_DIFFERENTIAL.md) |
| 8 | [`PASS8_PERF_HONESTY_COMPETITORS.md`](./PASS8_PERF_HONESTY_COMPETITORS.md) |
| 9 | [`PASS9_CERTIFICATION_READINESS.md`](./PASS9_CERTIFICATION_READINESS.md) |
| 10 | [`PASS10_REMEDIATION_SYNTHESIS.md`](./PASS10_REMEDIATION_SYNTHESIS.md) |
| 11 | [`PASS11_BEADS_FILED.md`](./PASS11_BEADS_FILED.md) |

---

## Explicit non-certification

| Claim | Status |
|-------|--------|
| **`strict-conformant-release.v1`** | **Not claimed** (PASS9: all four skill constants **RED**) |
| **Evidence bundle classes green** | **0 / 8** (PASS9) |
| **`release_certificate.json` / signed bundle** | **Not produced** |
| **`RELEASE_CERTIFICATION` / "gauntlet certified"** | **Not issued** |
| **This document** | **Audit report + remediation map only** — not a certificate |

Any future certification requires a real certifying run after remediation (WP1–WP6 + cross-program inputs + skill constants hold). See PASS9 §4.

---

## 1. Executive summary

### 1.1 Product and campaign

ast-sgrep is a **greenfield multi-reference hybrid** code-search product (lexical + AST/graph + semantic + agent surfaces), **not** a single-upstream 1:1 port. Oracles are **composite** (spec / fixture / peer / math / metamorphic / optional external subset / latency-only), never one canonical competitor identity (PASS1).

This campaign ran **audit-only** recon and synthesis (PASS1–11). Remediation work packages were designed (PASS10) and filed as beads (PASS11). **No** full skill ≥10-round implementation convergence loop was executed.

### 1.2 Maturity per pillar (PASS2; reaffirmed PASS8/PASS9)

| Pillar | Maturity (1–10) | One-line status |
|--------|:---------------:|-----------------|
| **(a) Performance** | **4 / 10** | Real bench plumbing (Criterion, `asgrep bench`, release-perf, profile artifacts, absolute CI ceilings) but **no** skill-grade pass-over-pass keep-gate; published competitor/quality ledgers largely **UNREPRODUCIBLE** |
| **(b) Conformance** | **5 / 10** | Strong **internal** oracles (ranking/graph soft fixtures, metamorphic, IVF CE-003, HitKey peer parity, proof-pack culture); almost no external Pattern-1 match-set CI; **no** conformal lower-bound / composite dispatch SSoT file |
| **(c) Surface parity** | **3 / 10** | Thin product tables (CLI/MCP/LSP/Pi + short feature IDs); **no** machine `present\|partial\|missing\|excluded` matrix, weights, or `parity_score` |

**Forbidden-victory:** none of the three pillars may be declared done alone. Combined skill **CERTIFIED** readiness: **blocked** (PASS2 §7; PASS9).

**Oracle / differential readiness (PASS7):** ~**4 / 10** skill-grade (strong internals; weak external + cert stack).

**Cert readiness (PASS9):** four required-pass constants all **RED**; multi-ref hybrid scorecard rows red/yellow only; **zero** skill-cert green rows invented.

### 1.3 Headline verdict

| Field | Value |
|-------|--------|
| Mode | `audit-only` |
| Tier | **T3 — Workspace** |
| Class | **Greenfield-Rust-class** + multi-reference external-tool oracles (**hybrid multi-oracle**) |
| Workspace version (PASS1 pin) | `1.4.0` |
| Remediation epic | `ast-sgrep-gauntlet-remediation-program-1vhy` |
| Work packages filed | **6** (`.1`–`.6`) |
| Certification | **None issued** |

---

## 2. Mode / class / tier (PASS1)

### 2.1 Mode: `audit-only`

| Criterion | This campaign |
|-----------|---------------|
| Intent | Report + pin inventory + gap synthesis + bead filing; **not** product remediation in-loop |
| Skill router | Existing multi-crate product; want report + plan, not code changes in audit passes |
| Explicit non-modes | Not `gauntlet-full` (≥10 remediation rounds); not workspace bootstrap; not single-pillar harden-only |

### 2.2 Tier: **T3 — Workspace**

| Signal | Evidence (PASS1) |
|--------|------------------|
| LOC band | ~37k Rust LOC across `crates/` |
| Multi-crate | 11 workspace members (core, cli, lang, embed, lsp, mcp, plugins, testkit, mmap, codemode, codemode-napi); `fuzz/` excluded from member list |
| Multi-surface | CLI + MCP + LSP + Pi + Code Mode |
| Multi-oracle domains | Lexical / structural / graph / semantic / machine JSON / index durability |

### 2.3 Project class

| Class | Match? |
|-------|:------:|
| SQL / RESP / Numerical-Python / ML-System / HTTP-Protocol ports | **No** (not primary product claims) |
| **Greenfield-Rust-class** with multi external-tool oracles | **Yes** |
| Single-port FrankenSQLite-class 1:1 differential | **No** |

**Skill mapping label:** `greenfield multi-reference hybrid`.

**Subject / Oracle / Comparator (summary):** Subject = workspace `@ HEAD`; Oracle = scenario-dispatched composite; Comparator = HitKey / soft ranking / recall@k SLO / sidecar identity / latency ledger (**not** correctness) / scrubbed machine JSON — see PASS1 §2.3.

### 2.4 Reference pins (inventory only; host 2026-08-07)

Host tools observed on PATH in PASS1 (versions are pin inventory, **not** certified benchmarks):

| Tool | Version noted | Role |
|------|---------------|------|
| ripgrep | 15.1.0 | Lexical **latency** competitor only |
| ast-grep / sg | 0.45.0 | Structural **latency** / future subset match-set |
| semgrep | 1.172.0 | Historical quality bake-off only |
| hyperfine | 1.20.0 | Latency driver |
| tree-sitter (crate) | 0.26.10 (`Cargo.lock`) | Parse substrate |

Full pin tables: PASS1 §3.

---

## 3. Findings table

Severity: **blocker** = cert / honesty wall; **major** = pillar maturity or release-gate gap; **minor** = secondary honesty / attribution; **owned-elsewhere** = tracked outside gauntlet epic (do not refile).

| ID | Severity | Pillar(s) | Finding | Residual WP | Owned elsewhere |
|----|----------|-----------|---------|-------------|-----------------|
| F-KEEP | **blocker** | Perf | Skill-grade keep-gate **absent**: optional ~**+50%** mean ratchet on thin/gitignored history; CI absolute `max-average-ms`; not committed multi-scenario pass-over-pass (−3%/−5% class) | **WP1** `.1` | — |
| F-DUAL | **blocker** | Perf + honesty | Dual-status published ledgers: file-level UNREPRODUCIBLE banners + subset "reproducible" rows + reproduce blocks naming **missing** scripts | **WP2** `.2` | — |
| F-BUDGET | major | Perf | Stale 110-file cold-index BUDGETS vs larger self-corpus reality (speed.md breach note); host-coupled absolute gates | **WP2** (checklist) / WP1 | Historical `tests/artifacts/perf/*` campaigns |
| F-HOTPATH | minor→major on win claims | Perf | HotPath / profile cards not required on keep "wins"; attribution incomplete | **WP1** (checklist) | — |
| F-DISPATCH | **blocker** (conf foundation) | Conf | No single composite oracle dispatch SSoT (channel × mode × comparator × gate_class) | **WP4** `.4` | ghiw owns harness shell, not this router |
| F-EXTP1 | major | Conf | No external match-set Pattern-1 in CI; full multi-engine hit-ID deferred (`jell-deferral.md`) | Encode excluded / Form-2 | **ghiw.3** first structural slice; jell deferred |
| F-CONFORMAL | major | Conf + Surface | No `parity_score` / Beta lower bound / committed `ratchet_state.json` | **WP6** `.6` | ghiw.5 = point suite report **input**, not the score |
| F-MATRIX | major | Surface | FeatureUniverse formal machine matrix absent (PASS6 **draft only**: 94 rows inventoried as 61 present / 17 partial / 8 missing / 7 excluded / 1 n/a — **draft structural counts, not certified coverage %**) | **WP5** `.5` | nz7i freezes dumps; ghiw.2 MUST matrix |
| F-NEGLED | major | All three | `docs/progress/` **absent**; no skill three ledgers + `retry_condition_predicate`; product `negative-ledgers.md` is fail-closed cases (naming collision) | **WP3** `.3` | Product fail-closed table stays product-owned |
| F-CERT | **blocker** | All | All four CERTIFICATION constants RED; 0/8 bundle classes; no multi-ref checklist + forbidden-victory tracker in product tree | **WP6** `.6` | Consumes ghiw.5, nz7i, b8q3 floors |
| F-PARITYCLEAN | major (misread risk) | Conf honesty | "parity clean" language is **latency/history**, not match-set correctness | **WP2** wording | — |
| F-COMPROLE | major (if misused) | Perf honesty | rg / ast-grep must remain **latency** competitors, not keep/correctness oracles | **WP1** docs + **WP4** `latency_only` | ghiw.3 match-set only when shipped |
| F-GOLDEN | owned-elsewhere | Surface | CLI/MCP/Pi/codemode/lang dump freezes, scrubbers, assert_golden | Cross-link only | **nz7i** (+.1–.5) |
| F-HARNESS | owned-elsewhere | Conf | DISC/COVERAGE, QG/machine MUST, compliance report runner | Cross-link only | **ghiw** (+.1–.5) |
| F-FUZZ | owned-elsewhere | Conf floor | Fuzz targets/seeds/sanitizers continuous floor | Cert input only | **b8q3** |
| F-EMBED | owned-elsewhere | Surface | Embed-on / mock-free e2e / soft-skip honesty | Cross-link for embed-on rows | **lbx1** |

**PASS3 violations of "gates that refuse to lie" (summary):** dual banners (V1), missing harness names in reproduce blocks (V2), absolute-ms host flip risk (V3/V10), optional 50% ratchet (V4), stale budgets (V5), "parity clean" misread (V6), negative-ledger naming (V7), README bare quality risk (V8) -- details in PASS3; closed via WP1–WP3.

---

## 4. Remediation plan (6 WPs + bead IDs)

**Epic:** [`ast-sgrep-gauntlet-remediation-program-1vhy`](./PASS11_BEADS_FILED.md)  
**Labels:** `gauntlet`, `audit-pass-11`, `three-pillar`, `honesty`  
**Source design:** [`PASS10_REMEDIATION_SYNTHESIS.md`](./PASS10_REMEDIATION_SYNTHESIS.md)

| WP | Bead ID | Title | P | Depends (blocks) |
|----|---------|-------|:-:|------------------|
| **WP1** | `ast-sgrep-gauntlet-remediation-program-1vhy.1` | Keep-gate that refuses to lie (skill-grade bench history / thresholds / host / cv) | P1 | — |
| **WP2** | `ast-sgrep-gauntlet-remediation-program-1vhy.2` | Published ledger provenance and budget honesty | P1 | — |
| **WP3** | `ast-sgrep-gauntlet-remediation-program-1vhy.3` | Negative-ledger discipline (`docs/progress/*` + Agents mandate) | P1 | — |
| **WP4** | `ast-sgrep-gauntlet-remediation-program-1vhy.4` | Composite oracle dispatch SSoT | P1 | soft: ghiw.1 DISC IDs |
| **WP5** | `ast-sgrep-gauntlet-remediation-program-1vhy.5` | FeatureUniverse formal matrix + cross-host surface honesty | P2 | **blocks on** `.3` (surface-deferrals ledger) |
| **WP6** | `ast-sgrep-gauntlet-remediation-program-1vhy.6` | Greenfield conformal score + multi-ref certification readiness | P2 | **blocks on** `.1`, `.4`, `.5` |

### 4.1 Package goals (one line each)

1. **WP1** -- Committed multi-scenario keep history; thresholds ≪ 50%; host fingerprint; `cv_pct>5` ineligible; competitor latency ≠ keep/correctness.  
2. **WP2** -- Per-section ledger status tags; fix/delete false reproduce blocks; lock or restore quality fingerprints; rebaseline budgets; no dual-banner lies.  
3. **WP3** -- Create three skill progress ledgers + retry predicates; bridge product fail-closed naming; extend Agents.md three-ledger rule.  
4. **WP4** -- Single dispatch file: channel → authoritative mode → Subject/Oracle → comparator → gate_class (`latency_only` / `never_correctness`).  
5. **WP5** -- Promote PASS6 draft to machine matrix; encode intentional exclusions (MCP no-fusion, jell, etc.); link surface-deferrals.  
6. **WP6** -- Multi-ref cert checklist (honest red/yellow); forbidden-victory same-window rule; lower-bound score path; **never** invent green cert.

### 4.2 Suggested implementation close-order (PASS10 §4.4 / PASS11 §5)

1. **WP3** + **WP2** (discipline + ledger honesty)  
2. **WP4** (dispatch SSoT)  
3. **WP5** (after WP3 for surface-deferrals)  
4. **WP1** (keep-gate)  
5. **WP6** last (score + scorecard once inputs exist)

### 4.3 Related programs (cross-link only; do not re-own)

| Program ID | Role |
|------------|------|
| `ast-sgrep-golden-artifacts-program-nz7i` | Golden dumps / freezes |
| `ast-sgrep-conformance-harness-program-ghiw` (+.1–.5) | DISC / MUST / pattern×ast-grep / compliance report |
| `ast-sgrep-fuzz-program-maturity-b8q3` | Fuzz floor for cert |
| `ast-sgrep-mock-free-e2e-gaps-lbx1` | Embed-on / process surface gaps |

Dependency edges and cycle check: PASS11 §2 (`br dep cycles` → none at filing).

---

## 5. Deferred list (with retry conditions)

These are **scope non-goals or deferred** for this campaign / product class. Encode as `excluded` / Form-2 (or skill retry forms) under WP3/WP5; do not pretend they are shipped.

| Deferred item | Why deferred | Retry / reopen condition (sketch) |
|---------------|--------------|-----------------------------------|
| **Full multi-engine hit-ID equality (`jell`)** | Authoritative non-goal (`docs/validation/jell-deferral.md`) | Product explicitly reopens jell; DISC + harness + gold fixtures exist; not required for multi-ref hybrid cert of **product promises** |
| **Full ast-grep / rg feature parity** | Product complements tools; does not replace them (`docs/comparison.md`) | Only if product positioning changes to claim full surface replacement |
| **Absolute hybrid rank freeze** | Soft ranking oracles by design | Hard absolute order only if ranking policy changes and fixtures re-authored |
| **Skill `strict-conformant-release.v1` / full cert** | Audit-only campaign; constants RED; machinery absent (PASS9) | WP1–WP6 land; nz7i/ghiw/b8q3 inputs green enough; real certifying run with four constants + 8 bundles; two consecutive ZERO-CHANGE on **implementation** loop (not audit markdown alone) |
| **External Pattern-1 match-set CI (broad)** | jell deferred; first structural slice owned by **ghiw.3** (not shipped as full CI match-set at audit time) | ghiw.3 acceptance + optional lexical⊆rg subset policy later |
| **UNREPRODUCIBLE quality fingerprints as live cert numbers** | Gold + eval harness absent for 18-gold / foreign bake-off rows | Restore gold+eval harness **or** permanently lock rows as historical with no dual "live" reading (WP2) |
| **BOCPD / multi-day soak on parity stream** | Full gauntlet / long soak; out of audit-only residual alone | After keep-gate + conformal score exist; optional later hardening |
| **tree-sitter CLI oracle dumps** | CLI not on PATH at PASS1 inventory | Install pin + define oracle role if needed |
| **MCP auto-fusion / full MCP↔CLI format parity** | Intentional product deltas (surface-parity) | Only if product removes intentional split; else permanent `excluded` + surface-deferrals entry |
| **Fake or provisional certificate from audit markdown** | Would invent green | Never; provisional-release.v1 only with explicit deviations list + real evidence (PASS9 §4.3) |

---

## 6. Convergence note

| Skill expectation | This campaign |
|-------------------|---------------|
| Full gauntlet: multi-round **implementation** remediation (≥10 rounds style), ZERO-CHANGE stops, open-hypothesis ledger closed | **Not run** |
| Condensed audit passes (recon, honesty, keep-gate, ledgers, matrix draft, oracle, cert readiness, synthesis, beads) | **PASS1–11 written** under `tests/artifacts/gauntlet-audit/` |
| Two consecutive ZERO-CHANGE on product code | N/A for audit-only (no remediation loop) |
| Beads | Filed once at PASS11 (epic + 6 children); not executed here |

**Honest label for this campaign:** **internal audit snapshot** (PASS9 §4.3) -- "Audit-only gauntlet PASS1–11 written; remediation packages filed; **no cert claim**."

Closing WP1–WP6 + parallel program work is **post-audit implementation**, not evidence that this report converges the skill full loop.

---

## 7. Honesty / runbook pointers (maintainers)

Folded here (no separate runbook file) to avoid bloat.

| Concern | Where to look / what to do |
|---------|----------------------------|
| Quote MRR / nDCG / latency | Only from `benchmarks/results/baselines.md` row or tag `UNREPRODUCIBLE` (`Agents.md`) |
| Competitor timing | Latency only; never keep-gate correctness (PASS8) |
| Keep claims | Require skill-shaped history after WP1; until then treat absolute ms as host-labeled smoke |
| Rejected work | After WP3: mine `docs/progress/*` before re-trying opts/budgets/evals |
| Oracle choice | After WP4: read dispatch SSoT; do not invent "the" oracle |
| Surface claims | After WP5: matrix statuses + exclusions; do not claim full rg/ast-grep parity |
| Certification | After WP6 + real run only; never mint `release_certificate.json` from markdown |
| Parallel programs | nz7i / ghiw / b8q3 / lbx1 -- implement there; gauntlet epic cross-links only |
| Negative evidence product cases | `docs/validation/negative-ledgers.md` = fail-closed ops; not campaign rejection ledger |
| jell | `docs/validation/jell-deferral.md` remains authoritative until product reopens |

---

## 8. Artifact index

```text
tests/artifacts/gauntlet-audit/
  PASS1_PROJECT_CLASS_REFERENCES.md
  PASS2_THREE_PILLAR_GAPS.md
  PASS3_EVIDENCE_HONESTY.md
  PASS4_KEEPGATE_RATCHET.md
  PASS5_NEGATIVE_LEDGERS.md
  PASS6_FEATURE_UNIVERSE_DRAFT.md
  PASS7_ORACLE_DIFFERENTIAL.md
  PASS8_PERF_HONESTY_COMPETITORS.md
  PASS9_CERTIFICATION_READINESS.md
  PASS10_REMEDIATION_SYNTHESIS.md
  PASS11_BEADS_FILED.md
  FINAL_GAUNTLET_REPORT.md          ← this file
```

---

## 9. Verdict block

| Item | Value |
|------|--------|
| **Report path** | `/Users/aditya/Developer/ast-sgrep/tests/artifacts/gauntlet-audit/FINAL_GAUNTLET_REPORT.md` |
| **Mode / tier / class** | audit-only · T3 Workspace · greenfield multi-reference hybrid |
| **Pillar maturity** | Perf **4/10** · Conf **5/10** · Surface **3/10** (PASS2) |
| **Cert** | **No RELEASE_CERTIFICATION issued**; strict-conformant-release.v1 **cannot** be claimed |
| **Remediation** | Epic `…-1vhy` + WP1–WP6 beads `.1`–`.6` |
| **Convergence** | Audit loop only -- **not** full ≥10 implementation rounds |
| **Invented metrics** | **None** |
| **Commit (this pass)** | **None** (orchestrator) |

**DONE** -- Final gauntlet audit report drafted from PASS1–11; no certification; no cargo; no beads mutation; no commit.

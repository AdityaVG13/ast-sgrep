# Pass 10/16 — Remediation Synthesis (program work packages for Pass 11)

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Date:** 2026-08-07  
**Skill:** `running-the-gauntlet-on-your-rust-port` -- synthesis for bead filing  
**Mode:** audit-only · **no** cargo · **no** beads filed this pass · **no** commit · **no** invented numbers  

**Priors:** PASS1–PASS9 under `tests/artifacts/gauntlet-audit/` (all residual themes merged here).  
**HARD:** Exactly **5–7 deep program work packages** -- no micro-items. Cross-link **nz7i / ghiw / b8q3 / lbx1**; do **not** re-own their work.

**This file is the key input to Pass 11** (`br create` epic + children). Pass 11 files beads; this pass only designs packages.

---

## 0. Executive summary

| Field | Value |
|-------|--------|
| **Class** | Greenfield multi-reference hybrid (T3); composite oracles |
| **Pillar readiness** | Perf 4/10 · Conf 5/10 · Surface 3/10 · Cert **blocked** |
| **Work packages** | **6** (deep programs) |
| **Owned elsewhere** | See §2 table -- do not refile |
| **Recommended epic shape** | One parent gauntlet remediation epic + 6 package children + deps (see §4) |
| **Beads this pass** | **None** |

**Package titles (report line):**

1. Keep-gate that refuses to lie  
2. Published ledger provenance and budget honesty  
3. Negative-ledger discipline (progress ledgers + Agents mandate)  
4. Composite oracle dispatch SSoT  
5. FeatureUniverse formal matrix and cross-host surface honesty  
6. Greenfield conformal score and multi-ref certification readiness  

---

## 1. Residual theme merge map (PASS2–9 → packages)

Every deep residual from prior passes folds into exactly one package. Micro-slices (F2, F3, B2, B3, P8-3 HotPath, C9-3 hooks) become acceptance checklist items inside the parent package -- **not** separate Pass 11 beads unless the implementer splits intentionally after epic create.

| Prior residual | Package |
|----------------|:-------:|
| PASS2 #1 keep-gate / bench history | **WP1** |
| PASS2 #7 HotPath attribution | **WP1** (checklist) |
| PASS2 #10 budget rebaseline | **WP2** |
| PASS2 #5 baselines provenance | **WP2** |
| PASS2 #4 negative ledgers + retry | **WP3** |
| PASS2 #2 composite oracle dispatch | **WP4** |
| PASS2 #3 FeatureUniverse + matrix | **WP5** |
| PASS2 #8 conformal / coverage score | **WP6** |
| PASS2 #9 certification scorecard | **WP6** |
| PASS2 #6 minimal external Pattern-1 | **Owned: ghiw.3** (not WP) |
| PASS3 T1 dual-banner / provenance closure | **WP2** |
| PASS3 T2 keep-gate refuses to lie | **WP1** |
| PASS3 T3 negative-evidence discipline | **WP3** |
| PASS3 T4 budget / corpus pin | **WP2** |
| PASS4 F1 / F2 / F3 | **WP1** |
| PASS5 B1 / B2 / B3 | **WP3** |
| PASS6 S1 formal SurfaceMatrix | **WP5** |
| PASS6 S2 parity_score pipeline | **WP6** (depends WP5) |
| PASS6 S3 cross-host agent honesty + surface-deferrals | **WP5** (+ ledger link WP3) |
| PASS7 R1 dispatch SSoT | **WP4** |
| PASS7 R2 conformal lower bound | **WP6** |
| PASS7 R3 multi-pillar CERT + tracker | **WP6** |
| PASS8 P8-1 keep-gate | **WP1** |
| PASS8 P8-2 competitor/ledger provenance | **WP2** |
| PASS8 P8-3 budget + HotPath | **WP2** / **WP1** |
| PASS9 C9-1 cert scorecard + forbidden-victory | **WP6** |
| PASS9 C9-2 conformal scoring | **WP6** |
| PASS9 C9-3 evidence plumbing consumers | **WP6** depends on WP1+WP5 (not a 7th package) |

---

## 2. Owned elsewhere -- do not refile

| Theme | Primary owner | Gauntlet action |
|-------|---------------|-----------------|
| assert_golden, scrubbers, CLI/MCP/Pi/codemode/lang **dump freezes** | **nz7i** (+.1–.5) | Cross-link only |
| DISC / COVERAGE seed, XFAIL conventions | **ghiw.1** | Cross-link; WP4 **references** DISC IDs when present |
| Query grammar + machine envelope MUST matrix | **ghiw.2** | Cross-link; WP5 cites contracts when written |
| **pattern: × ast-grep** match-set differential + DISC-pattern-native-subset | **ghiw.3** | **Do not** open a second Pattern-1 epic |
| Fixture PROVENANCE, IVF/migration RT corpora | **ghiw.4** | Cross-link |
| Compliance report emitter + proof-pack **runnable** gate | **ghiw.5** | WP6 **consumes** report as point-suite input -- does not re-spec runner |
| Fuzz targets, seeds, sanitizers, continuous floor | **b8q3** (+.1–.4) | Cross-link as cert floor only |
| Embed HTTP/neural mock-free e2e, soft-skip kill, process surfaces | **lbx1** | Cross-link for embed-on surface rows |
| Full multi-engine hit-ID equality (`jell`) | **jell-deferral.md** | Scope non-goal; encode as `excluded` in WP5 / Form-2 in WP3 |
| Full ast-grep / rg feature parity; absolute hybrid rank freeze | Product non-goals | Encode excluded / soft-oracle policy -- no "fix parity" beads |
| In-tree hotspot dumps under `tests/artifacts/perf/*` | Perf campaign history | Not a gauntlet epic; WP1 may require profile samples on keep claims only |
| Product fail-closed cases in `docs/validation/negative-ledgers.md` | Product tests | Keep file; WP3 bridges naming only |

---

## 3. Work packages (exactly 6)

### WP1 -- Keep-gate that refuses to lie

| Field | Content |
|-------|---------|
| **Title** | Keep-gate that refuses to lie |
| **Problem** | Product has optional **+50%** mean ratchet on a **gitignored** single-key `.bench-history.json` plus **absolute** `max-average-ms` CI. Skill keep-gate (committed multi-scenario history, primary/geomean-class thresholds, host fingerprint, `cv_pct>5` ineligible, same-window dual gates, `release-perf` for keep claims) is **absent**. Competitor latency can be misread as keep/correctness. |
| **Why** | Pillar (a) cannot leave 4/10 without a gate that refuses to lie (skill One Rule). Absolute host ms and 50% tripwires flip without product change (PASS3 V3/V4/V10; PASS4). |
| **Acceptance sketch** | (1) Committed multi-scenario history SSoT (skill-shaped `.bench-history/*.latest.json` **or** greenfield contract still ≪ 50% and **not** gitignored as the only truth). (2) Thresholds: −3%/−5% class or documented adaptation still order-of-magnitude tighter than 50%. (3) Default-on CI compare vs committed prior; demote bare `--max-average-ms` to smoke/host-labeled secondary. (4) Host fingerprint + git SHA + profile name on keep decisions. (5) `cv_pct > 5` → ineligible / quarantine. (6) Batch path emits cv + history + same rules; per-case suite keys. (7) Contract tests for fail/pass at threshold and cv reject. (8) Docs: competitor latency is **not** keep or correctness. (9) HotPath/profile sample required when claiming a **win** keep (not every micro commit). |
| **Priority** | **P0–P1** |
| **Depends** | None hard; soft: WP2 wording for competitor ledgers |
| **Cross-links** | PASS2 #1; PASS3 T2; PASS4 F1–F3; PASS8 P8-1; skill KEEP-GATE-RULES / pattern 155 |
| **Out of scope** | MRR gold regen; ghiw.3 match-set; BOCPD multi-day soak as day-1; re-owning perf artifact archaeology |

---

### WP2 -- Published ledger provenance and budget honesty

| Field | Content |
|-------|---------|
| **Title** | Published ledger provenance and budget honesty |
| **Problem** | `baselines.md` / `speed.md` / `head-to-head.md` / bakeoff/losses carry **dual-status** (file UNREPRODUCIBLE banners + subset "reproducible" rows + reproduce blocks naming **missing** scripts). Structural **"parity clean"** is latency language. 110-file cold-index BUDGETS still ship beside breached larger-corpus reality. README bare quality numbers risk looking live. |
| **Why** | Agents.md binds quotes to baselines, but dual banners and missing harness names create false regen confidence (PASS3 V1–V2, V5–V6, V8). Certification and competitor honesty both fail under hostile read (PASS8 P8-2). |
| **Acceptance sketch** | (1) Per-section (or per-table) status tags: `canonical \| historical \| UNREPRODUCIBLE \| reproducible-in-tree`. (2) Fix or delete reproduce blocks that name absent scripts (`eval-bakeoff.py`, `speed-report.py`, `watch-bench.py`, `run-speed-headtohead.sh`, etc.). (3) Quality fingerprints remain UNREPRODUCIBLE until gold+eval harness restored **or** permanently locked historical with no dual "live" reading. (4) Replace or annotate "parity clean" as latency-only / no match-set. (5) Rebaseline or archive 110-file BUDGETS; every budget row has corpus file-count + git SHA. (6) README quality snapshot: inline UNREPRODUCIBLE or clear canonical-row-only citation. (7) No invented new MRR/latency "wins." |
| **Priority** | **P1** (can ship in parallel with WP1; often first if doc integrity prioritized) |
| **Depends** | None |
| **Cross-links** | PASS3 T1/T4; PASS2 #5/#10; PASS8 P8-2/P8-3; Agents.md; `docs/RELEASING.md` |
| **Out of scope** | Building full 18-gold bake-off harness in the same PR if oversized (phase-2 of epic or follow-up child); nz7i freezes |

---

### WP3 -- Negative-ledger discipline (progress ledgers + Agents mandate)

| Field | Content |
|-------|---------|
| **Title** | Negative-ledger discipline (progress ledgers + Agents mandate) |
| **Problem** | `docs/progress/` **absent**. No `retry_condition_predicate` forms 1–8. Product `docs/validation/negative-ledgers.md` is **fail-closed cases**, not campaign rejection ledger (naming collision). Agents.md says "don't delete failures" but does not mandate three pillar ledgers + pre-flight mine. |
| **Why** | Skill K-3 / patterns 180–185: rejected opts, stale budgets, jell non-goals, withdrawn evals reappear as green without greppable predicates (PASS5). |
| **Acceptance sketch** | (1) Create `docs/progress/{perf-negative-results,conformance-negative-results,surface-deferrals}.md` + README index with skill headers and empty Closed/Open/Retired. (2) Entry template with hypothesis/workloads/measurement/outcome/retry_condition_predicate/bead/commit. (3) **Zero invented measurement closes** on first seed. (4) Pointer/Open imports: losses.md named outcomes; jell Form-2; dual-banner/process items; budget rebaseline Open; surface intentional deltas (PASS5 §3.4). (5) Header on product negative-ledgers.md bridging fail-closed vs campaign ledgers. (6) Extend Agents.md with three-ledger + blocker-if-unavailable rule; keep published-number rules 1–4. (7) Optional: failure-term list + mine-ledger notes (PASS5 B3) as same epic phase-2. |
| **Priority** | **P1** |
| **Depends** | None hard; imports content from WP2 themes without blocking create |
| **Cross-links** | PASS5 B1–B3; PASS3 T3; PASS2 #4; skill pattern 180/185 |
| **Out of scope** | Fabricating "we tried X and lost N%" without artifact paths; replacing product fail-closed table |

---

### WP4 -- Composite oracle dispatch SSoT

| Field | Content |
|-------|---------|
| **Title** | Composite oracle dispatch SSoT |
| **Problem** | Greenfield product uses many oracle modes (spec, fixture, peer, math, metamorphic, external subset, latency-only) but **no single dispatch file** maps channel × scenario → authoritative mode, Subject/Oracle IDs, comparator, DISC IDs, suite path, gate_class. Knowledge is scattered (proof-pack, jell-deferral, structural-patterns, per-crate tests). Pass 1 Q1 remains open. |
| **Why** | Without dispatch, agents invent "the" oracle, treat speed as correctness, or re-spec ghiw harnesses. Conformal weights (WP6) and honest multi-oracle claims need this router (PASS7 R1). |
| **Acceptance sketch** | (1) Land `docs/validation/oracle-dispatch.md` and/or `docs/contracts/oracle_dispatch.toml` covering lexical, graph, structural-native, semantic/ANN, hybrid/NL, machine JSON. (2) Each row: authoritative_mode, subject_id, oracle_id, comparator, disc_ids (link when ghiw.1 exists), suite_path, gate_class including `latency_only` and `never_correctness`. (3) jell / full rg identity listed deferred/excluded with pointer to jell-deferral. (4) Smoke/doc-lint: every proof-pack suite appears in table (or reverse). (5) Pass 1 Q1 answerable from one file. |
| **Priority** | **P1** (conformance foundation) |
| **Depends** | Soft: **ghiw.1** DISC/COVERAGE IDs for row links -- do not block writing the table with placeholder disc fields |
| **Cross-links** | PASS7 R1; PASS2 #2; PASS1 Q1/§2.3; conformance PASS4 design shell (do not duplicate ghiw harness) |
| **Out of scope** | Implementing pattern×ast-grep (**ghiw.3**); lexical⊆rg suite; inventing MRR; re-owning ranking_oracle existence |

---

### WP5 -- FeatureUniverse formal matrix and cross-host surface honesty

| Field | Content |
|-------|---------|
| **Title** | FeatureUniverse formal matrix and cross-host surface honesty |
| **Problem** | Surface pillar ~3/10: short `feature-universe.md` IDs and thin `surface-parity.md` without `present\|partial\|missing\|excluded` machine matrix, weights, or `docs/contracts/`. PASS6 drafted **94** rows (61 present / 17 partial / 8 missing / 7 excluded / 1 n/a) but skill stack (`supported_surface_matrix.toml`, coverage JSON, surface-deferrals) is absent. MCP graph tools / optional formats are real gaps; MCP no-fusion is intentional. |
| **Why** | Certification MIN_VERIFICATION_PCT and surface forbidden-victory cannot move without machine-checkable statuses against **product promises** (not full rg/ast-grep) (PASS6 S1/S3). |
| **Acceptance sketch** | (1) Promote PASS6 draft to `docs/contracts/supported_surface_matrix.toml` (statuses + rationale + evidence paths). (2) Optional first-pass category weights in `parity_score_contract.toml` (greenfield hybrid categories -- not SQL copy-paste). (3) Encode exclusions: MCP auto-fusion, jell, in-query boolean grammar, LSP doctor, etc. (4) Track or permanently exclude with predicates: MCP first-class defs/callers/chain tools, optional MCP format arg, Pi mode test matrix honesty -- **without** "fixing" intentional non-fusion. (5) Link surface-deferrals (WP3) for durable intentional deltas. (6) Cross-link **nz7i** for dump freezes; **lbx1** for embed-on; **ghiw.2** for machine MUST -- do not refile. |
| **Priority** | **P1** |
| **Depends** | Soft: WP3 for surface-deferrals file; scoring numbers live in WP6 |
| **Cross-links** | PASS6 S1/S3; PASS2 #3; PASS9 H5/H6 |
| **Out of scope** | nz7i extraction tree dumps; full MCP↔CLI format parity as a hard requirement if product keeps intentional split; inventing conformal green score from present-count |

---

### WP6 -- Greenfield conformal score and multi-ref certification readiness

| Field | Content |
|-------|---------|
| **Title** | Greenfield conformal score and multi-ref certification readiness |
| **Problem** | No `parity_score.json` / Beta conformal lower bound / committed `ratchet_state.json`. No multi-ref hybrid certification checklist, no forbidden-victory same-window tracker, no path to honest `strict-conformant-release.v1` (PASS9: all four constants RED, 0/8 bundle classes). Risk: shipping cargo-green or competitor "wins" as "certified." |
| **Why** | Skill release gates on **lower bound** and multi-evidence bundle; greenfield class still needs adapted cert, not silence (PASS7 R2/R3; PASS9 C9-1/C9-2). |
| **Acceptance sketch** | (1) Multi-ref checklist under `docs/validation/` (rows like PASS9 §3 H1–H14) -- all start red/yellow with evidence paths; **never invent green**. (2) Forbidden-victory rule greppable: no single pillar "done"/release if another pillar red in same evidence window; optional thin tracker later. (3) Define greenfield lower-bound score pipeline weighted by WP4 channels + WP5 features; emit `parity_score.json` (or project-named equivalent) with interval/lower bound + truncate policy; seed honest red/yellow. (4) Document that ghiw.5 compliance report, nz7i freeze status, b8q3 floor, WP1 keep-gate, WP2 unreproducible policy are **inputs** to cert -- not re-implemented here. (5) Explicit: no `release_certificate.json` until constants hold; provisional label only with deviations list if product policy needs a weaker ship tag. (6) Agents.md / releasing docs: never quote point estimate as certified without lower bound; never quote UNREPRODUCIBLE MRR as cert. |
| **Priority** | **P1–P2** (after or in parallel late with WP4/WP5; blocked for **green** numbers until WP1/WP4/WP5 exist) |
| **Depends** | Soft: WP4 for channel weights; WP5 for feature matrix; WP1 for bundle class 5; **ghiw.5** for suite point report; does **not** wait on full jell |
| **Cross-links** | PASS7 R2/R3; PASS6 S2; PASS9 C9-1/C9-2/C9-3; PASS2 #8/#9; CERTIFICATION.md |
| **Out of scope** | Fake certificates; re-spec ghiw/nz7i/b8q3; hard absolute hybrid ranking freeze; claiming strict-conformant-release from audit markdown |

---

## 4. Recommended epic shape for `br create` (Pass 11)

Pass 11 should **file** (this pass does not run `br`):

### 4.1 Parent epic

```text
Title:   ast-sgrep gauntlet remediation (multi-ref hybrid)
Type:    epic
Priority: P1
Labels:  gauntlet, audit-pass-11, three-pillar
Body:    Implements PASS10 packages WP1–WP6 from
         tests/artifacts/gauntlet-audit/PASS10_REMEDIATION_SYNTHESIS.md
         after audit-only PASS1–PASS9.
         Class: greenfield multi-reference hybrid (not FrankenSQL 1:1).
         Cross-link only: nz7i, ghiw(+.1–.5), b8q3, lbx1.
         Do not refile golden dumps, DISC shell, pattern×ast-grep,
         fuzz CI, or mock-free embed e2e.
         Forbidden victory: no pillar alone is "done."
         No invented performance/quality numbers; baselines.md SSoT.
```

Suggested epic id style (when filing):  
`ast-sgrep-gauntlet-remediation-program-<short>`  
(analogous to `…-program-nz7i` / `…-program-ghiw` / `…-program-maturity-b8q3`).

### 4.2 Children (one bead per WP)

| Child | Title | P | Blocks / deps |
|-------|-------|:-:|---------------|
| epic.1 | WP1 Keep-gate that refuses to lie | P1 | -- |
| epic.2 | WP2 Published ledger provenance and budget honesty | P1 | -- |
| epic.3 | WP3 Negative-ledger discipline | P1 | -- |
| epic.4 | WP4 Composite oracle dispatch SSoT | P1 | soft after ghiw.1 for DISC links |
| epic.5 | WP5 FeatureUniverse formal matrix + cross-host honesty | P1 | soft dep epic.3 for surface-deferrals |
| epic.6 | WP6 Conformal score + multi-ref cert readiness | P2 | soft dep epic.1, epic.4, epic.5; consumes ghiw.5 |

**Dependency edges (logical):**

```text
epic.2  ── parallel ──  epic.1
epic.3  ── parallel ──  epic.1 / epic.2
epic.4  ── parallel (soft ghiw.1)
epic.5  ── soft after epic.3; parallel epic.4
epic.6  ── after epic.4 + epic.5; soft after epic.1; cross-link ghiw.5/nz7i/b8q3
```

**Do not** create children for: F2, F3, B2, B3, S2-only, C9-3-only, HotPath-only -- those are checklist lines under WP1/WP3/WP6.

### 4.3 Pass 11 filing checklist (for the next agent)

1. Read this file + PASS8/PASS9 residual sections.  
2. `br create` parent epic with body quoting artifact path.  
3. `br create` six children; `br dep add` soft edges as above.  
4. On each child body: paste acceptance sketch + "out of scope / owned elsewhere" one-liners.  
5. `br dep cycles` must be empty.  
6. `br sync --flush-only` only if commit of `.beads/` is authorized (Pass 11 instructions).  
7. **Still no** cargo full workspace / invent numbers / fake cert.

### 4.4 Suggested close-order (implementation)

1. **WP3** (cheap discipline install) and **WP2** (doc honesty) can land first.  
2. **WP4** (dispatch SSoT) unblocks honest conf claims.  
3. **WP5** (matrix) unblocks surface + weights.  
4. **WP1** (keep-gate) unblocks perf pillar and cert class 5.  
5. **WP6** last for score + scorecard once inputs exist.

---

## 5. What success looks like (campaign, not this pass)

| Horizon | Signal |
|---------|--------|
| After WP2+WP3 | Dual banners resolved; progress ledgers greppable; no fake closed rejects |
| After WP4+WP5 | Q1 answerable; machine matrix; intentional exclusions encoded |
| After WP1 | Skill-shaped keep exists; absolute-ms not sole release keep |
| After WP6 | Honest red/yellow cert checklist + lower-bound path; still no fake green cert until constants hold |
| Full gauntlet cert | Only after programs nz7i/ghiw/b8q3 + WP1–6 + real certifying run -- see PASS9 §4 |

---

## 6. Evidence log

- Read PASS1–PASS9 under `tests/artifacts/gauntlet-audit/` residual tables and ownership maps.  
- Merged PASS2 ranked residuals, PASS3 T1–T4, PASS4 F1–F3, PASS5 B1–B3, PASS6 S1–S3, PASS7 R1–R3, PASS8 P8-1–P8-3, PASS9 C9-1–C9-3 into **6** packages.  
- Skill CERTIFICATION.md constants used only as readiness targets (PASS9).  
- **Did not:** cargo, `br create`, commit, invent numbers, emit certificates, re-own ghiw/nz7i/b8q3/lbx1.

---

## 7. Verdict block

| Item | Value |
|------|--------|
| **Artifact** | `/Users/aditya/Developer/ast-sgrep/tests/artifacts/gauntlet-audit/PASS10_REMEDIATION_SYNTHESIS.md` |
| **Package count** | **6** (within 5–7) |
| **Package titles** | WP1 Keep-gate · WP2 Ledger provenance · WP3 Negative ledgers · WP4 Oracle dispatch · WP5 FeatureUniverse matrix · WP6 Cert + conformal score |
| **Owned elsewhere table** | §2 |
| **Epic shape** | §4 parent + 6 children |
| **Beads filed this pass** | **none** (Pass 11) |
| **Cargo / commit** | **none** |

**DONE** -- Pass 10 synthesis complete; key input for Pass 11 bead filing; no beads; no commit; no fake certs.

# Pass 7/16 — Oracle & Differential Readiness (gaps beyond ghiw)

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Branch:** `perf/software-optimization` (no switch)  
**Date:** 2026-08-07  
**Skill:** `running-the-gauntlet-on-your-rust-port` (Conformance pillar / oracle wiring)  
**Mode:** audit-only · **no** cargo · **no** beads · **no** commit · **no** product code  

**Prior:** gauntlet PASS1–PASS6 under `tests/artifacts/gauntlet-audit/`; conformance PASS1–PASS7 under `tests/artifacts/conformance-audit/` (esp. PASS4 differential + PASS7_BEADS_FILED).  
**Cross-link only (do not re-spec):**  
`ast-sgrep-conformance-harness-program-ghiw` (+ `.1`–`.5`) ·  
`ast-sgrep-golden-artifacts-program-nz7i` ·  
`ast-sgrep-fuzz-program-maturity-b8q3` · mock-free `lbx1` where peer surfaces matter.

**Pass 1 carry-forward (this pass answers):** Q1 composite oracle completeness per channel; Q2 external differential honesty under jell-deferral; Q8 certification readiness (honest red/yellow).

---

## 0. Executive summary

| Field | Value |
|-------|--------|
| **Project class (PASS1)** | Greenfield multi-reference hybrid — composite oracles, not FrankenSQL 1:1 |
| **External Pattern-1 match-set CI** | **None** today (`jell-deferral.md` authoritative) |
| **Internal oracle culture** | Present and useful (ranking soft, graph, CE-003 IVF, HitKey peer, machine contracts, metamorphic) |
| **Composite dispatch SSoT** | **Absent** — Pass 1 Q1 still open; **true gauntlet residual** (not ghiw shell) |
| **Conformal / lower-bound cert number** | **Absent** (`parity_score.json` / Beta bands / truncate_score) |
| **Multi-pillar same-window gate** | **Absent** (forbidden-victory tracker) |
| **ghiw ownership** | Harness shell, DISC/COVERAGE, QG/MJ matrix, **pattern:×ast-grep** differential, fixtures/RT, compliance report + proof-pack runner |
| **Gauntlet residual maturity (oracle pillar only)** | **~4 / 10** readiness for skill-grade certification; ~6 / 10 counting internal refs already shipped |
| **Beads this pass** | **None** (HARD). Max **3** residual *themes* for Pass 11 |

**One-line verdict:** Oracles exist as **scattered harnesses**, not as a **scenario-dispatched composite** with EngineIdentity-asserted external differentials and a conformal lower-bound release score. ghiw will close the first external Pattern-1 slice and honesty registry; the gauntlet still owns **dispatch SSoT**, **certification lower bounds**, and the **multi-pillar convergence gate**.

---

## 1. Oracle inventory by channel

Subject everywhere: **asgrep / ast-sgrep workspace @ HEAD (v1.4.0)**.  
Product EngineIdentity: `docs/validation/engine-identity.md` (`tool=asgrep`, schema_version, embed_backend, index_format) — distinct from competitor identities.  
**Rule:** latency ledgers (`speed.md`, hyperfine, `speedup_vs_*`) are **not** correctness oracles (conformance PASS4 §4).

Legend for **Mode** (greenfield five-mode oracle, PASS1 §2.2):

| Glyph | Mode |
|-------|------|
| Sp | Spec / docs contract |
| Fx | Fixture soft/hard oracle |
| Sf | Self / peer surface |
| Rt | Round-trip / wire |
| Ex | External-tool (subset only) |
| Mt | Metamorphic (relation, not absolute truth) |
| Ma | Math / brute-force internal ref |

### 1.1 Channel matrix

| Channel | Authoritative oracle *today* (de facto) | Comparator / verdict shape | Paths (primary) | Greenfield modes in use | External? | Skill readiness | Notes |
|---------|------------------------------------------|----------------------------|-----------------|-------------------------|:---------:|:---------------:|-------|
| **Lexical** (`literal:` / `word:` / FTS keyword) | **None absolute.** Peer HitKey + product fail-closed; speed vs `rg` only | Latency p50/p95; optional future match-set `(file,line)` with DISC | `scripts/run-benchmarks.sh`; `docs/validation/jell-deferral.md`; HitKey peer tests | Sf (partial), Ex **timing only** | Timing yes / match-set **no** | **2/10** | ghiw later-phase `literal:`⊆rg after DISC-lexical-not-rg; **not** a current child. FTS ≠ rg semantics by design. |
| **Graph** (def / callers / imports / chain) | **Fixture graph oracle** | Non-empty / mode parity vs known symbols; case-fold queries | `crates/ast-sgrep-core/tests/graph_oracle.rs` | Fx, Sp (symbol norms partial) | No | **7/10** | Strong internal. No SCIP / external graph oracle (intentional). |
| **Structural / pattern:** (native subset) | **Smoke + routing**; no match-set vs ast-grep in CI | Routing tests; pattern lang unit tests; bench spawn optional | `pattern_routing.rs`; `ast-sgrep-lang/tests/pattern.rs`; structural speed scripts | Sp (structural-patterns.md), Fx partial | Latency yes / match-set **no** | **3/10** | **ghiw.3** owns bounded Pattern-1 vs `ast-grep` + DISC-pattern-native-subset. Full feature parity is non-goal. |
| **Semantic / ANN / IVF** | **Math oracle CE-003** (IVF top-k ≡ brute force when fully probing); sidecar RT + fingerprint | Set equality of top-k indices; fingerprint gate; adaptive non-vacuous n | `semantic_ivf_roundtrip.rs`; embed `math::`; `docs/validation/semantic-ivf-mmap.md` | Ma, Rt | No | **8/10** | Best internal Pattern-1-shaped suite. Not a PyTorch ULP port. |
| **Hybrid / NL ranking** | **Soft ranking fixture** (`must_include` + rank bounds); metamorphic relations | Soft include, not absolute RRF order freeze | `ranking_oracle.rs` + `tests/fixtures/ranking/cases.json`; `metamorphic.rs` | Fx, Mt | No (semgrep quality historical only) | **6/10** | Soft by design — absolute hybrid order vs competitors is non-goal (PASS4 §6). |
| **Machine / agent envelope** | **Contract tests + goldens path** | Scrubbed JSON equality; schema/engine-identity fields; fail-closed kinds | `machine_contracts.rs`; `docs/validation/machine-json-schema.md`; `engine-identity.md`; MCP `protocol.rs` | Sp, Sf, Rt (scrub) | No official MCP suite as release oracle | **7/10** | Envelope strong; multi-consumer freeze / dump freezes → **nz7i**. Official MCP suite → DISC (ghiw later / epic). |

### 1.2 Cross-cutting harnesses (not a product search channel)

| Concern | Oracle shape | Path | Owner if any |
|---------|--------------|------|--------------|
| Peer HitKey (CLI / core / LSP) | Sorted `SurfaceHitKey` equality | `no_embed_hit_key_parity.rs` (+ embed-on under lbx1) | lbx1 for embed-on residual; peer base in-tree |
| Extraction / languages | Presence goldens (13 langs); full dumps partial | `extraction_goldens`; golden program | **nz7i.4** dumps |
| Index durability / IVF wire | RT, schema `user_version=7`, IVF magic | RT suites; ghiw.4 corpus expansion | **ghiw.4** + existing RT |
| Fuzz crash / parse invariants | Differential-fuzz **not** suite scoring | `fuzz/` | **b8q3** |
| Forbid / soundness | Script gate | `scripts/verify-forbid-soundness` | proof-pack list |
| Proof pack | Manual command list | `docs/validation/proof-pack.md` | **ghiw.5** runnable gate |
| Full multi-engine hit-ID (`jell`) | **Deferred** | `docs/validation/jell-deferral.md` | Scope boundary — not “missing by accident” |

### 1.3 External tool pins (correctness candidates only)

| Tool | Host pin (PASS1 audit day) | Role if wired | In-tree match-set suite? | Ownership when landed |
|------|----------------------------|---------------|:------------------------:|------------------------|
| **ripgrep** `rg` | 15.1.0 | Lexical file/line subset oracle | **No** | ghiw epic “later phases” (after DISC); residual policy may stay deferred |
| **ast-grep** / `sg` | 0.45.0 | Supported `pattern:` subset match-set | **No** | **ghiw.3** |
| **semgrep** | 1.172.0 host | Quality bake-off only | **No** | Never correctness gate |
| **tree-sitter** crate | 0.26.10 lock | Parse substrate; dump gen | N/A product oracle | **nz7i** dumps + grammar pins |
| **hyperfine** | 1.20.0 | Latency driver | N/A | Perf pillar (not conf) |

**Envelope requirement for any future external differential** (conformance PASS4): DUT git SHA + competitor `--version` + corpus fingerprint + Pass/Fail/XFAIL + DISC rows. Self-comparison forbidden (skill EngineIdentity).

### 1.4 Composite dispatch — desired vs actual

**Desired (PASS1 §2.3 model):**

```text
Subject  = asgrep @ HEAD
Oracle   = scenario-dispatched composite:
             Spec | Ranking/graph fixtures | Peer HitKey | Math (IVF brute) |
             Prior-commit self | External (rg / ast-grep subset) | Tooling (miri/clippy/forbid)
Comparator = HitKey set | soft must_include | recall@k SLO | byte RT | scrubbed JSON |
             latency ledger (NEVER correctness)
```

**Actual:** No single SSoT file/module maps `(channel, scenario_class) → {oracle_mode, comparator, engine_ids, disc_ids, gate_class}`. Knowledge is split across proof-pack bullets, jell-deferral, structural-patterns, and per-test crates. **This is the #1 gauntlet residual for oracle readiness.**

---

## 2. What ghiw already owns vs gauntlet residual

### 2.1 ghiw program (filed; do not re-spec)

| ID | Owns | Oracle / differential relevance |
|----|------|----------------------------------|
| **ghiw** (epic) | Conformance harness program shape, later-phases fold-ins, non-goals | Holds “later: lexical ⊆ rg”; forbids full jell / MRR invention |
| **ghiw.1** | DISCREPANCIES.md seed, COVERAGE.md skeleton, verdict/XFAIL conventions, thin shell | Honesty substrate for all external and intentional divergences |
| **ghiw.2** | Query grammar + machine envelope MUST matrix + negative-path ledger | Spec-oracle numbering for QG/MJ/NL — not composite dispatch |
| **ghiw.3** | **pattern: subset × ast-grep match-set** differential + DISC-pattern-native-subset | First true external Pattern-1; env-gated competitor |
| **ghiw.4** | Fixture PROVENANCE, IVF/migration RT corpora, extraction presence→dump design | RT/math corpus strength; soft-dep **nz7i** dumps |
| **ghiw.5** | Compliance report emitter + proof-pack **runnable** gate | Report over suites; **not** conformal lower-bound score; cross-link nz7i.5 / b8q3.1 only |

**nz7i** owns assert_golden, scrub, dump freezes, CI golden hygiene — comparator *infrastructure*, not channel dispatch.  
**b8q3** owns fuzz bins/CI — crash oracles, not suite parity scores.

### 2.2 Ownership split (oracle-adjacent themes)

| Theme | Primary owner | Gauntlet residual? | Why |
|-------|---------------|:------------------:|-----|
| DISC / COVERAGE / verdict vocabulary | **ghiw.1** | No | Cross-link only |
| QG / machine MUST clause matrix | **ghiw.2** | No | Spec surface of contracts |
| pattern: × ast-grep Pattern-1 | **ghiw.3** | No (soft: post-DISC lexical rg later) | Highest-ROI external slice |
| IVF frames / migration DBs / PROVENANCE | **ghiw.4** | No | |
| Compliance report + proof-pack runner | **ghiw.5** | Partial — report ≠ conformal cert | Runner is ghiw; **lower-bound score + multi-pillar CERT** residual |
| Dump goldens / assert_golden | **nz7i** | No | |
| Fuzz continuous | **b8q3** | No | |
| Ranking / graph / CE-003 harnesses as they exist | In-tree (maintain) | No new bead for “have ranking_oracle” | Inventory only |
| **Composite oracle dispatch SSoT** | **Nobody** | **Yes** | PASS2 residual #2; PASS1 Q1 |
| **Conformal / Beta / greenfield lower-bound `parity_score`** | **Nobody** | **Yes** | PASS2 residual #8; skill release uses **lower bound**, not point estimate |
| **Multi-pillar forbidden-victory / same-window gate** | **Nobody** | **Yes** | PASS2 residual #1-adjacent tracker + cert Pass 9 |
| **Certification bundle** (multi-ref hybrid template) | **Nobody** | **Yes** | Pass 9 depth; seed honesty here |
| Full jell multi-engine hit-ID | Explicit non-goal | No (scope) | `jell-deferral.md` |
| Full ast-grep / rg feature parity | Explicit non-goal | No | Product design |
| Soft ranking absolute freeze | Non-goal | No | Soft oracle is correct policy |
| FeatureUniverse formal TOML + surface score | Gauntlet surface pillar (PASS6 draft) | Surface track | Not re-owned as “oracle” bead here |
| Keep-gate / bench history | Perf pillar (PASS4) | Perf track | Separate from oracle correctness |
| Skill progress negative ledgers | PASS5 B1–B3 themes | Ledger track | Import jell as Form-2 pointer when installed |

### 2.3 Gaps beyond ghiw (detail)

#### A. Composite dispatch SSoT (true residual)

**Missing artifact (design target, not implemented this pass):**

```text
docs/validation/oracle-dispatch.md   # or docs/contracts/oracle_dispatch.toml
  channel × scenario_class →
    authoritative_mode: Sp|Fx|Sf|Rt|Ex|Mt|Ma
    subject_id: Subject::asgrep
    oracle_id: Oracle::<fixture|math|peer|rg|ast-grep|spec>
    comparator: hitkey_set | soft_rank | topk_set | scrubbed_json | byte_rt | latency_only
    disc_ids: [DISC-…]
    gate_class: proof_pack | release_lower_bound | optional_env | never_correctness
    suite_path: …
```

**Why ghiw does not own this:** ghiw.1 is DISC/COVERAGE vocabulary and a thin shell; ghiw.2/3 land **specific** matrices and one external differential. Neither is the multi-channel **router** that answers “for hybrid NL, which oracle is authoritative?” without reading six files.

**Depends on ghiw (soft):** DISC IDs and COVERAGE rows should be *referenced* by the dispatch table once `.1` lands — related, not re-implemented.

#### B. Certification lower bounds (true residual)

Skill conformance gate for release: **conformal / Beta lower bound** (and greenfield-adapted feature weights), not “all listed cargo tests passed once.”

| Artifact | Status |
|----------|--------|
| `parity_score.json` / Beta bands / `truncate_score` | **Absent** |
| `feature_coverage.json` weighted by channel oracles | **Absent** (PASS6 draft matrix only) |
| Release uses **lower bound**, not point estimate | **Not encoded** |
| ghiw.5 compliance report Pass/Fail/Not-run | **Planned** — still a **point** suite report, not conformal |

**Honesty:** Internal suites being green is **necessary** evidence, not a skill-grade conformal certificate. Do not invent a green lower-bound number in audit-only.

#### C. Multi-pillar gate (true residual)

| Rule | Status |
|------|--------|
| Forbidden victory: no single pillar “done” alone | Documented in PASS2; **no tracker script** |
| Same-window: perf win blocked if conf/surface regress | **Absent** |
| Release certification bundle multi-ref (spec SHA, oracle suite versions, competitor pins, UNREPRODUCIBLE policy) | **Absent** template adapted to hybrid |

ghiw.5 may emit a conformance report; it must **not** be re-spec’d as the three-pillar CERT. CERT is gauntlet / Pass 9 material that **consumes** ghiw + nz7i + b8q3 + perf keep-gate evidence.

### 2.4 Maturity snapshot (oracle / differential only)

| Slice | Score | Basis |
|-------|:-----:|-------|
| Internal fixture/math/peer oracles | **6–8/10** by channel | graph/CE-003/machine high; lexical absolute low |
| External Pattern-1 | **~3/10** | None in CI; ghiw.3 designed not shipped |
| Composite dispatch SSoT | **1/10** | Model in PASS1 prose only |
| DISC/COVERAGE honesty registry | **0–1/10** | Missing files; ghiw.1 owns install |
| Compliance report automation | **1–2/10** | Manual proof-pack; ghiw.5 owns |
| Conformal lower-bound cert | **0/10** | Absent |
| Multi-pillar CERT / tracker | **0/10** | Absent |
| **Overall differential readiness (skill)** | **~4/10** | Strong internals; weak external + cert stack |

Aligned with conformance PASS4 report card (external ~3/10, internal ~6/10) and gauntlet PASS2 pillar (b) **5/10**.

---

## 3. Aggregated residual findings for beads (max 3; true gauntlet only)

**No beads created this pass** (HARD). Themes for **Pass 11** aggregation only.  
**Do not file** anything already under ghiw / nz7i / b8q3 / lbx1 / PASS5 ledger B1–B3 / PASS4 keep-gate / PASS6 surface S1–S3.

### R1 — Composite oracle dispatch SSoT (channel → mode → comparator → gate)

| | |
|--|--|
| **Priority** | P1 (conformance foundation for greenfield class) |
| **Pillar** | Conformance (+ honesty) |
| **Scope** | Land a single greppable dispatch contract (`docs/validation/oracle-dispatch.md` and/or `docs/contracts/oracle_dispatch.toml`) covering **lexical / graph / structural-native / semantic / hybrid / machine** with: authoritative mode, Subject/Oracle identity strings, comparator kind, DISC IDs (once ghiw.1 exists), suite path, and `gate_class` including explicit `latency_only` and `never_correctness` for speed ledgers. Wire a smoke test or doc-lint that every proof-pack suite row appears in the table (or reverse: every dispatch row has a suite or `disc`/`deferred`). |
| **Done when** | Pass 1 Q1 answerable from one file; `rg Oracle::` / Subject identity present; no channel claims “the” oracle without a row; jell / full rg identity listed as deferred or excluded with pointer to `jell-deferral.md`. |
| **Depends (soft)** | ghiw.1 DISC/COVERAGE IDs for row links; does **not** implement ghiw.3 harness. |
| **Out of scope** | Implementing Pattern-1 vs ast-grep (ghiw.3); inventing MRR; full jell. |
| **Evidence** | This file §1; PASS1 §2.3 + Q1; PASS2 residual #2; conformance PASS4 §8 design shell (do not duplicate harness under ghiw). |

### R2 — Greenfield conformal / coverage lower-bound scoring

| | |
|--|--|
| **Priority** | P1–P2 (release honesty) |
| **Pillar** | Conformance + Surface |
| **Scope** | Define a **greenfield-adapted** lower-bound score pipeline (not SQL e-process cargo-cult): weights over channels/oracle classes from R1 + FeatureUniverse (PASS6 draft); emit `parity_score.json` (or project-named equivalent) with **interval / lower bound** + truncate policy; document that release gates on **lower bound**. Seed with honest **red/yellow** from current missing external differentials and missing DISC — never invent green. |
| **Done when** | Score artifact path exists or is explicitly deferred with Form-2/5-style retry predicate in progress ledgers (PASS5); README/CERT never quote a point estimate as certified without lower bound; Agents.md published-number rules still bind MRR/etc. |
| **Depends** | Soft: R1 dispatch weights; PASS6 FeatureUniverse formalization; ghiw.5 suite Pass/Fail as **inputs**, not the score itself. |
| **Out of scope** | Replacing ranking soft oracle with hard absolute order; re-publishing UNREPRODUCIBLE baselines as certified. |
| **Evidence** | Skill kernel conformal pattern; PASS2 residuals #8 + pillar (b) gap #1; PASS1 Q8. |

### R3 — Multi-pillar same-window gate + multi-ref certification scorecard

| | |
|--|--|
| **Priority** | P2 (cert / Pass 9 prep) |
| **Pillar** | All three |
| **Scope** | (1) Document + optional thin tracker: **forbidden victory** — a “conformance green” or “perf win” cannot close release if another pillar is red in the same evidence window. (2) Draft `RELEASE_CERTIFICATION`-style checklist for **multi-reference hybrid**: subject SHA, oracle dispatch hash, competitor pins (when used), unreproducible policy, proof-pack/ghiw report status, golden freeze status (nz7i), fuzz gate (b8q3), keep-gate status (perf residual). All rows start **red/yellow** with evidence paths — no invented green. |
| **Done when** | Checklist file under `docs/validation/` or gauntlet assets path; three-pillar rule written in one greppable place; cert rows cross-link program IDs without re-owning their work. |
| **Out of scope** | Implementing ghiw.5 emitter; implementing keep-gate thresholds; shipping a green certificate from audit-only. |
| **Evidence** | PASS2 §2.6 / §3.6 / residual #9; PASS1 Q8; skill certification template adapted to hybrid. |

**If only one residual ships later:** **R1** (unblocks honest multi-oracle claims and weights for R2).

**Explicitly not residual beads (owned elsewhere):**

| Theme | Owner |
|-------|--------|
| DISC seed, COVERAGE, XFAIL conventions | ghiw.1 |
| QG/MJ MUST matrix | ghiw.2 |
| pattern: × ast-grep differential | ghiw.3 |
| PROVENANCE / IVF RT corpora | ghiw.4 |
| Compliance report + run-proof-pack | ghiw.5 |
| assert_golden / dumps / golden CI | nz7i.* |
| Fuzz CI / targets | b8q3.* |
| Lexical ⊆ rg (after DISC) | ghiw epic later-phases (may remain deferred) |
| Progress ledgers install | PASS5 B1–B3 |
| Keep-gate / bench history | PASS4 perf residual |
| FeatureUniverse TOML formalization | PASS6 S1–S3 |

---

## 4. Cross-pass map

| Source | Carry into Pass 7 | Treatment |
|--------|-------------------|-----------|
| Gauntlet PASS1 Q1 | Composite oracle completeness | §1 matrix + residual **R1** |
| Gauntlet PASS1 Q2 | External differential honesty | §1.3 + ghiw.3 ownership; jell non-goal |
| Gauntlet PASS1 Q8 | Certification readiness | §2.3 B–C + residual **R2/R3** |
| Gauntlet PASS2 pillar (b) | Conf 5/10; residuals #2,#8,#9 | Folded into R1–R3 |
| Gauntlet PASS2 residual #6 | Minimal external Pattern-1 | **ghiw.3** owns; not re-filed |
| Gauntlet PASS3 | Evidence honesty; oracles as soft/reproducible-in-tree | Inventory only |
| Gauntlet PASS4 | Keep-gate ≠ oracle | Not mixed into R1–R3 |
| Gauntlet PASS5 | jell Form-2 home in progress ledgers | Cross-link B1/B2; not re-owned |
| Gauntlet PASS6 | Feature weights feed R2 | Soft dep |
| Conformance PASS4 | Diff maturity 3/10 external; F1–F4 | F1→ghiw.3; F2→ghiw.1; F3 later; F4→ghiw.4/nz7i |
| Conformance PASS7_BEADS_FILED | Exact child map | §2.1 authoritative ownership |

---

## 5. Decision log (scope not silence)

| Decision | Status | Authority |
|----------|--------|-----------|
| Full jell multi-engine hit-ID equality | **Deferred / non-goal for now** | `docs/validation/jell-deferral.md` |
| Full ast-grep feature parity | **Non-goal** | structural-patterns / comparison honesty |
| rg-compatible FTS | **Non-goal** | jell-deferral + product FTS design |
| Absolute hybrid ranking vs competitors | **Non-goal** | soft ranking_oracle policy |
| Minimal pattern:×ast-grep match-set | **In scope for ghiw.3** | conformance PASS4 F1 + PASS7 beads |
| Minimal lexical×rg after DISC | **Later / optional** | ghiw epic later-phases |
| Composite dispatch SSoT | **In scope for gauntlet residual R1** | PASS1 Q1 / PASS2 #2 |
| Conformal lower-bound release number | **In scope for residual R2** | skill kernel |
| Three-pillar CERT | **In scope for residual R3** | PASS2 / Pass 9 |

---

## 6. Evidence log (what this pass actually did)

- Read gauntlet PASS1–PASS6 (structure + residual tables; PASS1 hybrid oracle model; PASS2 §2 conformance + §5 ranked residuals).  
- Read conformance PASS4 (differential inventory + F1–F4) and PASS7_BEADS_FILED (ghiw ownership graph).  
- `br show` epic `ast-sgrep-conformance-harness-program-ghiw` + child `.3` (read-only; no claim/create/close).  
- Confirmed on-disk oracles: `ranking_oracle.rs`, `graph_oracle.rs`, `metamorphic.rs`, `semantic_ivf_roundtrip.rs` (CE-003), `parity.rs`, `machine_contracts.rs`, HitKey peer tests.  
- Confirmed **absent:** `docs/contracts/`, `docs/progress/`, `parity_score.json`, composite dispatch SSoT file.  
- Read `docs/validation/{jell-deferral,engine-identity,proof-pack}.md` headers.  
- **Did not:** cargo, bead mutations, commits, product or docs implementation beyond this artifact.

---

## 7. Verdict block

| Item | Value |
|------|--------|
| **Artifact** | `/Users/aditya/Developer/ast-sgrep/tests/artifacts/gauntlet-audit/PASS7_ORACLE_DIFFERENTIAL.md` |
| **Channels inventoried** | lexical · graph · structural · semantic · hybrid · machine (+ cross-cutting) |
| **External match-set CI** | **None** (jell deferred; ghiw.3 will own first structural slice) |
| **ghiw vs residual** | ghiw = shell/DISC/matrices/pattern-diff/fixtures/report; residual = **dispatch SSoT · conformal lower bound · multi-pillar CERT** |
| **Residual bead themes** | **3** (R1–R3); none filed |
| **Cargo / beads / commit** | **none** |
| **Re-spec of ghiw/nz7i/b8q3** | **avoided** |

**DONE** — Pass 7 Oracle & differential readiness complete; audit-only; gaps beyond ghiw named; max 3 true-gauntlet residuals for Pass 11.

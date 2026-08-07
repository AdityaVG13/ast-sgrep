# Pass 2/16 — Three-Pillar Gap Inventory

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Branch:** `perf/software-optimization` (no switch)  
**Date:** 2026-08-07  
**Skill:** `running-the-gauntlet-on-your-rust-port` → [`references/THREE-PILLARS.md`](file:///Users/aditya/.cursor/skills/running-the-gauntlet-on-your-rust-port/references/THREE-PILLARS.md)  
**Prior:** [`PASS1_PROJECT_CLASS_REFERENCES.md`](./PASS1_PROJECT_CLASS_REFERENCES.md)  
**Mode:** audit-only · read/rg only · no product code · no beads · no commit · no workspace cargo  

**Class context (Pass 1):** greenfield multi-reference hybrid (T3 workspace). Pillars are scored against skill **gate definitions**, adapted to composite oracles (not FrankenSQLite 1:1 differential).

**Honesty rule:** No performance or quality numbers invented here. Any published figure cited below is already labeled in-tree (`UNREPRODUCIBLE`, superseded, or historical artifact). Agents.md: quote only from [`benchmarks/results/baselines.md`](../../../benchmarks/results/baselines.md) or tag unreproducible.

---

## 0. Executive summary

| Pillar | Headline maturity (1–10) | One-line status |
|--------|:------------------------:|-----------------|
| **(a) Performance** | **4 / 10** | Real tools (bench CLI, Criterion, release-perf, hotspots, optional 50% ratchet) but **not** skill pass-over-pass / weighted primary-score / HotPath counters; competitor + quality ledgers largely **UNREPRODUCIBLE** |
| **(b) Conformance** | **5 / 10** | Strong **internal** oracle / peer-parity / MR culture; almost no Pattern-1 external differential; **no** conformal lower-bound / e-process / DISC+COVERAGE skill stack |
| **(c) Surface parity** | **3 / 10** | Thin product tables (CLI/MCP/LSP/Pi + ~10 feature IDs); **no** `supported_surface_matrix.toml`, FeatureUniverse statuses, or `parity_score` / `feature_coverage.json` |

**Forbidden-victory reminder:** none of the three pillars may be declared “done” alone. Today none are green at skill gate; maturity scores are **inventory**, not certification.

**Parallel programs (do not re-file micro-work):**  
`ast-sgrep-golden-artifacts-program-nz7i` · `ast-sgrep-conformance-harness-program-ghiw` · `ast-sgrep-fuzz-program-maturity-b8q3` · mock-free epic `lbx1` · in-tree perf profiling under `tests/artifacts/perf/`.

---

## 1. Pillar (a) — Performance

### 1.1 Skill gate (what “done” means)

From THREE-PILLARS §Performance:

| Gate artifact | Skill expectation |
|---------------|-------------------|
| Headline report | Comprehensive multi-scenario bench JSON (v3-class), **per-category weighted primary score** — not raw mean ratio alone |
| Ratchet file | Committed `.bench-history/<bench>.latest.json` (pass-over-pass) |
| Regression thresholds | Primary score **−3%**, geomean **−5%**, category **−10%**, p90 **−15%**, throughput ratio drop **−5%** |
| Profile | `release-perf` (LTO / codegen-units discipline) |
| Detection | HotPath counters, flamegraph/samply/dhat triangulation, median+MAD detector, MT8 multi-writer attribution discipline |
| Negative ledger | `docs/progress/perf-negative-results.md` with **retry_condition** predicates |
| Competitor timing | Optional; must not masquerade as keep-gate **correctness** |

### 1.2 Evidence that exists today (paths)

| Path | Role | Skill fit |
|------|------|-----------|
| `Cargo.toml` `[profile.release-perf]` | Thin LTO, codegen-units=1 | **Present** (Phase 0-ish) |
| `crates/ast-sgrep-cli/src/bench.rs` | `asgrep bench` / `--suite`; `cv_pct`; optional history | **Partial** product bench |
| `crates/ast-sgrep-cli/.bench-history.json` | Schema v1, **single** entry (`query:process_request`) | **Thin** — not multi-bench committed history dir |
| `crates/ast-sgrep-core/benches/search.rs` | Criterion: process_request, NL hybrid, lexical, rank micro | **Micro** only |
| `crates/ast-sgrep-core/src/bench_suite.rs` | Suite fixture wiring for CLI bench | Supporting |
| `docs/benchmarks.md` | Documents `ASGREP_BENCH_RATCHET=1`, **50%** `ratchet_pct` | Coarse tripwire ≠ skill −3%/−5% |
| `scripts/check-bench-output.py` | Absolute `max-average-ms` + `identity_ok` on suite JSON | Absolute ceiling, not pass-over-pass |
| `scripts/check-error-budget.py` | Hyperfine p95 / same-host fingerprint drift | Stronger idea; not full primary-score stack |
| `scripts/run-benchmarks.sh` | Hyperfine warm literal vs `rg`, structural vs `ast-grep` | Latency only |
| `.github/workflows/speed.yml` | Manual: sample suite, `--max-average-ms 15` | Absolute gate |
| `.github/workflows/bakeoff.yml` | Manual: self suite, `--max-average-ms 100` | Absolute gate |
| `benchmarks/results/baselines.md` | Canonical quality/speed fingerprints; **UNREPRODUCIBLE** honesty banner | Provenance SSoT — **not** live harness |
| `benchmarks/results/speed.md` | Wall-clock vs competitors; **UNREPRODUCIBLE**; notes cold self-index budget **breached** vs historical 110-file budget | Historical ledger |
| `benchmarks/results/head-to-head.md`, `bakeoff.md`, `losses.md` | Competitor / loss narrative | Not keep-gate oracles |
| `docs/PERF_INVENTORY.md` | Cost drivers, micro-lever notes, IVF open conditions | Narrative + isolated micro claims |
| `docs/validation/semantic-ivf-mmap.md` + `ASGREP_PERF_ASSERTS=1` | Warm IVF open p99 assert (opt-in) | Focused correctness+perf assert |
| `docs/INSTRUMENTATION.md` | Stage timers behind `ASGREP_PERF_PROFILE=1` | Partial attribution; sample `perf_profile_sample.jsonl` **absent** from `benchmarks/results/` |
| `tests/artifacts/perf/20260702T180757Z/` | BASELINE, BUDGETS, hotspot_table, hyperfine JSON, CPU profiles | Real profile campaign (historical host) |
| `tests/artifacts/perf/20260806T211603Z/` | CPU/RSS/IO samples, analysis JSON | Later profile dump |
| `crates/ast-sgrep-embed/examples/bench_neural.rs` | Neural path example | Opt-in |

**Absent (skill-critical):**

- Repo-root / committed `.bench-history/*.latest.json` multi-scenario v3 reports  
- `comprehensive_bench`-class weighted primary score + category geomeans  
- Pass-over-pass gates at −3% / −5% (product ratchet is **50%**, optional env)  
- `HotPathProfileSnapshot` product counters / MT8 attribution form  
- `docs/progress/perf-negative-results.md` (no `docs/progress/` at all)  
- Runnable gold/eval harness that regenerates `baselines.md` quality rows  
- BOCPD / multi-day soak on parity-score stream (out of audit-only; residual for full gauntlet)

### 1.3 Headline maturity: **4 / 10**

| Band | Why this score |
|------|----------------|
| + | release-perf exists; product bench emits `cv_pct` + history meta; Criterion + real profile artifacts; absolute CI speed workflows; honesty banners on published numbers |
| − | Ratchet coarse (50%), history nearly empty, no weighted primary score, no HotPath skill stack, quality/competitor numbers UNREPRODUCIBLE, budgets stale vs grown corpus, no perf negative ledger |

Not lower: engineering has already built real measurement plumbing. Not higher: skill keep-gate is a **file-committed pass-over-pass program**, not optional 50% tripwires and historical markdown.

### 1.4 Critical gaps vs skill gates

1. **No skill-grade pass-over-pass primary score.** History is optional, one-key, and fails only at **+50%** mean latency — order of magnitude looser than −3%/−5%.  
2. **No committed multi-bench latest snapshots** under a `.bench-history/` contract the whole team treats as SSoT.  
3. **Competitor / quality ledgers misread risk.** `speed.md` / “parity clean” language is **latency/history**, not match-set correctness (conformance Pass4 already flags this).  
4. **UNREPRODUCIBLE quality fingerprints** with no in-tree regen path for the 18-gold / foreign bake-off rows (`baselines.md` banner).  
5. **Stale budgets:** 110-file cold index budget vs current self corpus (called out in `speed.md`) — re-baseline honesty unfinished.  
6. **Attribution incomplete:** hotspot tables exist as artifacts; no always-on HotPath counters / profile cards on every keep decision.  
7. **No perf negative ledger with retry_condition vocabulary.**

### 1.5 Cross-links to existing programs

| Owner | What they already cover for perf-adjacent work |
|-------|------------------------------------------------|
| **In-tree perf campaigns** (`tests/artifacts/perf/*`) | Hotspots, budgets, hyperfine dumps — optimization history, not gauntlet certification |
| **b8q3** (fuzz) | Sanitizer/exec floors, crash→regression — **not** end-to-end latency keep-gate |
| **lbx1.7** | ANN IVF scale quality/latency ignore gate (correctness-adjacent) |
| **ghiw / nz7i** | Do **not** own perf keep-gates; cross-link only when benches claim “parity clean” |
| **Gauntlet Pass 4 / 8** (later) | Keep-gate audit; competitor honesty |

### 1.6 Gaps NOT owned (true gauntlet residuals)

- Skill-shaped **weighted primary score + committed multi-entry bench history** with −3%/−5% gates  
- **Host fingerprint + same-machine** discipline as release policy (partial in `check-error-budget.py`, not product default)  
- **Perf negative ledger** + retry predicates under `docs/progress/`  
- **Rebaseline policy** for cold-index budgets after corpus growth  
- **Instrumentation sample artifacts** committed when claims cite stage timers  
- Convergence rule: perf win **blocked** if conformance/surface regress in same window (no tracker script yet)

---

## 2. Pillar (b) — Conformance

### 2.1 Skill gate (what “done” means)

| Gate artifact | Skill expectation |
|---------------|-------------------|
| Differential | Content-addressed FailureBundle / first-divergence; true Pattern-1 where a reference exists |
| Metamorphic | Typed families + `MismatchClassification` (CI fails only on `TrueDivergence`) |
| Fault / crash | Crash-boundary recovery proofs; fault schedules |
| E-process | Anytime-valid invariant monitoring (class-adapted for hybrid product) |
| Scoring | Beta posterior per category; **lower confidence bound** for release; `truncate_score` 6 dp |
| Negative ledger | `docs/progress/conformance-negative-results.md` + retry_condition |
| Greenfield adaptation | Composite oracle dispatch (spec / fixture / peer / math / prior-self / external subset) — Pass 1 model |

For this project: full external hit-ID identity is **explicitly deferred** (`docs/validation/jell-deferral.md`). Conformance “done” ≠ beat ripgrep bit-identically.

### 2.2 Evidence that exists today (paths)

| Path | Role | Skill fit |
|------|------|-----------|
| `docs/validation/proof-pack.md` | Minimal oracle/command list | **Manual** proof pack |
| `docs/validation/engine-identity.md` | EngineIdentity + FailureBundle exit map | Partial FailureBundle |
| `docs/validation/negative-ledgers.md` | Fail-closed cases (short list) | Static table, not retry ledger |
| `docs/validation/jell-deferral.md` | Full cross-engine differential deferred | **Authoritative non-goal** |
| `docs/validation/scored-property.md`, `machine-json-schema.md`, … | Domain notes | Spec fragments |
| `crates/ast-sgrep-core/tests/ranking_oracle.rs` + `tests/fixtures/ranking/cases.json` | Soft must_include + max_rank | Fixture oracle (~6/10 harness, ghiw Pass2) |
| `crates/ast-sgrep-core/tests/graph_oracle.rs` | Graph modes fixture | Internal |
| `crates/ast-sgrep-core/tests/metamorphic.rs` | Transform relations | MR suite — **not** absolute oracle |
| `crates/ast-sgrep-core/tests/semantic_ivf_roundtrip.rs` | CE-003 IVF vs brute; adaptive ignore | Best **internal** Pattern-1-shaped |
| `crates/ast-sgrep-core/tests/parity.rs` | Smoke / IVF preserve | Name oversells (3/10) |
| `crates/ast-sgrep-cli/tests/no_embed_hit_key_parity.rs` | CLI/core/LSP HitKey peer parity (+ embed-on post mock-free) | Peer differential |
| `crates/ast-sgrep-cli/tests/machine_contracts.rs` | Machine JSON contracts | Strong envelope |
| `crates/ast-sgrep-mcp/tests/protocol.rs` | Own MCP process tests | Not official MCP suite |
| `crates/ast-sgrep-lang/tests/extraction_goldens.rs` | Presence/forbid tuples | Same-engine family |
| `crates/ast-sgrep-core/tests/properties.rs`, `determinism_loop.rs`, durability/*, caches | Properties + consistency | Supporting |
| `fuzz/` + `tests/artifacts/fuzz-audit/` | Fuzz program maturity | Crash/invariant, not suite conformance |
| `tests/artifacts/conformance-audit/PASS1…7` | Full ghiw audit trail | **Owns** harness program design |
| `tests/artifacts/golden-audit/PASS1…7` | Golden freeze gaps | **Owns** dump goldens |
| `tests/artifacts/mock-free-audit/` | Soft-skip / mock risk | **Owns** vacuous-green honesty |

**Absent (skill-critical):**

- Differential V2 envelope with content-addressed `artifact_id`  
- `MismatchClassification` triage (TrueDivergence-only CI fail)  
- Conformal lower-bound ratchet / `parity_score.json`  
- E-process / crash-boundary matrix as skill catalog  
- `DISCREPANCIES.md` + living `COVERAGE.md` (ghiw.1 target)  
- `docs/progress/conformance-negative-results.md`  
- In-tree external Pattern-1 suites for **minimal** rg / ast-grep subsets (deferred; ghiw.3 owns first structural slice)

### 2.3 Headline maturity: **5 / 10**

Aligned with conformance-audit Pass1/Pass2 culture scores (~5/10 harness maturity; external Pattern-1 ~3/10; compliance report ~1–2/10). Composite score **5** reflects:

| Strength | Weakness |
|----------|----------|
| Real oracles, peer HitKey, CE-003, machine contracts, metamorphic honesty, mock-free culture | No conformal release score; jell external correctness absent; panic-only verdicts; no DISC/COVERAGE; negative ledger is static |

### 2.4 Critical gaps vs skill gates

1. **No conformal lower-bound release number** (`parity_score` / Beta bands / truncate_score).  
2. **External Pattern-1 near-zero** under intentional jell deferral — residual is *minimal subset* design + XFAIL, not “full rg identity.”  
3. **Verdict model:** panic-only; rare `#[ignore]`; no Pass/Fail/Skip/XFAIL enum or structured JSON-line results.  
4. **Mismatch taxonomy missing** — every failure is cargo red; no TrueDivergence vs OrderDependent triage.  
5. **Crash-boundary / fault-VFS skill arsenal absent** as a first-class matrix (durability tests exist; not crash-arming catalog).  
6. **E-process invariants not modeled** for hybrid index/search (generation counters exist; not Ville-threshold e-values).  
7. **Composite oracle dispatch not written as SSoT** (Pass 1 Q1 still open).  
8. **Compliance report / proof-pack CI automation red** (ghiw.5 + Pass6).

### 2.5 Cross-links to existing programs

| Program | Owns |
|---------|------|
| **ghiw** (+ `.1`–`.5`) | Harness shell, DISC/COVERAGE, query/machine MUST matrix, **pattern: vs ast-grep differential**, fixtures/RT, compliance report + proof-pack gate |
| **nz7i** (+ `.1`–`.5`) | assert_golden, scrubbers, CLI/agent/protocol dump freezes, extraction dumps |
| **b8q3** (+ `.1`–`.4`) | Fuzz targets, seeds, sanitizers, wire parse — **not** suite-level conformance scoring |
| **lbx1** mock-free | Embed HTTP/neural e2e, soft-skip kill, process surfaces (watch/LSP stdio) |
| **jell-deferral.md** | Scope boundary for full external hit-ID equality |

### 2.6 Gaps NOT owned (true gauntlet residuals)

- **Composite oracle dispatch matrix** (channel → authoritative oracle mode) as a single gauntlet contract  
- **Three-pillar forbidden-victory / convergence tracker** (scripts/convergence-tracker.sh skill analog)  
- **Conformal / Beta lower-bound scoring adapted to greenfield feature weights** (not SQL e-process copy-paste)  
- **Certification bundle** for multi-reference hybrid (spec SHA, oracle suite versions, unreproducible policy) — Pass 9  
- **Cross-program residual only:** lexical `literal:` ⊆ rg differential after DISC (ghiw “later phases”; not child) — may stay deferred  
- E-process / crash-boundary **skill-grade** catalog if product claims durability under kill (partial durability tests already exist — residual is *matrix + proofs*, not first test)

---

## 3. Pillar (c) — Surface parity

### 3.1 Skill gate (what “done” means)

| Gate artifact | Skill expectation |
|---------------|-------------------|
| Matrix | `docs/contracts/supported_surface_matrix.toml` — every reference/product feature `present \| partial \| missing \| n/a \| excluded` + rationale |
| FeatureUniverse | Typed features with weights; status promotions only with evidence |
| Scoring | `feature_coverage.json` + `parity_score.json` (Beta + conformal lower bound + truncate) |
| Negative / deferral | `docs/progress/surface-deferrals.md` with retry_condition |
| Enforcement | Verification contract on bead close / release (skill: fail-missing-evidence) |
| Certification | 100% verification of non-excluded obligations |

**Greenfield twist:** matrix is against **product promises** (hybrid search, agent formats, MCP tools, lang extractors), **not** full ripgrep or full ast-grep surfaces (Pass 1 non-goals).

### 3.2 Evidence that exists today (paths)

| Path | Role | Skill fit |
|------|------|-----------|
| `docs/validation/feature-universe.md` | ~10 feature IDs (hybrid/semantic/keyword/pattern/graph/chain/compact/doctor/mcp_index/forbid) | **List only** — no status enum, no weights |
| `docs/validation/surface-parity.md` | CLI / MCP / LSP / Pi capability table + intentional deltas | **Partial** matrix; no Code Mode / plugins / lang / formats rows |
| `docs/validation/compact-output.md` | Compact format contract + measured reduction narrative | Domain contract |
| `docs/comparison.md`, `docs/ARCHITECTURE.md`, `docs/mcp.md`, `docs/codemode.md`, `docs/pi-package.md` | Product surface docs | Narrative |
| HitKey peer tests | CLI/core/LSP search identity | Strong **one** surface class |
| MCP `protocol.rs`, LSP `lsp.rs`, codemode tests | Process/API smoke | Incomplete freeze (nz7i) |
| `crates/ast-sgrep-lang` extraction goldens | 13 langs presence | Partial vs full dumps |
| Golden / conformance audits | Inventory of missing freezes and MUST scores | Design evidence |

**Absent (skill-critical):**

- `docs/contracts/` entirely  
- `supported_surface_matrix.toml`  
- `parity_score_contract.toml` / `feature_coverage.json` / `parity_score.json`  
- Harness modules: `parity_taxonomy`, `invariant_catalog`, `feature_coverage_dashboard`, `validation_manifest`, `verification_contract_enforcement`  
- `docs/progress/surface-deferrals.md`  
- Status progression `Missing → Partial → Passing` with proof obligations  
- Gauntlet Pass 6 will draft FeatureUniverse — **not done yet**

### 3.3 Headline maturity: **3 / 10**

| Band | Why |
|------|-----|
| + | Documented multi-surface product; intentional MCP/LSP deltas written; HitKey peer parity; feature ID seeds; golden program will freeze dumps |
| − | No formal matrix, no weights, no lower-bound score, no deferred surface ledger, Code Mode/plugins/lang incomplete in the short table, no certification floor |

### 3.4 Critical gaps vs skill gates

1. **No present|partial|missing|excluded classification** for product-promised surfaces.  
2. **FeatureUniverse too small** vs real surfaces (formats × 6, 13 langs, MCP tool schemas, codemode catalog, Pi tools, doctor/status, watch, plugins).  
3. **No weighted parity score / coverage JSON** for release decisions.  
4. **Intentional exclusions** (MCP no auto-fusion; structural native subset; jell deferred) not encoded as `Excluded` + retry_condition.  
5. **Peer parity ≠ full surface parity** — HitKey covers search keys, not tool schemas / handbook / formats.  
6. **No verification_contract_enforcement** tying bead close to surface evidence.

### 3.5 Cross-links to existing programs

| Program | Surface-related ownership |
|---------|---------------------------|
| **nz7i** | Dump goldens for CLI hits/formats, handbook, MCP tools/list, codemode catalog, lang extraction trees |
| **ghiw.2** | Query grammar + machine envelope MUST matrix (conformance-facing surface of contracts) |
| **ghiw.3** | pattern: subset vs ast-grep (structural surface honesty) |
| **lbx1** | Embed-on surface parity, MCP semantic backends, codemode embed-on, LSP stdio e2e |
| **b8q3.4** | Wire protocol **fuzz** (parse robustness), not feature matrix |

### 3.6 Gaps NOT owned (true gauntlet residuals)

- **Formal FeatureUniverse + surface matrix** against product promises (gauntlet Pass 6 drafts; no program owns TOML + scoring yet)  
- **parity_score / feature_coverage** greenfield scoring pipeline  
- **surface-deferrals.md** with retry predicates  
- **Release certification template** adapted to multi-reference hybrid (not FrankenSQL CERT 100% copy)  
- Mapping **excluded** items to jell / comparison.md non-goals so “100% of non-excluded” is honest

---

## 4. Cross-pillar ownership map (de-dupe)

| Theme | Primary owner | Gauntlet residual? |
|-------|---------------|:------------------:|
| assert_golden + scrub + dump freezes | **nz7i** | No (cross-link) |
| DISC / COVERAGE / harness shell / compliance report | **ghiw** | No |
| pattern: × ast-grep minimal differential | **ghiw.3** | Soft residual: lexical ⊆ rg later |
| Fuzz continuous / seeds / sanitizers | **b8q3** | No |
| Mock-free embed / soft-skip / process e2e | **lbx1** | No |
| Hotspot profile dumps | `tests/artifacts/perf/*` campaigns | Partial — keep-gate still residual |
| UNREPRODUCIBLE quality fingerprints | `baselines.md` honesty | Yes — regen **or** permanent policy |
| Skill keep-gate (−3%/−5%, weighted score) | **Nobody** | **Yes** |
| FeatureUniverse formal statuses + parity_score | **Nobody** (Pass 6 draft) | **Yes** |
| Negative ledgers with retry_condition | Static `negative-ledgers.md` only | **Yes** |
| Composite oracle dispatch SSoT | Pass 1 Q1 open | **Yes** |
| Forbidden-victory three-pillar tracker | **Nobody** | **Yes** |
| Certification bundle multi-ref | **Nobody** | **Yes** |

---

## 5. Ranked residual list (≤10 deep themes)

For later bead aggregation (Pass 11). **Deep themes only** — not one bead per micro-path. Prefer cross-linking nz7i/ghiw/b8q3/lbx1.

| Rank | Theme | Pillars | Why residual | Notes |
|-----:|-------|---------|--------------|-------|
| **1** | **Skill-grade keep-gate & bench history** — committed multi-scenario history, weighted primary score, pass-over-pass ≤ skill thresholds (or documented greenfield-adapted thresholds still ≪ 50%), host fingerprint | Perf (+ honesty) | Product ratchet 50% + absolute ms gates ≠ skill gate | Pass 4 will deepen; do not invent numbers |
| **2** | **Composite oracle dispatch SSoT** — per channel (lexical/graph/structural-native/semantic/hybrid/machine JSON) authoritative mode + comparator | Conf | Pass 1 Q1 unanswered; prevents fake “one oracle” | Distinct from ghiw harness shell |
| **3** | **FeatureUniverse + surface matrix (product promises)** — present\|partial\|missing\|excluded + weights + intentional deltas (MCP no fusion, jell, native structural subset) | Surface | No TOML/JSON scoring stack | Pass 6 drafts; nz7i freezes dumps; matrix ownership is gauntlet |
| **4** | **Negative-ledger discipline** — `docs/progress/{perf-negative,conformance-negative,surface-deferrals}.md` + retry_condition vocabulary | All three | `docs/progress/` missing; static negative-ledgers only | Skill non-negotiable |
| **5** | **Published metric provenance closure** — restore gold harness **or** permanently lock baselines rows as historical; never dual-canonical | Perf + Conf honesty | Agents.md already binds quotes; regen path still open | Cross-link losses.md |
| **6** | **Minimal external Pattern-1 under jell** — optional rg file:line subset + ast-grep pattern subset with XFAIL; **not** full hit-ID | Conf | ghiw.3 owns structural first; lexical + policy residual | Pass 7 oracle readiness |
| **7** | **Attribution & HotPath counters** — stage/profile cards on keep decisions; commit samples when claims cite timers | Perf | INSTRUMENTATION + artifacts partial | Avoid micro-lever trap |
| **8** | **Greenfield conformal / coverage score** — lower-bound feature/oracle coverage number adapted to multi-ref (not SQL e-process cargo-cult) | Conf + Surface | No parity_score.json | Depends on themes 2–3 |
| **9** | **Certification readiness scorecard (honest red/yellow)** — multi-ref cert template; unreproducible policy; same-window three-pillar rule | All | No RELEASE_CERTIFICATION_TEMPLATE adapted | Pass 9; never invent green |
| **10** | **Budget rebaseline honesty** — cold-index / sample budgets vs current corpus size; supersede 110-file figures | Perf | Documented breach in speed.md | Fold into theme 1 if single epic |

**Explicitly NOT residual (owned elsewhere):** golden dump freezes (nz7i), DISC/COVERAGE file seed + pattern differential harness (ghiw), fuzz CI bin/seeds (b8q3), Ollama/cloud/neural mock-free e2e (lbx1).

---

## 6. Top 5 residuals (orchestrator report)

1. **Keep-gate / bench-history skill gap** (maturity blocker for pillar a)  
2. **Composite oracle dispatch SSoT** (pillar b foundation for greenfield class)  
3. **FeatureUniverse formal matrix + scoring** (pillar c)  
4. **Negative ledgers + retry predicates** (all pillars)  
5. **baselines.md provenance closure** (honesty; blocks any CERTIFIED claim)

---

## 7. Maturity scoreboard (final)

| Pillar | Score | Confidence |
|--------|:-----:|------------|
| (a) Performance | **4 / 10** | High — gates and files inspected; no cargo runs |
| (b) Conformance | **5 / 10** | High — aligns with ghiw Pass1–4 independent scores |
| (c) Surface parity | **3 / 10** | High — short docs + missing contracts dir |

**Combined readiness for skill CERTIFIED:** **blocked** (all three yellow/red; forbidden-victory would fire if any single pillar were over-claimed).

---

## 8. Evidence log (what this pass actually did)

- Read skill `references/THREE-PILLARS.md` (perf/conformance/surface gates, forbidden-victory, negative ledgers)  
- Read `tests/artifacts/gauntlet-audit/PASS1_PROJECT_CLASS_REFERENCES.md`  
- Inventoried: `benchmarks/results/*`, `docs/validation/*`, `docs/benchmarks.md`, `docs/PERF_INVENTORY.md`, `docs/INSTRUMENTATION.md`, `Cargo.toml` release-perf, CLI `.bench-history.json`, Criterion bench, workflows speed/bakeoff, `scripts/check-bench-output.py` / `check-error-budget.py`  
- Read parallel audits: conformance PASS2/3/4/6/7, golden PASS1/2/4/7, fuzz PASS7/10, mock-free PASS1/8, `tests/artifacts/perf/20260702*` BASELINE/BUDGETS/hotspot  
- Confirmed **absent:** `docs/progress/`, `docs/contracts/`, repo `.bench-history/`, `feature_coverage.json`, `parity_score.json`, `perf_profile_sample.jsonl`  
- **Did not run:** workspace cargo test/build/bench; did not file beads; did not commit; did not invent numbers  

---

## 9. Verdict block

| Item | Value |
|------|--------|
| **Artifact** | `/Users/aditya/Developer/ast-sgrep/tests/artifacts/gauntlet-audit/PASS2_THREE_PILLAR_GAPS.md` |
| **Perf maturity** | **4 / 10** |
| **Conformance maturity** | **5 / 10** |
| **Surface maturity** | **3 / 10** |
| **Top 5 residuals** | keep-gate; oracle dispatch; FeatureUniverse matrix; negative ledgers; baselines provenance |
| **Beads** | none (Pass 11 only) |

**DONE** — Pass 2 complete; audit-only; no beads; no commit.

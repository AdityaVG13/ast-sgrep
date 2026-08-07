# Pass 3/16 — Existing Evidence Honesty Inventory

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Branch:** `perf/software-optimization` (no switch)  
**Date:** 2026-08-07  
**Skill:** `running-the-gauntlet-on-your-rust-port` (One Rule: *gates that refuse to lie*)  
**Prior:** [`PASS1_PROJECT_CLASS_REFERENCES.md`](./PASS1_PROJECT_CLASS_REFERENCES.md), [`PASS2_THREE_PILLAR_GAPS.md`](./PASS2_THREE_PILLAR_GAPS.md)  
**Mode:** audit-only · no product code · no beads · no commit · **no** full cargo/rustc/test/bench  

**Honesty rule (this pass):** No numbers restated as live. Published figures are cited only as **UNREPRODUCIBLE** / **historical** / **ledger** with path, or as **reproducible-in-tree** only when the exact harness path exists and the artifact itself claims regen.

**Note on tools:** ZeroStack `zs` materialization truncated large file bodies to previews; full text was read from workspace paths after that limitation. Inventory is path/content inspection only.

---

## 0. Executive summary

| Lens | Finding |
|------|---------|
| **Canonical SSoT** | [`benchmarks/results/baselines.md`](../../../benchmarks/results/baselines.md) — fingerprint table + Agents.md quote rule |
| **Honest losses** | [`losses.md`](../../../benchmarks/results/losses.md), speed.md lexical losses, head-to-head caveats, jell-deferral non-claim |
| **Largest integrity risk** | **Dual-status ledgers**: file-level "UNREPRODUCIBLE / no harnesses" banners co-exist with subsection "reproducible from this tree" rows and with **missing** scripts named in reproduce blocks |
| **Skill keep-gate** | **Not present** as a gate that refuses to lie (absolute ms ceilings, optional 50% mean ratchet, single `.bench-history.json` key) |
| **Negative evidence** | Product static table (`docs/validation/negative-ledgers.md`); skill `docs/progress/*` + `retry_condition` **absent** |
| **Host coupling** | Nearly all published latency/quality rows are Apple M5 Max / single-host; only partial fingerprinting (`tests/artifacts/perf/20260702…/fingerprint.json`) |

**Pillar coverage of evidence artifacts:** Perf-heavy ledgers dominate; Conformance evidence is command lists + fixtures (not conformal scores); Surface evidence is short tables (no `parity_score`).

---

## 1. Evidence catalog

Status legend:

| Status | Meaning |
|--------|---------|
| **canonical** | Sole SSoT for a metric×corpus×config; must be cited for quotes |
| **historical** | Measured once; may still be true but not the release SSoT |
| **UNREPRODUCIBLE** | Numbers present; tree lacks harness and/or gold and/or raw dump needed to regenerate |
| **ledger** | Negative/deferral/policy evidence (not a score claim) |
| **reproducible-in-tree** | Exact command + fixtures (or named script) present **and** artifact claims regen path |

Host-coupled = yes when numbers depend on a specific machine/noise window without a committed same-host gate.

| Artifact | Pillar | Reproducible? | Host-coupled? | Status |
|----------|--------|:-------------:|:-------------:|--------|
| `benchmarks/results/baselines.md` — quality fingerprints (`self-hybrid-d3eab74`, `rg-hybrid-default-d3eab74`, `rg-neural-rerank-d3eab74`) | Conf (quality) + policy | **No** (gold + eval harness absent) | Yes (provenance: M5 Max) | **canonical** + **UNREPRODUCIBLE** |
| `baselines.md` — cold index / NL query / watch-mode tables | Perf | **No** (`watch-bench.py`, `corpora.lock` missing; hyperfine snippet only) | Yes | **historical** + **UNREPRODUCIBLE** |
| `baselines.md` — `self-hist-pre-29129bd` ~0.75 | Conf (quality) | No | Yes | **SUPERSEDED** (ledger of old figure) |
| `benchmarks/results/speed.md` — file banner | Perf | Claims none | — | **policy banner** (see §2 conflict) |
| `speed.md` — 2026-08-05 release-state self corpus (1,107 files) | Perf | **Partial yes** via `scripts/run-benchmarks.sh` + hyperfine (script **exists**) | Yes (M5 Max) | **reproducible-in-tree** for *self* warm/cold rows; still host-coupled |
| `speed.md` — 2026-07-10 head-to-head tables (self/rg/flask lexical+structural) | Perf | **No** (`run-speed-headtohead.sh`, `speed-report.py`, raw `results/20260710…` not in tree) | Yes (shared-machine noise admitted) | **historical** + **UNREPRODUCIBLE** |
| `speed.md` — Semgrep 29-pattern suite (20.96× aggregate) | Perf (+ partial conf narrative) | **No** (JSON dump not in tree; harness incomplete) | Yes | **historical** + **UNREPRODUCIBLE** |
| `speed.md` — 100k cold-overhead table | Perf | **No** (scale corpus `/tmp/scale-ann-…` not shipped) | Yes | **historical** + **UNREPRODUCIBLE** |
| `speed.md` — budget breach note (285 ms cold self-index vs 1,107-file reality) | Perf honesty | N/A | Yes | **ledger** (negative/stale-budget) |
| `benchmarks/results/head-to-head.md` — 23k/100k lexical+structural+Semgrep win rows | Perf | **No** (explicit "historical dump; not in-tree") | Yes | **historical** + **UNREPRODUCIBLE** |
| `head-to-head.md` — "parity clean" structural rows | Conf **misread risk** / Perf | No match-set dump in tree | Yes | **historical** latency claim; **not** Pattern-1 proof |
| `head-to-head.md` — 2026-08-05 self-corpus rows | Perf | **Partial yes** (`run-benchmarks.sh`) | Yes | **reproducible-in-tree** (host-coupled) |
| `benchmarks/results/bakeoff.md` | Conf (quality) | **No** | Yes | **historical** + **UNREPRODUCIBLE** |
| `benchmarks/results/losses.md` | Conf (quality honesty) | **No** (incomplete reproduce block; gold dump absent) | Yes | **historical** + **UNREPRODUCIBLE** + **ledger** (named losses) |
| `docs/validation/proof-pack.md` | Conf | **Commands exist** (not executed this pass) | No (unit/process tests) | **ledger** / gate checklist — not a score |
| `docs/validation/negative-ledgers.md` | Conf / Surface | Cases are product tests (static list) | No | **ledger** (static; no `retry_condition`) |
| `docs/validation/jell-deferral.md` | Conf | N/A | No | **ledger** (authoritative non-goal) |
| `docs/validation/engine-identity.md` | Conf / Surface | Spec | No | **ledger** (identity + FailureBundle map) |
| `docs/validation/feature-universe.md` | Surface | List only | No | **ledger** (IDs only; no status enum) |
| `docs/validation/surface-parity.md` | Surface | Partial matrix | No | **ledger** (CLI/MCP/LSP/Pi; incomplete) |
| `docs/validation/scored-property.md`, `machine-json-schema.md`, `semantic-ivf-mmap.md`, `compact-output.md`, … | Mixed | Domain notes | Mixed | **ledger** / domain contracts |
| `docs/validation/cargo-geiger-baseline.txt` | Conf (unsafe) | Text baseline | No | **historical** / **ledger** |
| `Agents.md` § Benchmark claims | Policy all pillars | N/A | No | **canonical policy** |
| `docs/RELEASING.md` unreproducible README rules | Policy | N/A | No | **policy** (aligns with Agents.md) |
| `README.md` quality snapshot (0.712 / 0.889 / 0.751) | Conf (public claim) | Points at baselines (UNREPRODUCIBLE row) | Yes (original measure) | **cites canonical row** — bare numbers without inline `UNREPRODUCIBLE` tag (see §2) |
| `crates/ast-sgrep-cli/.bench-history.json` | Perf | Yes (product bench history file) | Host-sensitive | **thin keep-adjacent** (1 key; not skill multi-bench) |
| `scripts/check-bench-output.py` (`--max-average-ms`) | Perf | Yes (script) | **Yes** (absolute ceiling) | **gate that can flip by host/corpus** |
| `scripts/check-error-budget.py` | Perf | Yes (script) | Partial same-host idea | **partial** honesty plumbing |
| `scripts/run-benchmarks.sh` | Perf | Yes | Yes | **reproducible harness** for self-corpus release-state rows |
| `.github/workflows/speed.yml` / `bakeoff.yml` | Perf | Manual workflow | CI host ≠ M5 Max | **absolute** gates; not pass-over-pass |
| `tests/artifacts/perf/20260702T180757Z/*` (BASELINE, BUDGETS, fingerprint, hyperfine JSON, hotspot) | Perf | **Partial** (JSON in tree; budgets for **110-file** self) | Yes (fingerprinted) | **historical** profile campaign |
| `tests/artifacts/perf/20260806T211603Z/*` | Perf | Partial (CPU/RSS/IO dumps; **no** `fingerprint.json`) | Yes | **historical** profile dump |
| `tests/fixtures/ranking/cases.json` + ranking/graph oracles | Conf | In-tree fixtures | No | **reproducible-in-tree** soft oracles (≠ published MRR gold) |
| Missing: `benchmarks/eval-bakeoff.py`, `watch-bench.py`, `speed-report.py`, `results.json`, `corpora.lock`, `results-*-speed.json` | Perf/Conf | — | — | **UNREPRODUCIBLE enablers** (named by docs, absent) |
| Missing: `docs/progress/{perf-negative,conformance-negative,surface-deferrals}.md` | All | — | — | **skill ledger gap** |
| Missing: `docs/contracts/supported_surface_matrix.toml`, `feature_coverage.json`, `parity_score.json` | Surface | — | — | **skill score gap** |
| Missing: multi-entry `.bench-history/*.latest.json` + weighted primary score | Perf | — | — | **skill keep-gate gap** |

### 1.1 Cross-check: harness named vs present (2026-08-07)

| Named by docs / reproduce blocks | Present in tree? |
|----------------------------------|:----------------:|
| `scripts/run-benchmarks.sh` | **Yes** |
| `scripts/run-speed-headtohead.sh` | **No** |
| `benchmarks/speed-report.py` | **No** |
| `benchmarks/eval-bakeoff.py` | **No** |
| `benchmarks/watch-bench.py` | **No** |
| `benchmarks/results.json` / lexical-structural JSON dumps | **No** |
| `benchmarks/corpora.lock` | **No** |
| `tests/fixtures/ranking/cases.json` | **Yes** (different schema/corpus than 18-gold MRR) |

---

## 2. Violations of skill "gates that refuse to lie"

Skill One Rule (SKILL.md): *If the gate can flip on a rerun, different host, fresh `target/`, renamed bench-history, or quiet default change, the gate is a lie.* Keep-gate rules require same-window focused+broad, `cv_pct`, MT8/profile attribution, pass-over-pass −3%/−5%, negative ledger with retry conditions.

### 2.1 Structural / policy violations (high)

| # | Violation | Evidence | Why it fails the One Rule |
|---|-----------|----------|---------------------------|
| V1 | **Dual status on the same files** | Every `benchmarks/results/{baselines,speed,head-to-head,bakeoff,losses}.md` opens with "every numeric row … **unreproducible**" + "No runnable harnesses ship"; `speed.md` §2026-08-05 and `head-to-head.md` §2026-08-05 claim **reproducible** via `run-benchmarks.sh` | Hostile reader cannot tell which rows the banner covers; banners overclaim absence of harnesses while a harness exists for a subset |
| V2 | **Reproduce blocks that cannot regenerate** | `head-to-head.md` "Reproduce" incomplete (missing harness binary names); `losses.md` reproduce ends mid-pipeline; `speed.md` points at `speed-report.py` / `run-speed-headtohead.sh` which are **MISSING**; `baselines.md` cites `watch-bench.py` / `eval-bakeoff.py` **MISSING** | Gate is narrative, not harness-encoded honesty |
| V3 | **Absolute latency gates as release proxies** | `check-bench-output.py --max-average-ms`; workflows speed/bakeoff | Flip on host, load, corpus growth; not pass-over-pass; no same-window dual gate |
| V4 | **50% optional mean ratchet ≠ skill keep-gate** | `docs/benchmarks.md` + `.bench-history.json` single key `query:process_request` | Order-of-magnitude looser than −3%/−5%; optional env; one scenario — can green while primary workloads regress |
| V5 | **Stale budgets still look authoritative** | `tests/artifacts/perf/20260702…/BUDGETS.md` cold index **110 files** &lt;250–285 ms; `speed.md` states budget **breached** on 1,107-file corpus | Quoting BUDGETS without corpus pin invents a pass; dual-canonical "budget" risk |
| V6 | **Latency "parity clean" without match-set proof** | `head-to-head.md` structural rows: "parity clean" + historical dump not in tree | Correctness-sounding language on a speed ledger (Pass2 residual; skill forbids masquerading competitor timing as correctness) |
| V7 | **No skill negative ledgers / retry_condition** | Only static `docs/validation/negative-ledgers.md`; no `docs/progress/*` | Rejected hypotheses can reappear as green; honesty not encoded for agents grepping ledgers |
| V8 | **Public README bare quality numbers** | README cites **0.712 / 0.889 / 0.751** with link to baselines but **no inline UNREPRODUCIBLE** | Agents.md prefers tag or canonical row; link helps, but product surface still looks "current guaranteed" under hostile read (`docs/RELEASING.md` warns exactly this) |
| V9 | **Config dual-MRR still easy to misquote** | Fingerprints separate 0.290 vs 0.605; bakeoff tables label neural run as "asgrep hybrid" | Mitigated in baselines, but bakeoff/losses wording still confusable without fingerprint id |
| V10 | **Host fingerprint not default keep discipline** | Perf campaign `20260702` has `fingerprint.json`; `20260806` does not; product CI absolute ms | Same binary, different host → gate flips; no concurrent_mode-style mode-default proof file |

### 2.2 What is *not* a violation (do not over-flag)

- Publishing **losses** (rg wins 2/3 lexical corpora; three asgrep rank losses; shared miss `rg_search_core`) — skill-aligned honesty.
- Explicit **jell-deferral** of full external hit-ID identity.
- Fingerprint ids + SUPERSEDED row for ~0.75 self MRR.
- Noise bounds (±30% wall clock; ranking deterministic to 3 decimals) written in baselines.
- Semgrep suite caveats (20/29 rejections; not universal rules).
- Warm-indexed vs cold-scan asymmetry called out for lexical "wins."

### 2.3 Gate flip scenarios (hostile checklist)

| Scenario | Can today's "gate" flip green/red without product change? |
|----------|-----------------------------------------------------------|
| Fresh `target/` rebuild cold noise | Yes for absolute ms / 50% ratchet near threshold |
| Different CI runner vs M5 Max | Yes for `--max-average-ms` and any host-tied budget |
| Corpus growth (110 → 1,107 files) | Already flipped cold-index budget; file still ships old BUDGETS |
| Rename / empty `.bench-history.json` | Thin history → ratchet silent or resets |
| Quiet feature flag (embed on/off, neural) | Yes for quality 0.290 vs 0.605 if configs conflated |
| Rerun without gold file | Quality rows cannot fail CI — they only exist as markdown |

---

## 3. What Agents.md / baselines.md already do right

### 3.1 Agents.md (`Benchmark and published-number claims`)

| Rule | Effect |
|------|--------|
| **No bare quotes** | Forces MRR/Recall/nDCG/latency/speedup to trace `baselines.md` **or** explicit `UNREPRODUCIBLE` + missing harness name |
| **Harness path required for "reproducible"** | Prevents calling markdown numbers live without gold + command + competitor pins |
| **Negative ledger** | Failures/withdrawals must be written down, not deleted to fake green |
| **Conflicting figures** | One versioned fingerprint per metric×corpus×config; demote rest to superseded / different config |

These four rules are the project's **policy kernel** and match skill K-2 (*honesty in the harness*) at the documentation layer. They are necessary but not sufficient: harness-encoded keep-gates are still missing.

### 3.2 baselines.md strengths

1. **File-level unreproducibility banner** (for historical quality/speed tables).  
2. **Canonical fingerprint table** with explicit `UNREPRODUCIBLE` per row and dual-config separation (`0.290` default hybrid vs `0.605` neural+rerank).  
3. **SUPERSEDED** handling for pre-`29129bd` ~0.75 / 0.746 self MRR.  
4. **Provenance block** (date, commit `d3eab74`, machine, rustc, competitor versions).  
5. **Pinned corpus SHAs** even when `corpora.lock` is absent (recoverability of trees, not golds).  
6. **Noise bounds** and rule that ranking diffs &gt; 0.001 are regressions, not noise.  
7. **Quote rules** restated: no number without reproduce path; rebaseline needs two runs + commit of results together.  
8. **Explicit** statement that current `tests/fixtures/ranking/cases.json` is a **different** schema/corpus than the 18-gold MRR run.

### 3.3 Adjacent good practice

| Artifact | Right thing |
|----------|-------------|
| `losses.md` | Publishes asgrep losses and shared miss; separates neural fingerprint from default hybrid |
| `jell-deferral.md` | Non-claim encoded as scope, not silence |
| `speed.md` losses section | rg wins 2/3; no hidden erasure of small-corpus losses when citing 23k/100k |
| `docs/RELEASING.md` | No unreproducible README GATE; optional job URL for regen |
| `docs/getting-started.md` | Points readers at baselines with UNREPRODUCIBLE note |
| Pass1/Pass2 audits | Already refuse to invent green; de-dupe vs nz7i/ghiw/b8q3 |

---

## 4. Aggregated honesty themes for beads (max 4)

Gauntlet-level only. Prefer Pass 11 aggregation. **Do not** refile golden dump freezes (nz7i), DISC/COVERAGE shell (ghiw), or fuzz CI (b8q3).

| # | Theme | Pillars | Why one bead-class epic | Out of scope (owned elsewhere) |
|---|-------|---------|-------------------------|--------------------------------|
| **T1** | **Published-ledger provenance closure** — resolve dual banners; mark each table section with `canonical \| historical \| UNREPRODUCIBLE \| reproducible-in-tree`; fix or delete reproduce blocks that name missing scripts; either restore gold/eval harness **or** permanently lock quality fingerprints as historical-only | Perf + Conf honesty | V1–V2, V8–V9; Pass2 residual #5; Agents.md already binds quotes but regen path open | nz7i goldens; fixture ranking cases |
| **T2** | **Keep-gate that refuses to lie** — committed multi-scenario history (or greenfield-adapted thresholds still ≪ 50%), same-host fingerprint on keep decisions, ban absolute-ms-as-sole-release-gate, stop using competitor latency as correctness | Perf | V3–V4, V6, V10; Pass2 residual #1; skill KEEP-GATE-RULES | Micro hotspot dumps in `tests/artifacts/perf/*` (campaign history only) |
| **T3** | **Negative-evidence discipline with `retry_condition`** — stand up `docs/progress/{perf-negative,conformance-negative,surface-deferrals}.md` (or product-equivalent) so rejected opts, stale budgets, jell exclusions, and withdrawn evals are greppable predicates, not only static fail-closed lists | All three | V5, V7; Pass2 residual #4; skill patterns 180/185 | Product `negative-ledgers.md` operational cases can remain and link up |
| **T4** | **Budget / corpus pin honesty** — rebaseline or archive 110-file BUDGETS; require corpus file-count + git SHA on every budget row; never leave breached budgets look "passing" | Perf | V5; speed.md already documents breach; Pass2 residual #10 | Full scale-corpus build (separate scale beads if any) |

**If only one epic ships first:** **T1** (stops false regeneration confidence and README/doc misquotes). **T2** is the skill-certification blocker for pillar (a).

---

## 5. Map to Pass 1 questions / Pass 2 residuals

| Pass 1 Q / Pass 2 residual | Pass 3 answer (evidence only) |
|----------------------------|---------------------------------|
| Q3 Perf keep-gate vs UNREPRODUCIBLE competitors | Competitor 23k/100k + Semgrep suite = **UNREPRODUCIBLE**; self 2026-08-05 = **partial in-tree** via `run-benchmarks.sh`; keep-gate still **not** skill-grade |
| Q4 Fingerprint rows | Canonical: `self-hybrid-d3eab74` (0.712…), `rg-hybrid-default-d3eab74` (0.290), `rg-neural-rerank-d3eab74` (0.605); SUPERSEDED: `self-hist-pre-29129bd` |
| Q6 Negative-ledger discipline | Static only; no retry predicates |
| Pass2 #1 keep-gate | Confirmed residual |
| Pass2 #5 baselines provenance | Confirmed residual; dual-banner is new concrete defect |
| Pass2 #10 budget rebaseline | Confirmed; BUDGETS.md still 110-file |

---

## 6. Evidence log (what this pass actually did)

- Read `tests/artifacts/gauntlet-audit/PASS1_*.md`, `PASS2_*.md`  
- Read full (or honesty-header + body sections): `benchmarks/results/{baselines,speed,head-to-head,bakeoff,losses}.md`  
- Read `docs/validation/{negative-ledgers,proof-pack,jell-deferral,engine-identity,feature-universe,surface-parity}.md`  
- Read `Agents.md` benchmark rules; README quality snapshot; skill One Rule + KEEP-GATE-RULES excerpts from skill-src  
- Verified path presence/absence: `run-benchmarks.sh` (yes); `eval-bakeoff.py`, `speed-report.py`, `watch-bench.py`, `run-speed-headtohead.sh`, `corpora.lock`, results JSON dumps, `docs/progress/`, `docs/contracts/` (no)  
- Listed `tests/artifacts/perf/20260702T180757Z` and `20260806T211603Z`; read `fingerprint.json`, `BUDGETS.md`, `BASELINE.md` headers  
- **Did not run:** cargo test/build/bench, hyperfine regen, beads, commits  

---

## 7. Verdict block

| Item | Value |
|------|--------|
| **Artifact** | `/Users/aditya/Developer/ast-sgrep/tests/artifacts/gauntlet-audit/PASS3_EVIDENCE_HONESTY.md` |
| **SSoT for quality quotes** | `benchmarks/results/baselines.md` fingerprints — all **UNREPRODUCIBLE** |
| **Best in-tree speed regen** | `scripts/run-benchmarks.sh` → self-corpus rows in speed/head-to-head 2026-08-05 only |
| **Top honesty defect** | Dual-status banners + missing harnesses still named in reproduce blocks |
| **Skill keep-gate** | **Absent** (absolute + 50% thin history) |
| **Beads** | none (themes T1–T4 for Pass 11 only) |

**DONE** — Pass 3 complete; audit-only; no beads; no commit; no numbers invented as live.

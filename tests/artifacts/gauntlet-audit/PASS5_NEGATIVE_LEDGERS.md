# Pass 5/16 — Negative-ledger discipline

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Skill:** `running-the-gauntlet-on-your-rust-port` — Negative-Ledger Mandate (patterns 180 / 185)  
**Mode:** audit-only · no cargo · no beads · no commit · no invented numbers  

**Inputs read:** PASS1–PASS4 under `tests/artifacts/gauntlet-audit/`; `docs/validation/negative-ledgers.md`; `docs/validation/jell-deferral.md`; `docs/validation/surface-parity.md`; `docs/validation/proof-pack.md` (+ related validation stubs); `benchmarks/results/{baselines,losses,head-to-head,speed,bakeoff}.md` headers/bodies; `Agents.md` / `AGENTS.md` negative-ledger + published-number rules; skill `SKILL.md` § Negative-Ledger Mandate; `references/patterns/180-NEGATIVE-LEDGER.md`; `references/patterns/185-RETRY-CONDITION-PREDICATE.md`; `references/methodology/RETRY-CONDITION-VOCABULARY.md` (forms 1–8 + anti-vocabulary); `assets/negative-ledger-seed.md`.

**Path check (this pass):** `docs/progress/` — **ABSENT** (no `perf-negative-results.md`, `conformance-negative-results.md`, or `surface-deferrals.md`).

---

## Executive summary

| Dimension | Finding |
|-----------|---------|
| **Skill three ledgers** | **Missing.** `docs/progress/` does not exist. No closed entries with `retry_condition_predicate`. |
| **Product negative evidence** | **Present but partial.** Fail-closed product cases, quality losses, historical speed caveats, and explicit jell non-goal are scattered under `docs/validation/` and `benchmarks/results/`. |
| **Agents.md mandate** | Product rule exists: do not delete failures; update results docs. **Does not** name the three skill ledgers, cass 60-day mine, or retry-condition vocabulary. |
| **Naming collision** | `docs/validation/negative-ledgers.md` is a **fail-closed product case table**, not a gauntlet rejection ledger (PASS3 V7). |
| **PASS3 T3** | Confirmed residual: stand up greppable negative-evidence discipline with predicates (this pass deepens inventory + recommended structure; does not implement). |
| **Risk if unfixed** | Agents re-run rejected opts / stale budgets / withdrawn evals; dual-banner honesty issues (PASS3) lack a durable “rejected / withdrawn” home with predicates. |

**Verdict:** Negative *evidence* exists in hybrid form (quality losses + fail-closed + deferrals + historical caveats). Negative-ledger *discipline* per skill (three pillar ledgers + forms 1–8 predicates + pre-flight mine) is **not** installed.

---

## 1. Inventory of existing negative-evidence docs

Mapped by **pillar affinity** (perf / conformance-quality / surface). Paths are absolute under the repo root. Status is structural only — no numbers restated as live.

### 1.1 Performance (speed / keep / budget honesty)

| Path | What it records | Skill-shaped? | Notes |
|------|-----------------|---------------|-------|
| `benchmarks/results/speed.md` | Competitor wall-clock history; losses section; cold-overhead / mmap caveats; small-corpus lexical losses vs rg | Partial ledger narrative | File-level **UNREPRODUCIBLE** banner + subset “reproducible from this tree” rows (PASS3 dual-status). Losses are prose, not `retry_condition` entries. |
| `benchmarks/results/head-to-head.md` | Aggregated win/loss classes; “Losses and caveats”; indexed-vs-scan honesty | Partial | “parity clean” is **latency** language (PASS2/3 residual) — not match-set conformance. |
| `benchmarks/results/baselines.md` | Cold index / NL / watch tables; SUPERSEDED fingerprint row | Partial | Quality fingerprints also live here; SUPERSEDED row is demotion ledger, not perf reject entry. |
| `docs/validation/pattern-prefilter-profile.md` | Notes historical one-off work-span/Brent numbers | Thin | Not a rejection ledger. |
| `docs/validation/semantic-ivf-mmap.md` / `ivf-alloc-bounds.md` | Bounds / mmap validation contracts | Contract | Fail-closed semantics for IVF headers — closer to product negative cases than perf rejects. |
| `tests/artifacts/perf/**` (e.g. BUDGETS, BASELINE under dated dirs) | Perf artifact history | Artifact | PASS3: budget breach vs grown corpus is honesty residual, not a greppable ledger entry with predicate. |

**Absent (skill):** `docs/progress/perf-negative-results.md`.

### 1.2 Conformance / quality honesty

| Path | What it records | Skill-shaped? | Notes |
|------|-----------------|---------------|-------|
| `benchmarks/results/losses.md` | Named retrieval losses vs semgrep hand-patterns; shared miss; UNREPRODUCIBLE banner | **Strongest quality negative ledger in tree** | Outcome rows (asgrep loss / win / tie / shared miss). No hypothesis / scratch / retry_condition fields. Reproduce block incomplete (PASS3 V2). |
| `benchmarks/results/baselines.md` | Canonical fingerprint SSoT; SUPERSEDED demotion | Policy + historical | Agents.md cites this as quote authority. |
| `benchmarks/results/bakeoff.md` | Bake-off aggregate / per-query history | Historical | Same UNREPRODUCIBLE class as losses. |
| `docs/validation/negative-ledgers.md` | Fail-closed operational cases (missing root, empty index, doctor, MCP escape, embed URL, empty binary, regex panic) | **Misnamed vs skill** | Product static table; harness stubs referenced. Not rejection-of-candidate ledger. |
| `docs/validation/jell-deferral.md` | Authoritative **non-goal**: full cross-engine hit-ID differential deferred | Deferral ledger (conformance) | Closest in-tree analogue of a surface/conformance “do not claim X” entry — still no predicate form. |
| `docs/validation/proof-pack.md` | Minimal gates + links to negative-ledgers / fixtures | Gate checklist | Points at product negative table. |
| `docs/validation/engine-identity.md` | EngineIdentity + FailureBundle map | Spec | Conformance identity, not rejected-opt ledger. |
| `docs/validation/scored-property.md` | Miri/TSan skipped in CI (cost) | Deferral note | Soft “not in merge bar” — no retry predicate. |
| `docs/validation/cargo-geiger-baseline.txt` | Unsafe baseline text | Historical | Inventory artifact. |
| `docs/validation/issue-12-senpi.md` | External monorepo graph validation snapshot | Evidence | Not a negative ledger; growth disclaimer present. |

**Absent (skill):** `docs/progress/conformance-negative-results.md`.

### 1.3 Surface (API / multi-surface / product scope)

| Path | What it records | Skill-shaped? | Notes |
|------|-----------------|---------------|-------|
| `docs/validation/surface-parity.md` | CLI / MCP / LSP / Pi capability matrix; intentional deltas (MCP no auto-fuse; LSP IDE-focused) | Partial surface deferral table | Deltas are intentional, not “rejected candidates with retry_condition”. |
| `docs/validation/feature-universe.md` | Feature ID list | IDs only | No supported/excluded/deferred enum status (PASS3). |
| `docs/validation/compact-output.md` | Compact format contract + measurement notes | Contract | Not a deferral ledger. |
| `docs/validation/machine-json-schema.md` | Machine JSON notes | Contract | — |
| `docs/validation/neural-trust.md` | ORT/neural trust boundary | Trust | Failures not silent hashed swaps — policy. |
| `docs/validation/childguard.md` | Supervisor Pid/ChildGuard notes | Safety | — |

**Absent (skill):** `docs/progress/surface-deferrals.md`.

### 1.4 Policy hooks (not ledgers, but related)

| Path | Content |
|------|---------|
| `Agents.md` / `AGENTS.md` § Benchmark and published-number claims | (1) no bare quotes without baselines row or `UNREPRODUCIBLE` tag; (2) harness path for “reproducible”; (3) **Negative ledger** = update results doc when eval fails/withdrawn — do not delete the miss; (4) conflicting figures demotion. |
| Global / skill AGENTS negative-ledger paragraph | Skill expects grep of **three** `docs/progress/*` files + cass 60-day mine + blocker if unavailable. **Not present** in product `Agents.md`. |
| `docs/RELEASING.md` | Unreproducible README rules (PASS3 cites alignment). |

### 1.5 Explicit absences (confirmed this pass)

| Expected by skill | Status |
|-------------------|--------|
| `docs/progress/` directory | **ABSENT** |
| `docs/progress/perf-negative-results.md` | **ABSENT** |
| `docs/progress/conformance-negative-results.md` | **ABSENT** |
| `docs/progress/surface-deferrals.md` | **ABSENT** |
| Entry field `retry_condition_predicate` (or `retry_condition`) in product docs | **Not found** as structured field |
| `scripts/mine-ledger.sh` in product tree | **ABSENT** (skill-only under skill pack) |
| Closed fake seed entries invented this pass | **Not created** (HARD: no invent numbers / no fake closed work) |

---

## 2. Skill requirements vs as-is

### 2.1 Skill requirements (condensed)

From **pattern:180-NEGATIVE-LEDGER**, **pattern:185**, **RETRY-CONDITION-VOCABULARY**, **SKILL.md § Negative-Ledger Mandate**:

1. **Three durable git-committed ledgers** under `docs/progress/`:
   - `perf-negative-results.md`
   - `conformance-negative-results.md`
   - `surface-deferrals.md`
2. **Mandatory entry shape** (pattern 180 verbatim fields): date/title/status; Hypothesis; Workload(s); Measurement summary; Outcome (`rejected | reverted | abandoned | within-noise | correctness-abandoned`); Scratch worktree; Profile evidence; **Retry-condition predicate**; Bead id; Commit.
3. **Retry-condition predicates** must use one of **8 forms** (never anti-vocabulary: “later”, “if it seems important”, “tracked elsewhere” without id, “TODO”, “future work”, …):
   - Form 1 — Profile attribution above noise  
   - Form 2 — Architectural defer  
   - Form 3 — Gate-driven reconsideration  
   - Form 4 — Standalone retirement  
   - Form 5 — Evidence-pipeline mandate  
   - Form 6 — Structural not numerical  
   - Form 7 — Workload-property threshold  
   - Form 8 — Blocked-by architectural dependency (named bead)
4. **AGENTS.md mandate paragraph:** before major perf work, grep the three ledgers + mine 60 days cass failure terms + recent commits; if cass/ledger unavailable, write **blocker** or **patch-ready** entry — never silent skip.
5. **Pre-flight tooling:** `mine-ledger.sh` / convergence tracker treat ledgers as first-class; entries with anti-vocabulary remain “open” for convergence.
6. **Seed file** (`assets/negative-ledger-seed.md`): header + vocabulary pointer + example seed entry + Open / Retired sections; lintable later.

Kernel axiom **K-3:** negative evidence is first-class output; three ledgers + retry predicates.

### 2.2 As-is gap matrix

| Skill requirement | As-is in ast-sgrep | Gap severity |
|-------------------|--------------------|--------------|
| Three files under `docs/progress/` | Directory missing | **Critical** |
| Entry schema (hypothesis/workloads/measurement/outcome/scratch/profile/retry/bead/commit) | Not used in product docs | **Critical** |
| `retry_condition_predicate` forms 1–8 | Absent | **Critical** |
| AGENTS paragraph naming three ledgers + cass mine | Only product “don’t delete failures” rule | **High** |
| Pre-flight mine-ledger script in repo | Absent | **Medium** (can call skill script from workspace later) |
| Fail-closed product cases | `docs/validation/negative-ledgers.md` | **None** for product honesty; **naming collision** with skill |
| Named quality losses | `benchmarks/results/losses.md` | Partial — good content, wrong schema |
| Explicit non-goal (jell) | `jell-deferral.md` | Partial — needs surface/conformance ledger home + Form 2/4/8-style predicate when promoted |
| Surface intentional deltas | `surface-parity.md` | Partial |
| Superceded / dual-config fingerprints | `baselines.md` | Partial policy ledger (Agents.md #4) |
| UNREPRODUCIBLE honesty banners | results/*.md | Honesty present; dual-status still a defect (PASS3) — not fixed by inventing closed perf rejects |
| Convergence / retired-candidates section | N/A | Missing with ledgers |

### 2.3 Naming collision (load-bearing)

| Name | Actual role |
|------|-------------|
| Skill “negative ledger” | Measured-and-**rejected** optimization / behavior candidates + retry predicates |
| Product `docs/validation/negative-ledgers.md` | Cases that must **not** succeed as silent empty hits (fail-closed) |

PASS3 already classified product file as **ledger (static; no `retry_condition`)**. Pass 5: keep product file; **do not rename skill concept into that path** without a clear dual-purpose split (recommended structure §3).

### 2.4 Agents.md vs skill mandate

| Agents.md “Negative ledger” | Skill mandate |
|-----------------------------|---------------|
| On failed/withdrawn eval/bake-off/gate: **update** results doc or short note under `benchmarks/results/` | Append structured entry to the matching **pillar** file under `docs/progress/` |
| Don’t close honesty work by omitting the miss | Don’t re-attempt rejected candidates without grepping ledgers first |
| No cass / 60-day / blocker language | Cass mine + blocker if unavailable |

Product rule is **compatible subset** of skill honesty; it is **not** sufficient for Phase-8 / K-3 compliance.

---

## 3. Recommended structure (hybrid project — no fake closed entries)

**Class context (PASS1):** greenfield multi-reference hybrid — not FrankenSQLite 1:1. Ledgers still apply per greenfield adaptation; oracles are composite.

### 3.1 Directory and files (create later; not this pass)

```
docs/progress/                          # CREATE (empty skeletons only when authorized)
  README.md                             # Index: what each ledger is; link product fail-closed table
  perf-negative-results.md              # Skill perf ledger
  conformance-negative-results.md       # Quality / oracle / differential rejects & withdrawals
  surface-deferrals.md                  # CLI/MCP/LSP/Pi intentional non-features & deferrals
```

Optional but recommended **bridge** (avoid dual homes without links):

```
docs/validation/negative-ledgers.md     # KEEP name; add 3–5 line header:
                                        #   “Product fail-closed cases.
                                        #    Rejected campaign candidates → docs/progress/*.”
```

### 3.2 Header text (each of the three)

Use skill seed header spirit:

> This ledger records [perf | conformance | surface] ideas that were measured and rejected (or explicitly deferred). Check it before starting a new campaign in this pillar, and add an entry whenever a candidate is abandoned, reverted, kept out of the tree, or withdrawn because evidence did not support the claim.

Then: link to `references/methodology/RETRY-CONDITION-VOCABULARY.md` **or** a short in-repo copy of forms 1–8 + anti-vocabulary (if skill path is not vendored).

### 3.3 Entry template (copy into each file once)

```markdown
### YYYY-MM-DD — <short title> — <status>

- **Hypothesis:** …
- **Workload(s) probed:** …
- **Measurement summary:** … (cite baselines.md fingerprint id or UNREPRODUCIBLE + missing harness; no bare invented numbers)
- **Outcome:** rejected | reverted | abandoned | within-noise | correctness-abandoned | deferred
- **Scratch worktree:** path or `none`
- **Profile evidence:** path or `n/a`
- **retry_condition_predicate:** "<exactly one of forms 1–8>"
- **Bead id (if applicable):** …
- **Commit (if attempted):** <sha | uncommitted | not attempted>
- **Related product docs:** (optional) e.g. losses.md#…, jell-deferral.md
```

### 3.4 How to seed **without** inventing closed work

Do **not** invent measurement rows or pretend opts were rejected. Allowed first content:

| Entry type | Where | Form | Content source (already in tree) |
|------------|-------|------|----------------------------------|
| **Open Candidates** section only | All three | n/a | Queue items from PASS2 residuals / PASS3 T1–T4 themes — “not yet measured” |
| **Index-only pointers** (status: `imported-from-results`, not `rejected`) | conformance | Form 5 or 3 when known | `losses.md` named losses: `rg_std_printer`, `rg_json_output`, `rg_overrides`, shared miss `rg_search_core` — **predicate = regenerate under pinned gold+harness when harness returns**, not fake MRR |
| **Authoritative deferral import** | conformance or surface | Form 2 | `jell-deferral.md` — full hit-ID differential reconsider only inside broader external-oracle redesign |
| **Intentional surface delta import** | surface | Form 2 or 4 | `surface-parity.md`: MCP no auto-fuse; LSP non-doctor — Form 4 if permanent product choice |
| **SUPERSEDED fingerprint policy** | conformance | Form 6 or 4 | `baselines.md` `self-hist-pre-29129bd` — structural demotion; not a perf retry |
| **Budget / dual-banner honesty** | perf | Form 3 or 5 | PASS3 V1/V5 themes — “Worth reconsidering when banners are split per-row” / “Do not retry cold-budget claim from cold read of 110-file BUDGETS” — **predicates only; no new latency numbers** |
| **Product fail-closed** | remain in `docs/validation/negative-ledgers.md` | n/a | Link from progress README; do not duplicate as “rejected optimizations” |

**Forbidden on first seed PR:** fabricating “we tried X, −N% on workload Y” without an existing artifact path.

### 3.5 Agents.md mandate paragraph (draft for later edit)

When implementation is authorized, append (skill-aligned, project-local paths):

> Before major perf or quality campaigns: `rg` / read `docs/progress/perf-negative-results.md`, `conformance-negative-results.md`, and `surface-deferrals.md`; mine recent session history for failure terms (`rejected`, `reverted`, `abandoned`, `slower`, `regressed`, `within noise`, `not a keep`, …); check recent commits. If ledgers or cass are unavailable, write a **blocker** entry rather than skipping. Closed entries must carry a **retry_condition_predicate** from the project vocabulary (forms 1–8). Product fail-closed cases remain in `docs/validation/negative-ledgers.md`. Published numbers still follow baselines.md / UNREPRODUCIBLE rules above.

Keep existing Agents.md items 1–4; extend item 3 or add item 5 rather than deleting the product rule.

### 3.6 Mapping existing docs → future homes

| Existing artifact | Future home | Migration style |
|-------------------|-------------|-----------------|
| `benchmarks/results/losses.md` | Keep as detailed quality report; **index** rows into conformance ledger | Pointer entries + Form 5 until harness restores regen |
| `speed.md` / `head-to-head.md` caveats | perf ledger when a candidate is rejected; keep results docs as measurement SSoT | Don’t move raw tables into progress/ |
| `jell-deferral.md` | surface-deferrals **or** conformance-negative (prefer **conformance** — oracle non-goal) | One Form-2 entry + keep short authoritative file |
| `surface-parity.md` deltas | surface-deferrals | One entry per intentional delta if durable |
| `docs/validation/negative-ledgers.md` | Stay product fail-closed | Header cross-link only |
| PASS3 dual-banner / missing harness names | perf + conformance **process** entries after fix work | Form 5 (“Do not retry from cold read of banner; use per-row status”) |

### 3.7 Hybrid-class extras (not FrankenSQL clones)

- **Quality channel losses** (NL vs hand-pattern asymmetry) belong in **conformance** ledger with explicit “asymmetric oracle” note — not perf.
- **Competitor speed “parity clean”** language: if ever used as keep justification, require either match-set proof **or** ledger Form 5 mandating evidence pipeline that separates latency from correctness (PASS2 residual).
- **Multi-surface** (CLI/MCP/LSP/Pi): surface ledger owns “won’t ship auto-fusion on MCP” style decisions so agents don’t “fix” intentional deltas.

---

## 4. Aggregated findings for beads (max 3; fold PASS3 T3)

**No beads created this pass** (HARD). Themes for Pass 11 / later aggregation only.

### B1 — Install three progress ledgers + Agents mandate + product naming bridge  
**(folds PASS3 T3; PASS2 residual “no perf negative ledger”; PASS1 Q6)**

| | |
|--|--|
| **Priority** | P1 (discipline) |
| **Pillars** | All three |
| **Scope** | Create `docs/progress/{perf-negative-results,conformance-negative-results,surface-deferrals}.md` with skill headers + empty Closed / Open / Retired sections; **zero invented measurement closes**; optional pointer imports per §3.4; `docs/progress/README.md` index; header on `docs/validation/negative-ledgers.md` clarifying product fail-closed vs campaign ledger; extend `Agents.md` / `AGENTS.md` with three-ledger + retry_condition + blocker rule (keep existing published-number rules). |
| **Done when** | Paths exist; entry template present; `rg retry_condition_predicate docs/progress` finds the template or real entries; Agents.md names the three files; product negative-ledgers.md links up. |
| **Out of scope** | Running benches; inventing rejected opts; wiring cass mine scripts unless trivial copy. |
| **Evidence** | This file §1–3; PASS3 T3; skill pattern 180/185. |

### B2 — Import known honesty residuals as **Open** or **pointer** entries (no fake rejects)

| | |
|--|--|
| **Priority** | P2 |
| **Depends on** | B1 |
| **Pillars** | Perf + Conformance |
| **Scope** | After B1: add **Open Candidates** (or `imported-from-results` pointers) for: (a) `losses.md` four named quality outcomes; (b) jell full differential non-goal → Form 2 draft predicate; (c) dual-status banner / missing harness names (PASS3 V1–V2) as process Open items; (d) budget rebaseline vs 110-file (PASS3 T4 / V5) as Open with Form 3-style gate language **without new numeric claims**. |
| **Done when** | Each known residual is greppable under `docs/progress/` with status Open or pointer; none claim measured reject without artifact path. |
| **Folds** | PASS3 T3 remainder + T4 process pointer; does not replace PASS3 T1 banner split epic. |

### B3 — Optional pre-flight: mine-ledger convenience + failure-term list for hybrid search

| | |
|--|--|
| **Priority** | P3 |
| **Depends on** | B1 |
| **Scope** | Document project failure-term list (skill default + hybrid terms: `UNREPRODUCIBLE`, `shared miss`, `parity clean`, `jell`, `no-rerank`, `within noise`, `budget breach`); either vendor `mine-ledger.sh` invocation notes in `docs/progress/README.md` or a thin wrapper that greps the three files. No cass requirement until available — **blocker** path must be documented. |
| **Done when** | Pre-flight instructions exist and reference real paths; silent skip forbidden in Agents.md. |

**Not separate beads (already covered elsewhere / do not duplicate):**

- PASS3 **T1** dual-banner / harness honesty → keep as honesty epic (not re-beaded here).  
- PASS3 **T2** / PASS4 keep-gate → separate certification track.  
- Product fail-closed expansion of `negative-ledgers.md` cases → product test work, not campaign ledger.

**If only one bead ships:** **B1** (= PASS3 T3 core).

---

## 5. Cross-pass map

| Prior pass | Relevant residue | Pass 5 treatment |
|------------|------------------|------------------|
| PASS1 Q6 | Retry predicates vs static docs | Answer: static only; structure recommended §3 |
| PASS2 #7 / no perf negative ledger | Critical gap | Confirmed; B1 |
| PASS2 #4 quality residual | losses narrative | Inventory §1.2; B2 pointers |
| PASS3 V7 / T3 | Skill ledgers absent | Deep inventory + bead theme B1 |
| PASS3 V1 dual banner | Honesty defect | Not a closed ledger entry; Open under B2; T1 remains primary fix |
| PASS4 keep-gate | Rejected candidates need ledger home | B1 enables future keep-gate rejections to land somewhere durable |

---

## 6. Evidence log (what this pass actually did)

- Confirmed `docs/progress/` **ABSENT**.  
- Listed `docs/validation/*` and `benchmarks/results/*`; read negative-ledgers, jell-deferral, surface-parity, losses, baselines headers, head-to-head caveats, Agents.md § published-number + negative ledger.  
- Read skill Negative-Ledger Mandate, pattern 180, pattern 185 summary, RETRY-CONDITION-VOCABULARY forms 1–8 + anti-vocabulary, negative-ledger-seed.md.  
- Read PASS1–4 for T3 / V7 / residual alignment.  
- **Did not:** cargo, beads create/close, commit, invent measurements, create `docs/progress/*` (audit-only deliverable under `tests/artifacts/gauntlet-audit/`).

---

## 7. Deliverable status

| Field | Value |
|-------|-------|
| **Artifact** | `/Users/aditya/Developer/ast-sgrep/tests/artifacts/gauntlet-audit/PASS5_NEGATIVE_LEDGERS.md` |
| **docs/progress** | **ABSENT** (confirmed) |
| **Skill three ledgers** | Not installed |
| **Product negative evidence** | Present (fail-closed + losses + deferrals + caveats) |
| **Beads** | none (B1–B3 themes only; B1 ≡ PASS3 T3) |
| **Cargo / commit** | none |

**DONE** — Pass 5 complete; audit-only; no cargo; no beads; no commit; no invented numbers.

# Pass 2/10 — Existing Harness Architecture Audit

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Branch:** `perf/software-optimization` (audit only; no beads, no implementation, no commits)  
**Date:** 2026-08-07  
**Skill:** `testing-conformance-harnesses` (loop step 4: HARNESS quality)  
**Prior:** [`PASS1_SPEC_SURFACE_INVENTORY.md`](./PASS1_SPEC_SURFACE_INVENTORY.md)  
**Scope:** Architecture quality of tests that claim parity / oracle / conformance / contract. Not clause extraction, not new tests.

---

## 0. Skill scoring rubric (applied uniformly)

Each harness is scored against the six Pattern-1 harness components:

| Component | What "present" means |
|-----------|----------------------|
| **Fixture loader** | Shared load path for goldens/cases (JSON, bytes, source), not ad-hoc `fs::write` only in the test body |
| **Comparator** | Named compare helper (structural / set / key / shape), not only inline `assert_eq!` soup |
| **Verdict** | Explicit Pass / Fail / Skip / XFAIL (ExpectedFailure); not panic-only |
| **Requirement levels** | MUST / SHOULD / MAY (or equivalent) on cases |
| **Structured results** | Serializable per-case result (JSON-line / enum) collectable by CI |
| **Report generator** | Compliance matrix / COVERAGE.md / markdown report binary |

**Score guide (1–10):**  
1–2 = smoke asserts · 3–4 = one strong piece · 5–6 = loader+comparator, cargo panic verdict · 7–8 = near skill (multi-verdict or levels) · 9–10 = full harness crate + DISCREPANCIES + report.

**Global absence (all Rust harnesses unless noted):**  
- No `ConformanceTest` trait, no `tests/conformance/` crate layout.  
- **No `DISCREPANCIES.md`** and **no `COVERAGE.md`** anywhere in-tree (confirmed search; only Pass 1 mentions them).  
- No shared `assert_golden` / `UPDATE_GOLDENS` workflow.  
- No requirement-level tagging.  
- No structured JSON-line conformance results and no `generate_report` binary.

---

## 1. Per-harness scorecard

### 1.1 `crates/ast-sgrep-core/tests/parity.rs`

| | |
|--|--|
| **Tests** | 3 (`parity_search_option_wiring`, `index_all_preserves_semantic_ivf_on_noop_and_file_failure`, `parity_index_defs_hybrid_chain`) |
| **Skill pattern** | Internal smoke / peer self-check — **not** Pattern 1 differential |
| **Score** | **3 / 10** |

| Component | Present? | Notes |
|-----------|:--------:|-------|
| Fixture loader | partial | `index_sample` / sample corpus via testkit; one test builds inline temp corpus |
| Comparator | no | Presence asserts (`any` Def/Caller/NL hit); IVF byte `assert_eq!` for sidecar preserve |
| Verdict | panic | cargo test fail only |
| Requirement levels | no | |
| Structured results | no | |
| Report generator | no | |

**Missing:** name is false friend (Pass 1); no case table, no multi-surface compare, no XFAIL.  
**Protect:** IVF sidecar preserve-on-noop / preserve-on-file-failure is a sharp durability check.

---

### 1.2 `crates/ast-sgrep-core/tests/ranking_oracle.rs` + `tests/fixtures/ranking/cases.json`

| | |
|--|--|
| **Tests** | 1 driver over ~12 fixture cases |
| **Skill pattern** | Soft oracle (must_include + max_rank) — Pattern 2 sparse / fixture oracle |
| **Score** | **6 / 10** |

| Component | Present? | Notes |
|-----------|:--------:|-------|
| Fixture loader | **yes** | Deserializes `cases.json` with `deny_unknown_fields`; binds fixture name `"sample"` |
| Comparator | **yes** | `hit_matches` + rank window `take(max_rank)` |
| Verdict | partial | Collects `failures: Vec<String>`, single terminal assert — multi-case report in panic message, not Pass/XFAIL enum |
| Requirement levels | no | All constraints treated equal |
| Structured results | no | Free-form strings only |
| Report generator | no | |

**Missing:** full ordered ranking golden; competitor differential; per-case JSON results; XFAIL for known soft ranks; COVERAGE of modes.  
**Protect:** This is the **best in-repo fixture→oracle loop** for retrieval: typed cases, deny_unknown_fields, multi-case failure aggregation, wired into proof-pack. Do not collapse back into hard-coded asserts.

---

### 1.3 `crates/ast-sgrep-core/tests/graph_oracle.rs`

| | |
|--|--|
| **Tests** | 1 large (`graph_oracle_defs_callers_imports_chain_parity`) |
| **Skill pattern** | Hand-built fixture oracle (~P4 presence / case-fold) |
| **Score** | **5 / 10** |

| Component | Present? | Notes |
|-----------|:--------:|-------|
| Fixture loader | partial | Inline `index_oracle_fixture()` writer (Rust + TS sources), not external golden files |
| Comparator | partial | Local loops: `symbols_named`, defs/callers queries, imports non-empty, chain expand; counts defs_ok / callers_ok / chain_ok |
| Verdict | panic | Aggregated counters then assert |
| Requirement levels | no | |
| Structured results | no | |
| Report generator | no | |

**Missing:** external case JSON; FQN / non-ASCII matrix; DISC for out-of-contract cases; structural graph golden (nodes/edges dump).  
**Protect:** Explicit multi-casing query table (`SYMBOLS`) targeting Issue #12 class; store-level + search-level dual check.

---

### 1.4 `crates/ast-sgrep-core/tests/downstream_correctness.rs`

| | |
|--|--|
| **Tests** | 6 bead-named regressions (`2hhq`, `50hx`, `ql1u`, …) |
| **Skill pattern** | Targeted regression oracles (post-bug contracts) |
| **Score** | **4 / 10** |

| Component | Present? | Notes |
|-----------|:--------:|-------|
| Fixture loader | no | Per-test hand-built stores / temps; one path reuses ranking `cases.json` semantics (embed must_include hard-fail) |
| Comparator | ad-hoc | Per-bead asserts (edge_count, shared hit lines, seed rules) |
| Verdict | panic | Culture: **no soft-skip** on empty embed must_include (documented) |
| Requirement levels | no | Bead IDs act as informal case IDs |
| Structured results | no | |
| Report generator | no | |

**Missing:** shared harness shell; case registry; XFAIL for deferred beads.  
**Protect:** Explicit "hard fail empty embed" policy (anti soft-skip) — aligns with testkit safety culture.

---

### 1.5 `crates/ast-sgrep-core/tests/metamorphic.rs`

| | |
|--|--|
| **Tests** | 18 `#[test]` (~25 `mr_*` helpers including proptest variants) |
| **Skill pattern** | Metamorphic relations — **explicitly not** absolute-oracle conformance |
| **Score** | **7 / 10** (as MR harness); **n/a as conformance** |

| Component | Present? | Notes |
|-----------|:--------:|-------|
| Fixture loader | partial | Temp corpora + proptest strategies; fixed fixtures for non-vacuous IVF paths |
| Comparator | **yes** | `hit_keys` sets, subset relations, bit-identical kmeans, probe monotony |
| Verdict | panic | Documented DROP matrix for weak MRs; early `return` only for empty-row guards (not SKIP verdicts) |
| Requirement levels | no | Strength matrix F×I/C in module docs (different axis than MUST/SHOULD) |
| Structured results | no | Module docs act as human matrix |
| Report generator | no | Strength tables in `//!` comments only |

**Missing:** machine-readable MR inventory; CI export of which MRs ran.  
**Protect:** Outstanding diagnosis of the oracle problem; ship-gate Score ≥ 2.0; clear separation from unit/differential; mutant-oriented design notes. **Do not rebrand as conformance.**

---

### 1.6 `crates/ast-sgrep-core/tests/semantic_ivf_roundtrip.rs`

| | |
|--|--|
| **Tests** | 9 (1 `#[ignore]`) |
| **Skill pattern** | Pattern 3 round-trip + corrupt reject + internal differential (IVF vs brute force) |
| **Score** | **7 / 10** |

| Component | Present? | Notes |
|-----------|:--------:|-------|
| Fixture loader | partial | Synthetic vectors in-test; no checked-in binary corpus of historical IVF frames |
| Comparator | **yes** | Vector equality, fingerprint gate, set equality IVF↔brute, corrupt case table |
| Verdict | partial | Panic fail; **one true SKIP**: `#[ignore = "release-mode ANN recall/latency tradeoff"]`; optional perf assert behind `ASGREP_PERF_ASSERTS=1` |
| Requirement levels | no | |
| Structured results | no | `eprintln!` latency lines only |
| Report generator | no | |

**Missing:** versioned on-disk golden frames (v1→v2); DISC for intentional format breaks; XFAIL registry for recall SLO.  
**Protect:** Corrupt-frame matrix without panic; fingerprint/generation reject; CE-003 non-vacuous threshold discipline (`n >= DEFAULT_ANN_THRESHOLD`); publication/atomic replace survival.

---

### 1.7 `extraction_goldens.rs` + `testkit::assert_language_conformance`

| | |
|--|--|
| **Tests** | 1 driver over 13 `LanguageConformanceCase`s |
| **Skill pattern** | Presence / forbid / pattern tuples (misnamed "conformance") |
| **Score** | **5 / 10** |

| Component | Present? | Notes |
|-----------|:--------:|-------|
| Fixture loader | **yes** | `include_str!` per language under `lang/tests/fixtures/extract/*` |
| Comparator | **yes** | Shared `assert_language_conformance`: parse clean, symbols/imports/calls/patterns, forbid list, span invariants |
| Verdict | panic | First failure aborts language case (no multi-language failure bag) |
| Requirement levels | no | |
| Structured results | no | |
| Report generator | no | |

**Missing:** full extract dumps (order/spans golden); grammar-version pin in fixture meta; DISC when tree-sitter upgrades change extract; XFAIL per language.  
**Protect:** Shared testkit API used by lang crate; span-covers-name invariant; forbid symbols (doc-only noise). Strongest **shared** "conformance-shaped" helper today.

---

### 1.8 `crates/ast-sgrep-cli/tests/machine_contracts.rs` + fixtures

| | |
|--|--|
| **Tests** | 16 |
| **Fixtures** | `capabilities.json`, `envelopes.json`, `machine_shapes.json` |
| **Skill pattern** | Pattern 2 + 5 (golden JSON + machine contract) |
| **Score** | **8 / 10** (strongest contract harness) |

| Component | Present? | Notes |
|-----------|:--------:|-------|
| Fixture loader | **yes** | `fixture(name)` via `include_str!` compile-time embed |
| Comparator | **yes** | Full value `assert_eq!` goldens; `assert_shape` key-set compare; `assert_success` / `assert_doctor_unhealthy` envelope invariants |
| Verdict | panic | Fail closed; no XFAIL; intentional product "skipped_reason" only on bench vs ast-grep when not compared |
| Requirement levels | no | Schema pin `1.0.0` is a de facto MUST on every success path |
| Structured results | no | |
| Report generator | no | |

**Missing:** UPDATE_GOLDENS workflow; full hit dumps frozen; MCP/LSP envelope unified with same goldens; DISCREPANCIES for intentional field drops (compact); clause IDs from machine-json-schema.md.  
**Protect:** Compile-time fixture embed; schema_version/tool/command/ok/exit_code gate on every success path; shape freeze separate from full dump; fail-closed operational/usage exit codes; embed-default-ON contract (anti `--no-embed`-only suite). **Proof-pack member.**

---

### 1.9 `crates/ast-sgrep-cli/tests/no_embed_hit_key_parity.rs`

| | |
|--|--|
| **Tests** | 3 (multi-mode no-embed; embed-on parity; related) |
| **Skill pattern** | Peer-surface differential (CLI ↔ core ↔ LSP), not external oracle |
| **Score** | **6 / 10** |

| Component | Present? | Notes |
|-----------|:--------:|-------|
| Fixture loader | partial | `CliSession::sample` + sample corpus |
| Comparator | **yes** | Sorted `SurfaceHitKey` / `HitKey` via testkit (`json_hit_keys`, `core_search_hit_keys`, `lsp_search_hit_keys`); embed-kind filter |
| Verdict | panic | Explicit non-empty embed keys (no soft-skip) |
| Requirement levels | no | |
| Structured results | no | |
| Report generator | no | |

**Missing:** third-party oracle; MCP surface in same matrix; structured per-query results.  
**Protect:** Rich HitKey identity (file, line, kind, symbol, callee, caller); sorted compare for order-unstable ties; embed-on and no-embed dual paths. Testkit `hit.rs` multi-format JSON normalization is excellent shared infra.

---

### 1.10 `crates/ast-sgrep-mcp/tests/protocol.rs`

| | |
|--|--|
| **Tests** | 12 |
| **Skill pattern** | Pattern 6 process harness (own server stdio JSON-RPC) |
| **Score** | **6 / 10** |

| Component | Present? | Notes |
|-----------|:--------:|-------|
| Fixture loader | partial | Temp repos; no golden response files |
| Comparator | partial | Exact tool name list; protocolVersion pin `2024-11-05`; compact tuple shape; byte-identical tools/list; envelope key order stability |
| Verdict | panic | |
| Requirement levels | no | |
| Structured results | no | |
| Report generator | no | |

**Missing:** official MCP conformance runner; schema golden files for tools/list; DISC vs MCP draft changes; multi-client matrix.  
**Protect:** Real process spawn; multi-request session for compact id continuity; sandbox root tests; schema rejection; tools/list byte stability. **Proof-pack member.**

---

### 1.11 Pi `release-contract.json` + `packages/pi/scripts/check-contract.mjs` (+ related tests)

| | |
|--|--|
| **Entry** | `packages/pi/release-contract.json` (schemaVersion 2) · `check-contract.mjs` · launcher/security tests · release-acceptance |
| **Skill pattern** | Pattern 5 contract testing (version/surface/layers) |
| **Score** | **8 / 10** (contract consistency); **5 / 10** if judged only as product-behavior conformance |

| Component | Present? | Notes |
|-----------|:--------:|-------|
| Fixture loader | **yes** | Contract + targets.json + package.json + Cargo.toml + runtime sources as inputs |
| Comparator | **yes** | Field required checks, version equality, platform matrix alignment, formatting determinism |
| Verdict | **partial** | Collects `errors[]` then exits non-zero — closest thing to multi-failure structured bag outside Rust |
| Requirement levels | partial | Prose MUST policies inside JSON; not tagged test cases |
| Structured results | partial | Console messages; not machine compliance matrix |
| Report generator | partial | Human log only |

**Missing:** full behavioral conformance of Pi tools vs CLI; no DISCREPANCIES for surface separation (MCP not linked).  
**Protect:** Single frozen contract file for versions, machineSchema `1.0.0`, indexFormat 7, tool/command lists, offline semantics. Best multi-package consistency gate in the monorepo.

---

### 1.12 DISCREPANCIES.md / COVERAGE.md

| | |
|--|--|
| **Status** | **Absent** (expected) |
| **Score** | **0 / 10** (component gap, not a harness) |

Skill mandates both for any conformance claim. Pass 1 already noted intentional divergences (pattern subset vs ast-grep, compact field drops, case-fold scope) live only in prose docs — not sequential DISC-NNN ledger with test linkage.

---

## 2. Scorecard summary table

| Harness | Path | Score | Pattern | Loader | Compare | Verdict | Levels | Struct | Report |
|---------|------|:-----:|---------|:------:|:-------:|:-------:|:------:|:------:|:------:|
| ranking_oracle | `core/tests/ranking_oracle.rs` | **6** | fixture oracle | Y | Y | bag+panic | N | N | N |
| graph_oracle | `core/tests/graph_oracle.rs` | **5** | fixture oracle | ~ | ~ | panic | N | N | N |
| parity | `core/tests/parity.rs` | **3** | smoke | ~ | N | panic | N | N | N |
| downstream_correctness | `core/tests/downstream_correctness.rs` | **4** | bead regressions | N | ~ | panic | N | N | N |
| metamorphic | `core/tests/metamorphic.rs` | **7*** | MR (not conf.) | ~ | Y | panic | N† | N | N† |
| semantic_ivf_roundtrip | `core/tests/semantic_ivf_roundtrip.rs` | **7** | P3 + corrupt | ~ | Y | panic+ignore | N | N | N |
| extraction_goldens | `lang/tests` + `testkit/lang` | **5** | presence "conf." | Y | Y | panic | N | N | N |
| machine_contracts | `cli/tests/machine_contracts.rs` | **8** | P2+P5 | Y | Y | panic | N | N | N |
| no_embed_hit_key_parity | `cli/tests/no_embed…` | **6** | peer differential | ~ | Y | panic | N | N | N |
| MCP protocol | `mcp/tests/protocol.rs` | **6** | P6 process | ~ | ~ | panic | N | N | N |
| Pi release-contract | `packages/pi/*` | **8** | P5 contract | Y | Y | errors[] | ~ | ~ | ~ |
| DISCREPANCIES/COVERAGE | — | **0** | mandatory docs | — | — | — | — | — | — |

\* Metamorphic score is for **MR harness quality**, not conformance maturity.  
† Metamorphic has a **human** strength matrix in docs; not skill RequirementLevel / report binary.

**Weighted takeaway:** Highest harness architecture quality is **machine_contracts** and **Pi check-contract**; best retrieval oracle loop is **ranking_oracle**; best round-trip is **IVF**; best shared assert helper is **assert_language_conformance**; none implement the full skill shell (trait + XFAIL + levels + report + DISC/COVERAGE).

---

## 3. Shared infrastructure opportunities (testkit)

Existing building blocks worth extending rather than re-inventing:

| Asset | Path | Opportunity |
|-------|------|-------------|
| `LanguageConformanceCase` + `assert_language_conformance` | `testkit/src/lang.rs` | Seed of a trait-like case runner; add multi-failure bag + optional golden dumps |
| `HitKey` / multi-format `hit_keys` | `testkit/src/hit.rs` | Canonical comparator for all surfaces; extend to MCP compact tuples |
| `SurfaceHitKey` + `core_search_hit_keys` / `json_hit_keys` / `lsp_search_hit_keys` | `testkit/src/index.rs` | Single peer-parity API; add MCP + Pi agent formats |
| `sample_root` / `index_sample` / `CliSession` | fixture, index, cli | Shared fixture loader root for sample corpus |
| Factory corpora | `testkit/src/factory.rs` | Graph/credential themes for oracle fixtures without ad-hoc writers |
| `RealGateStatus` anti soft-skip | `testkit/src/safety.rs` | Closest existing **verdict enum** culture — map to Skip vs Fail for optional real-network cases |
| `TestLogger` / `IndexSnapshot` | `testkit/src/test_log.rs` | Logging only; do not confuse with approval goldens (Pass 1 false friend) |

**Not in testkit today (gaps for later architecture work):**

1. Shared `assert_golden` / `UPDATE_GOLDENS` (CLI fixtures reinvent include_str + assert_eq).  
2. Shared `TestResult { Pass, Fail, Skipped, ExpectedFailure }` + JSON-line sink.  
3. Shared fixture path resolver (ranking cases live under `tests/fixtures/`; lang under crate; CLI under `tests/fixtures/` of crate).  
4. Requirement level + case ID fields on table-driven cases.  
5. Report aggregator (proof-pack is a **shell command list**, not a matrix generator).  
6. DISCREPANCIES registry loaded by harness to auto-XFAIL.

---

## 4. XFAIL vs SKIP vs panic patterns in use

| Pattern | Where | Role |
|---------|-------|------|
| **Panic / assert** | Nearly all harnesses | Default Fail |
| **`#[ignore = "…"]`** | `semantic_ivf_roundtrip` (ANN tradeoff) | Cargo skip — only real XFAIL-adjacent mark found in this set |
| **Env-gated assert** | `ASGREP_PERF_ASSERTS=1` on IVF open p99 | Optional hard assert; default does not fail on latency |
| **Feature `cfg`** | `parity_search_option_wiring` neural/rerank fail-closed when features off | Conditional assert block, not Skip |
| **Early `return`** | metamorphic row/query normalization helpers | Guard against vacuous data — **not** a Skip verdict |
| **Product `skipped_reason`** | machine_contracts bench `ast_grep_comparison` | Output field when comparison not run — test still Passes |
| **Anti soft-skip policy** | safety.rs, ranking embed must_include, no_embed parity, machine embed-on, downstream | Culture: empty optional channel → **Fail**, never silent green |
| **`RealGateStatus`** | testkit safety | NotRequested / Ready / RequestedUnavailable — hard Result, not cargo ignore |
| **Error bag then exit** | Pi `check-contract.mjs` | Multi-failure collection (best multi-verdict-ish pattern) |
| **Failure `Vec` then one assert** | ranking_oracle | Multi-case diagnostic, still single panic |
| **XFAIL / ExpectedFailure enum** | **nowhere** | Skill component absent |
| **SKIP verdict enum** | **nowhere** (only `#[ignore]` / env) | |

**Implication:** The repo prefers **hard fail over soft skip** (good for honesty) but has **no intentional divergence registry**. Known product divergences must either fail CI or live only in prose — never as documented XFAIL linked to a case ID.

---

## 5. Aggregated architecture findings (max 6 deep items)

These are later-bead-sized themes (not micro-nits):

### F1 — No shared conformance shell
There is no `ConformanceTest` trait, runner, structured `TestResult`, or report binary. Each suite is a free-standing integration test file. Highest leverage for any future "conformance program" is a thin testkit module (or small crate) that standardizes loader + verdict + JSON-line results, adopted first by ranking_oracle and machine_contracts.

### F2 — DISCREPANCIES / COVERAGE are missing while divergence is real
Product intentionally diverges from ast-grep, compact drops provenance fields, case-fold is ASCII-only, MCP is not full MCP suite. Without DISC-NNN + COVERAGE accounting, "conformance" language overclaims and intentional gaps look like bugs (or vanish when tests are green).

### F3 — Verdict model is panic-only (plus rare `#[ignore]`)
Cannot express ExpectedFailure, Skip-with-reason, or MUST vs SHOULD outcomes in CI artifacts. ranking_oracle's failure bag and Pi's `errors[]` are the only multi-failure patterns; neither is shared or serializable as a compliance matrix.

### F4 — Fixture loaders are excellent but fragmented
Three strong styles coexist: (a) compile-time `include_str!` goldens (CLI machine), (b) path-loaded JSON cases (ranking), (c) hand-written temp corpora (graph/MCP/metamorphic). No common root, naming, or UPDATE_GOLDENS discipline. Risk: golden drift and inconsistent review.

### F5 — Peer parity ≠ external oracle (architecture smell in naming)
`parity`, `extraction_goldens`, `assert_language_conformance`, and HitKey surface equality all compare **internal** paths. They catch surface drift well; they cannot detect shared semantic bugs. Harness design should keep peer differential separate from fixture oracles and from metamorphic suites (metamorphic already documents this correctly).

### F6 — Proof-pack is a command list, not a harness report
`docs/validation/proof-pack.md` gates ranking_oracle, graph_oracle, machine_contracts, MCP protocol, embed math — excellent merge discipline — but produces no COVERAGE matrix, no clause IDs, no DISC linkage. Elevating proof-pack to emit a structured summary would close the report-generator gap without inventing a second culture.

---

## 6. What is already excellent (protect)

1. **Machine contract goldens** — `schema_version` / envelope invariants / shape freeze / compile-time fixtures; proof-pack critical path.  
2. **Ranking cases.json oracle** — typed must_include + max_rank, deny_unknown_fields, multi-case failure aggregation.  
3. **Anti soft-skip culture** — testkit `RealGateStatus`, embed must_include hard-fail, no empty-embed green.  
4. **HitKey cross-surface comparator** — multi-format JSON normalization + sorted peer parity (CLI/core/LSP).  
5. **IVF wire discipline** — round-trip, fingerprint, corrupt reject table, non-vacuous brute-force differential (CE-003), ignore only where SLO is tradeoff-bound.  
6. **Metamorphic documentation quality** — oracle-problem diagnosis, strength matrix, DROP list, proptest + fixed fixtures.  
7. **MCP process harness** — real stdio session, protocol pin, tool list freeze, sandbox, compact id continuity.  
8. **Pi release-contract** — multi-package version/surface/schema pin with deterministic check script.  
9. **Shared lang conformance assert** — parse cleanliness + presence/forbid/spans in one testkit entry point.  
10. **Graph case-fold dual path** — store `symbols_named` + search `defs:` against mixed-case queries (Issue #12 class).

---

## 7. Component coverage heatmap (all harnesses combined)

| Skill component | Coverage today |
|-----------------|----------------|
| Fixture loader | **Partial–strong** on CLI machine + ranking + lang includes; weak elsewhere |
| Comparator | **Strong** HitKey + shape + hit_matches + IVF sets; fragmented ownership |
| Verdict Pass/Fail/Skip/XFAIL | **Fail-only** (panic); rare `#[ignore]`; intentional anti-Skip culture |
| Requirement levels | **Absent** |
| Structured results | **Absent** (except free-form failure strings / Pi errors array) |
| Report generator | **Absent** (proof-pack is shell checklist only) |
| DISCREPANCIES.md | **Absent** |
| COVERAGE.md | **Absent** |

**Harness maturity (this pass):** **5 / 10** — matches Pass 1 overall culture score: strong oracle/contract practice, incomplete conformance harness architecture.

---

## 8. Out of scope (confirmed)

- No implementation of harness shell, goldens, or DISC/COVERAGE files  
- No `br` beads filed  
- No commits  
- No clause extraction (Pass 3+)  
- Did not re-run full test suite (architecture read of sources + fixtures only)

---

## 9. Report card

| Item | Value |
|------|--------|
| **Deliverable** | `/Users/aditya/Developer/ast-sgrep/tests/artifacts/conformance-audit/PASS2_HARNESS_ARCHITECTURE.md` |
| **Harnesses scored** | 11 + DISC/COVERAGE gap |
| **Top scores** | machine_contracts **8**, Pi release-contract **8**, IVF **7**, metamorphic **7** (MR), ranking_oracle **6**, hit_key parity **6**, MCP **6** |
| **Weakest named "parity/conformance"** | parity.rs **3**, extraction_goldens **5** (name overclaims) |
| **Top 3 protect** | machine goldens; ranking cases loop; anti soft-skip + HitKey |
| **Top 3 later architecture themes** | F1 shared shell · F2 DISC/COVERAGE · F3 multi-verdict |
| **Beads filed** | none (per mission) |

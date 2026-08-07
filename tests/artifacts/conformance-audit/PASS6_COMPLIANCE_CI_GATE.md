# Pass 6/10 — Compliance Report & CI Gate

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Branch:** `perf/software-optimization` (audit only; no beads, no CI YAML, no commits)  
**Date:** 2026-08-07  
**Skill:** `testing-conformance-harnesses` (loop steps 7–8: MATRIX + MAINTAIN / CI report regeneration)  
**Prior:** PASS1–PASS5 under `tests/artifacts/conformance-audit/`  
**Cross-programs (do not duplicate beads):**
- Golden artifacts: `ast-sgrep-golden-artifacts-program-nz7i.5` (CI golden hygiene + CONTRIBUTING trigger drift + PR template)
- Fuzz maturity: `ast-sgrep-fuzz-program-maturity-b8q3` / `.1` (CI bin name, release-gate fuzz parity, PR/nightly tiers)

**Search coverage:** `CONTRIBUTING.md`, `docs/validation/proof-pack.md`, `docs/RELEASING.md`, `scripts/local-release-gate.sh`, `scripts/verify-forbid-soundness`, `scripts/check-bench-output.py`, all `.github/workflows/*.yml`, `package.json` Pi scripts, PASS1–5 report/COVERAGE absences, skill checklist item "CI regenerates report on every PR". Shell + `rg`; ZeroStack available (`zs 1.3.0`).

---

## 1. Executive summary

| Question | Answer |
|----------|--------|
| Does a **compliance / coverage report** exist? | **No** — no `COVERAGE.md`, no `DISCREPANCIES.md`, no `generate_report` binary, no harness markdown matrix artifact |
| Is any report **generated** by CI or local gate? | **No** — gates print cargo/npm PASS/FAIL; bench workflows upload **latency/identity** JSON only |
| Closest living "bar" docs | `docs/validation/proof-pack.md` (shell checklist), `docs/validation/feature-universe.md` (feature ID table), `docs/validation/surface-parity.md` (CLI/MCP/LSP/Pi capability matrix) |
| Skill gap: **"CI regenerates report on every PR"** | **Total miss** — (a) no report emitter, (b) **no PR-triggered workflows at all** (every workflow is `workflow_dispatch` only) |
| CONTRIBUTING accuracy | **Stale on two claims:** PR auto-runs `forbid-soundness` + `cargo check`; official release "invokes `local-release-gate.sh` through release-acceptance" |

**Verdict (this pass):** Report + CI matrix maturity **1/10**. The monorepo has strong **oracle/contract suites** and a documented **proof-pack**, but zero skill-shaped compliance reporting and zero automatic PR regeneration. Full workspace tests and fuzz are manual-dispatch only; the cheap default bar is human-run CONTRIBUTING commands.

This pass is **inventory + design only**. It does **not** implement CI YAML, file beads, or commit.

---

## 2. Command inventory (conformance-like)

### 2.1 Documented local bars

| Source | Commands | What it actually gates | Conformance-like? |
|--------|----------|------------------------|-------------------|
| **CONTRIBUTING default bar** | `bash scripts/verify-forbid-soundness` | First-party `unsafe` ban (not cargo-audit) | Soundness, not product clauses |
| | `cargo check --workspace -j1` | Typecheck | Build health |
| | `cargo test -p ast-sgrep-core --test parity -j1 -- --test-threads=1` | Thin e2e index+search/chain (file still named `parity.rs`; also see `e2e_smoke.rs`) | **Peer / smoke oracle** (not external differential) |
| | `cargo build --release -p ast-sgrep-cli -j1` + `./target/release/asgrep --help` | Binary links | Smoke |
| **CONTRIBUTING release bar** | `bash scripts/local-release-gate.sh` | See §2.2 | Full workspace test + fmt/clippy + **rank** fuzz only |
| **Proof pack** (`docs/validation/proof-pack.md`) | `verify-forbid-soundness` | Same soundness | |
| | `cargo test -p ast-sgrep-core --test ranking_oracle` | Soft rank oracle over `tests/fixtures/ranking/cases.json` (12 cases) | **Best retrieval oracle** |
| | `cargo test -p ast-sgrep-core --test graph_oracle` | defs/callers/imports/chain case-fold | **Graph oracle** |
| | `cargo test -p ast-sgrep-cli --test machine_contracts` | Machine JSON goldens + shapes + fail-closed exits | **Contract freeze** |
| | `cargo test -p ast-sgrep-mcp --test protocol` | MCP pin `2024-11-05`, tools/list, sandbox | **Process contract** |
| | `cargo test -p ast-sgrep-embed --lib math::` | Embed math unit | Math contract |
| **docs/RELEASING.md (Pi local)** | `npm run check:pi-contract` | `packages/pi/scripts/check-contract.mjs` vs `release-contract.json` | **Release contract** (Pattern 5) |
| | `npm run check:pi-release` | workflow structure check | Packaging |
| | `npm run test:pi-release-gate` / `test:pi-e2e` | pack/verify/self-test / loader E2E | Packaging, not search clauses |
| **README** | Same parity one-liner as CONTRIBUTING | Mirrors cheap bar | |

**Not a command but related docs:**
- `docs/validation/feature-universe.md` — feature ID inventory (`hybrid_search`, `pattern_search`, …); not executed.
- `docs/validation/surface-parity.md` — capability × surface table; not executed.
- `docs/validation/negative-ledgers.md` / `engine-identity.md` — prose MUST-not / identity; enforced only where tests exist.

### 2.2 `scripts/local-release-gate.sh` (17 lines)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
# requires cargo-fuzz + nightly:
cargo +nightly fuzz run rank -- -max_total_time=30 -timeout=5
```

| Property | Status |
|----------|--------|
| Emits compliance markdown? | **No** |
| Runs proof-pack subset explicitly? | **No** (indirectly via full workspace test) |
| Fuzz coverage | **`rank` only** — not `query_grammar` (owned by fuzz program `.1`) |
| Invoked by official Pi release workflow? | **No** (see §2.4 drift) |

### 2.3 GitHub Actions workflows (all `workflow_dispatch` only)

| Workflow | Jobs (relevant) | Conformance / correctness role | Report artifact? |
|----------|-----------------|--------------------------------|------------------|
| **`.github/workflows/ci.yml`** | `forbid-soundness`, `cargo-check` | Soundness + typecheck (manual) | None |
| | `build-and-test` (ubuntu+macos): `cargo build/test --workspace --release` | Full suite when manually run | None (panic-only) |
| | `windows-smoke` | CLI index/search/cancel paths | None |
| | `clippy`, `fmt`, `audit` | Quality / deps | None |
| | `bounded-fuzz` | Intended 30s `parsed_query` + `rank` | **Broken first step:** bin `parsed_query` does not exist (`query_grammar` is real) — owned by fuzz `.1` |
| **`bakeoff.yml`** | `asgrep bench` self suite → `check-bench-output.py --max-average-ms 100` | **Latency + identity_ok** gate; uploads `bakeoff-results.json` | Bench JSON, not compliance matrix |
| **`speed.yml`** | sample fixture bench → max 15 ms avg | Same class as bakeoff | `speed-results.json` |
| **`install-smoke.yml`** | crates.io install version check | Publish smoke | None |
| **`pi-cross-smoke.yml`** | mac dual-arch build + `version`/`doctor` | Binary smoke | None |
| **`pi-native-artifacts.yml`** | native build matrix + pack dry-run + `test:pi-e2e` | Pi packaging acceptance | tarball checksums, not COVERAGE |
| **`pi-npm-release.yml`** | `release-gate` (signed tag + contract) → build → verify → publish | **Package release**, not product clause matrix | Installs `cargo-fuzz` but **never runs fuzz or `local-release-gate.sh`** |

**Trigger truth (2026-08-07):** every workflow `on:` is **`workflow_dispatch` only**. No `pull_request`, no `push`, no `schedule`.

### 2.4 CONTRIBUTING vs reality (docs drift)

| CONTRIBUTING claim | Reality | Owner if fixed |
|--------------------|---------|----------------|
| "GitHub Actions runs `forbid-soundness` and `cargo check` on every `pull_request`" | **False** — `ci.yml` has no `pull_request` trigger; entire file is dispatch-only | **`nz7i.5`** (golden program: CONTRIBUTING drift + B4 PR-slice decision) |
| "Full build/test/clippy/audit/fuzz matrices remain `workflow_dispatch`" | **True** | — |
| "official package release invokes `scripts/local-release-gate.sh` through the release-acceptance command" | **False** — `packages/pi/scripts/release-acceptance.mjs` `gate` validates signed tag + registry plan + contract scripts only; **no** `spawn` of `local-release-gate.sh`, **no** `cargo test`/`clippy`/`fuzz` | **This pass finding** (release-path honesty); partial overlap with fuzz `.1` if Rust gate is re-wired |
| `pi-npm-release` installs nightly + cargo-fuzz | Install steps exist; **no subsequent fuzz run** in that job | Dead weight unless wire-up later |

### 2.5 Integration / oracle suites (inventory for report rows)

| Crate test file | Approx. `#[test]` count | Role / skill pattern | In proof-pack? | In CONTRIBUTING default? |
|-----------------|:-----------------------:|----------------------|:--------------:|:------------------------:|
| `core/tests/ranking_oracle.rs` | 1 (12 fixture cases) | Soft ranking oracle (P2/oracle) | **yes** | no |
| `core/tests/graph_oracle.rs` | 1 | Graph case-fold oracle | **yes** | no |
| `core/tests/parity.rs` | 3 | Thin e2e parity smoke | no | **yes** |
| `core/tests/e2e_smoke.rs` | 6 | Broader e2e (renamed lineage from parity) | no | no |
| `cli/tests/machine_contracts.rs` | 16 | Machine JSON freeze | **yes** | no |
| `mcp/tests/protocol.rs` | 15 | MCP process contracts | **yes** | no |
| `embed` `math::` lib tests | (lib filter) | Math | **yes** | no |
| `lang/tests/extraction_goldens.rs` | 1 (13 langs) | Presence "conformance" tuples | no | no |
| `core/tests/semantic_ivf_roundtrip.rs` | 9 | IVF wire RT (P3) | no | no |
| `core/tests/metamorphic.rs` | 22 | MR suite (not absolute oracle) | no | no |
| `core/tests/signal_provenance.rs` | 2 | Provenance fields | no | no |
| `cli/tests/no_embed_hit_key_parity.rs` | 3 | Cross-surface hit keys | no | no |
| `lsp/tests/lsp.rs` | 13 | LSP smoke | no | no |
| `core/tests/pattern_routing.rs` | 3 | pattern: routing | no | no |
| `core/tests/properties.rs` | 7 | Parse never panics etc. | no | no |
| Pi `check-contract.mjs` | N/A (Node) | Release contract | via RELEASING | no |

There is **no** cargo feature, test name prefix, or tag attribute that marks "conformance suite membership." Membership is tribal (proof-pack list, CONTRIBUTING, agent memory).

---

## 3. Gap analysis — report generation

### 3.1 Skill requirements (checklist items for this pass)

From skill **Compliance Report Generator** + checklist:

| Skill expectation | In-repo status |
|-------------------|----------------|
| Structured results collectable by CI (JSON-line / `TestResult` enum) | **Absent** — cargo panic / assert messages only |
| Markdown matrix: section × MUST/SHOULD × pass/total × score | **Absent** |
| `generate_report` binary (or equivalent script) | **Absent** |
| `COVERAGE.md` living artifact | **Absent** (PASS5 recommended skeleton only) |
| `DISCREPANCIES.md` loaded for XFAIL | **Absent** |
| **CI regenerates report on every PR** | **Absent** on both axes (no emitter; no PR CI) |
| Fixture regenerate / maintain loop documented | Partial only in audit trees; product CONTRIBUTING has no golden/report SOP (golden `nz7i.5` owns SOP) |

### 3.2 What exists that is *almost* a report

| Artifact | Why not enough |
|----------|----------------|
| **`docs/validation/proof-pack.md`** | Human command list. No pass counts, no clause IDs, no machine emission. PASS2 F6. |
| **`docs/validation/feature-universe.md`** | Static feature IDs; not tied to test outcomes. |
| **`docs/validation/surface-parity.md`** | Capability matrix; not executed. |
| **PASS3 coverage tables** | Audit-time estimates (`assumed passing`); not regenerated in CI. |
| **PASS5 COVERAGE skeleton** | Design only under artifacts. |
| **`asgrep bench` JSON + `check-bench-output.py`** | Gates `ok` + `identity_ok` + latency; wrong domain for MUST-clause compliance. |
| **Pi `check-contract.mjs` / preflight** | Console PASS/FAIL for packaging; not search/spec matrix. |
| **`pipeline_parts::write_json` / `sub1ms` report** | Perf micro-report, not conformance. |

### 3.3 Gap vs skill: "CI regenerates report on every PR"

Decompose into three independent gaps:

| # | Gap | Severity for skill claim | Notes |
|---|-----|--------------------------|-------|
| **G-R1** | No report generator over oracle/contract suites | **Blocker** for skill MATRIX step | Even local `generate_report` does not exist |
| **G-R2** | No PR-triggered CI of any kind | **Blocker** for "every PR" | Product may *intentionally* keep Actions manual (minute cost); skill still fails until either PR job exists **or** claim is explicitly out-of-scope |
| **G-R3** | No structured per-case tags (MUST/SHOULD, clause ID, surface) | **Blocker** for meaningful matrix rows | Tests are untagged free-standing files; ranking cases have `name` only |

**Recommended product stance (audit recommendation only):** Treat skill PR regeneration as a **future optional tier**, not an immediate full-workspace tax. Minimal honest path:

1. **Local / release:** regenerate markdown from a **static registry + last cargo results** (or cargo `--format json` post-process of a **named proof-pack filter**).
2. **PR (if enabled later):** cheap job = proof-pack tests + report artifact upload — **not** full `cargo test --workspace` unless cost-approved (align with golden `nz7i.5` B4 decision record).
3. **Do not** claim "conformance report in CI" while workflows remain dispatch-only.

### 3.4 Overlaps already owned (do not re-bead)

| Theme | Existing bead | Why cross-link only |
|-------|---------------|---------------------|
| CONTRIBUTING claims PR soundness/check; all workflows dispatch-only | **`ast-sgrep-golden-artifacts-program-nz7i.5`** | Explicit acceptance: fix CONTRIBUTING drift **or** document dispatch-only; B4 PR-slice decision |
| CI golden compare-only env + `*.actual` upload | **`nz7i.5`** | Golden hygiene, not compliance matrix |
| PR template checkboxes | **`nz7i.5`** | Review process |
| CI fuzz bin `parsed_query` vs `query_grammar` | **`ast-sgrep-fuzz-program-maturity-b8q3.1`** | G-CI-NAME |
| Release gate rank-only fuzz; dual-target parity | **`b8q3.1`** | G-RELEASE-PARITY |
| PR/nightly continuous fuzz tiers | **`b8q3` / `.1`** | G-CI-TRIGGERS for fuzz only |

**This pass owns only** report generation + proof-pack→matrix elevation + honesty about release path not running `local-release-gate.sh` / not emitting compliance reports.

---

## 4. Proposed minimal report design (design only — do not implement)

### 4.1 Goals (smallest useful MATRIX)

- One **checked-in or CI-uploaded** markdown file humans can open in a PR.
- Rows come from **existing test names + optional tags**, not a full RFC clause extraction rewrite.
- Compatible with oracle/contract culture (Pass 1): no false "≥95% MUST conformant" banner until clause IDs exist.
- Zero dependency on a new harness crate for **v0** (script over `cargo test` JSON or a static registry).

### 4.2 Artifact layout (proposed)

```
docs/validation/
  COVERAGE.md              # optional hand-maintained skeleton (PASS5 §7) — status source of truth until auto
  DISCREPANCIES.md         # seed registry (PASS5 §6) — XFAIL legends
tests/conformance/         # optional later; not required for v0
  registry.toml            # static membership: suite → surface → level → clause_ids[]
scripts/
  generate-compliance-report.py   # or .sh + jq; v0 emitter
# CI / local output (gitignored or uploaded artifact):
tests/artifacts/compliance/
  COMPLIANCE_REPORT.md     # regenerated
  results.jsonl            # optional structured sink
```

**v0 can skip** `registry.toml` and hard-code the proof-pack + extended suite list in the script (still design, not impl).

### 4.3 Registry row schema (conceptual)

```toml
[[suite]]
id = "ranking_oracle"
package = "ast-sgrep-core"
test_filter = "ranking_oracle"
surface = "S4-ranking"          # Pass 3 surface ids
pattern = "oracle"              # oracle | contract | roundtrip | metamorphic | peer-parity | process
level_default = "SHOULD"        # soft must_include ≠ formal MUST
clause_ids = ["RK-soft"]        # optional until B1 numbering lands
tags = ["proof-pack", "retrieval"]

[[suite]]
id = "machine_contracts"
package = "ast-sgrep-cli"
test_filter = "machine_contracts"
surface = "S1-machine-json"
pattern = "contract"
level_default = "MUST"
clause_ids = ["MJ-envelope", "MJ-shapes"]
tags = ["proof-pack", "machine"]

# … graph_oracle, protocol, embed_math, extraction_goldens, semantic_ivf_roundtrip,
#   parity, signal_provenance, no_embed_hit_key_parity, pattern_routing, lsp …
```

Ranking **fixture cases** (finer grain, optional v0.1):

| Case `name` (from `cases.json`) | Suggested tag | Level |
|---------------------------------|---------------|-------|
| `defs_auth_refresh` | graph+rank | SHOULD |
| `callers_process_request` | graph+rank | SHOULD |
| `literal_process_request` | lexical | SHOULD |
| `nl_auth_refresh` | hybrid-nl | SHOULD |
| `synonym_credential_renewal` | semantic | SHOULD |
| `rust_defs_auth_refresh` … `csharp_defs_AuthRefresh` | lang×defs | SHOULD |

### 4.4 Markdown table shape (skill-aligned, monorepo-pragmatic)

```markdown
# Compliance report (generated)

Generated: <ISO-8601>
Git: <sha>
Command: <exact regenerate command>
Mode: oracle/contract inventory — NOT RFC MUST-score claim

## Summary

| Surface | Suites | Cases run | Pass | Fail | Skip/XFAIL | Notes |
|---------|:------:|:---------:|:----:|:----:|:----------:|-------|
| S1 Machine JSON | 1 | 16 | … | … | 0 | machine_contracts |
| S4 Ranking soft | 1 | 12 | … | … | 0 | cases.json |
| … | | | | | | |

## Suite detail

| Suite | Package | Pattern | Default level | Tags | Status | Duration |
|-------|---------|---------|---------------|------|--------|----------|
| ranking_oracle | ast-sgrep-core | oracle | SHOULD | proof-pack | PASS/FAIL | … |
| graph_oracle | ast-sgrep-core | oracle | MUST-ish | proof-pack | … | … |
| machine_contracts | ast-sgrep-cli | contract | MUST | proof-pack | … | … |
| protocol | ast-sgrep-mcp | process | MUST | proof-pack | … | … |
| embed math:: | ast-sgrep-embed | unit | MUST | proof-pack | … | … |
| extraction_goldens | ast-sgrep-lang | peer-presence | SHOULD | lang | … | … |
| semantic_ivf_roundtrip | ast-sgrep-core | roundtrip | MUST | wire | … | … |
| parity / e2e_smoke | ast-sgrep-core | peer-parity | SHOULD | contrib-bar | … | … |
| … | | | | | | |

## Known non-claims (DISC)

| ID | Summary | Status |
|----|---------|--------|
| DISC-pattern-native-subset | Native pattern: ≠ full ast-grep | disc (no XFAIL harness yet) |
| DISC-no-jell-harness | Multi-engine Jell deferred | deferred |
| DISC-lexical-not-rg | Not byte-diff vs ripgrep | deferred |
| DISC-mcp-not-full-suite | Not official MCP compliance runner | disc |
| DISC-ranking-soft-oracle | must_include not full rank dump | disc |

## How to regenerate

```bash
# local (proposed)
python3 scripts/generate-compliance-report.py \
  --registry tests/conformance/registry.toml \
  --out tests/artifacts/compliance/COMPLIANCE_REPORT.md

# or thin wrapper around proof-pack:
bash scripts/run-proof-pack.sh --report tests/artifacts/compliance/COMPLIANCE_REPORT.md
```
```

**Score column policy (v0):** omit fake MUST% until clause IDs land (Pass 3 B1). Use **Pass/Fail/Not-run** only. When clause IDs exist, add:

`| Clause | Level | Suite | Status |` and compute Score only on rows with `level = MUST`.

### 4.5 Emitter algorithm (v0)

1. Load registry (static suite list).
2. For each suite: run `cargo test -p <pkg> --test <name> -- --test-threads=1` (or single workspace run with name filters).
3. Parse exit code (v0) or `cargo test -- -Z unstable-options --format json` when available on pinned toolchain.
4. Optionally parse ranking `cases.json` names as child rows without re-exec if parent suite passed (parent pass ⇒ all cases pass under current single-test aggregation).
5. Write markdown + optional JSONL `{suite,status,level,tags,secs}`.
6. Exit non-zero if any registered suite failed (report still written — always emit on failure for CI artifacts).

**Not in v0:** XFAIL auto from DISCREPANCIES, RequirementLevel on every `#[test]`, multi-platform matrix columns, external differential vs rg/ast-grep.

### 4.6 CI integration (design only — no YAML)

| Tier | Trigger | What runs | Report |
|------|---------|-----------|--------|
| **T0 Local default** | developer | CONTRIBUTING cheap bar | no report required |
| **T1 Proof-pack + report** | local pre-merge / optional | proof-pack commands + emitter | write `COMPLIANCE_REPORT.md` |
| **T2 PR (optional product)** | `pull_request` **if** cost-approved | forbid-soundness + cargo check **and/or** proof-pack filter + report upload | skill "every PR" only if this tier ships |
| **T3 Dispatch full** | current `ci.yml` | workspace test + clippy + (fixed) fuzz | optional report job attached |
| **T4 Release** | human + `local-release-gate.sh` **if re-wired** | fmt/clippy/workspace/fuzz | attach report to release notes optional |

**Honesty rule:** Until T2 exists, docs must say reports are **local/dispatch**, not "on every PR."

### 4.7 What a monorepo `generate_report` should **not** do

- Quote MRR/latency from benches as compliance scores (Agents.md honesty; use `baselines.md` or `UNREPRODUCIBLE`).
- Label presence-tuple extraction as full language conformance without DISC.
- Auto-update goldens (golden program owns `ASGREP_UPDATE_GOLDENS`; CI compare-only).
- Require full RFC MUST extraction before shipping v0 table of **existing suite names**.

---

## 5. Aggregated findings for beads (max 4 deep)

> **Do not file this pass.** Themes only; fold with PASS2 F6, PASS3 B5, PASS5 #4.

| # | Theme | Why deep | Suggested acceptance (later) | Explicit non-overlap |
|---|-------|----------|------------------------------|----------------------|
| **1** | **Compliance report emitter + registry over existing suites** | Skill MATRIX step is empty; proof-pack is the only bar and produces no matrix; humans cannot see suite×status without re-running cargo and memory | Land `scripts/generate-compliance-report.py` (or rust bin) + static registry of proof-pack + extended oracles; emit `tests/artifacts/compliance/COMPLIANCE_REPORT.md` (+ optional JSONL); document regenerate one-liner in `proof-pack.md` | Not golden `assert_golden`; not fuzz CI rename |
| **2** | **Proof-pack elevation: command list → runnable gate with report** | Pass 2 F6 / Pass 3 B5: best cultural hook is proof-pack, not a second "conformance crate" culture war | `scripts/run-proof-pack.sh` runs existing five cargo filters + forbid-soundness; calls report emitter; exit aggregates; link from CONTRIBUTING "merge honesty" without replacing cheap default bar | Does not force PR CI by itself |
| **3** | **CI/docs honesty: no PR report regeneration; release path ≠ local-release-gate** | Skill "every PR" fails; CONTRIBUTING overclaims PR soundness **and** release-acceptance→local-release-gate; `pi-npm-release` installs cargo-fuzz unused | (a) Fix CONTRIBUTING release-path sentence to match `release-acceptance.mjs` reality; (b) record product decision: keep dispatch-only **or** add T2 proof-pack+report job; (c) either wire `local-release-gate.sh` into a documented release prep step **or** stop claiming it is on the official path | **CONTRIBUTING PR-trigger prose + B4 PR-slice** already in **`nz7i.5`** — only add *report* + *release-path local-gate* honesty here; **fuzz bin/triggers** stay in **`b8q3.1`** |
| **4** | **Living COVERAGE rows: case/suite tags without full harness rewrite** | Without clause IDs / levels, report is a green checklist only; Pass 3 scores stay "assumed" | Add optional `level` + `clause_ids` to registry; seed ranking case tags from `name`; when DISC seed lands (PASS5 #3), print DISC section; still no ConformanceTest trait required for v0 | Clause **numbering** of QUERY_GRAMMAR / machine schema is Pass 3 B1 — can soft-depend; DISC file seed is Pass 5 #3 |

**Out of scope for these four (already owned elsewhere):**
- `ASGREP_UPDATE_GOLDENS`, `*.actual` upload, PR template → **`nz7i.5`**
- Fuzz `parsed_query` rename, dual-target release fuzz, nightly schedule → **`b8q3.1`**
- Full ConformanceTest trait + XFAIL enum (Pass 2 F1/F3) — larger than report v0
- External differential vs rg/ast-grep (Pass 4) — DISC deferred

---

## 6. Cross-links

| Program / bead | Path / id | Overlap with this pass | Action |
|----------------|-----------|------------------------|--------|
| Golden CI + SOP | `ast-sgrep-golden-artifacts-program-nz7i.5` | CONTRIBUTING CI trigger drift; optional PR contract slice; golden CI hygiene | **Cross-link only** — do not refile PR-trigger or golden SOP |
| Golden epic | `ast-sgrep-golden-artifacts-program-nz7i` | assert_golden foundation for freezes the report will *cite* | Soft-depends for richer machine dumps; report v0 works without it |
| Fuzz epic | `ast-sgrep-fuzz-program-maturity-b8q3` | Continuous fuzz CI maturity | Fuzz ≠ compliance report |
| Fuzz CI/ops child | `ast-sgrep-fuzz-program-maturity-b8q3.1` | `parsed_query` name, release-gate fuzz parity, PR/nightly fuzz | **Cross-link only** for CI trigger/fuzz truth |
| Fuzz audit PASS6 | `tests/artifacts/fuzz-audit/PASS6_SANITIZERS_PERF_CI.md` | Documents dispatch-only + G-CI-* | Cite for CI trigger facts |
| Golden audit PASS6 | `tests/artifacts/golden-audit/PASS6_CI_REVIEW_WORKFLOW.md` | CI compare-only / CONTRIBUTING drift | Cite for golden hygiene boundary |
| Conformance PASS2 F6 | proof-pack not a report | Same root cause as finding #2 | Fold |
| Conformance PASS3 B5 | oracle→matrix emitter | Same as findings #1/#4 | Fold |
| Conformance PASS5 §7 | COVERAGE skeleton | Input design for finding #4 | Fold |

---

## 7. Protect list (do not regress)

1. **Proof-pack curated list** — keep ranking_oracle / graph_oracle / machine_contracts / MCP protocol / embed math as the merge honesty bar even before a report exists.
2. **Cheap CONTRIBUTING default** — do not force full workspace + fuzz on every local change.
3. **Manual Actions minute policy** — if product keeps `workflow_dispatch`, document it; do not silently enable full matrix on every PR without cost sign-off (align `nz7i.5` B4).
4. **Bench JSON gates** — identity_ok + latency remain speed/bakeoff concerns, not substitutes for clause coverage.
5. **Pi release-contract checks** — packaging honesty is real Pattern-5 conformance; keep separate from search oracle matrix.
6. **Anti soft-skip oracle culture** — report rows must not invent green when suites were not run (`Not-run` ≠ `Pass`).

---

## 8. Report card (this pass)

| Dimension | Score (1–10) | Evidence |
|-----------|:------------:|----------|
| Compliance report exists | **0** | No COVERAGE/DISC/generate_report in tree |
| Local gate commands documented | **7** | CONTRIBUTING + proof-pack + local-release-gate clear as text |
| Local gate ↔ release path honesty | **2** | local-release-gate not on Pi official gate; CONTRIBUTING wrong |
| CI runs conformance-like suites | **3** | Only if human dispatches `build-and-test`; default none |
| CI regenerates report on every PR | **0** | No emitter; no PR workflows |
| Structured suite registry / tags | **1** | proof-pack prose only |
| Overlap hygiene with golden/fuzz beads | **8** | Explicit non-duplication of `nz7i.5` / `b8q3.1` |
| **Overall (report + CI gate maturity)** | **2** | Strong oracles, no MATRIX/MAINTAIN automation |

**Skill checklist residual for this monorepo:**

- [x] Spec surfaces identified (PASS1)
- [ ] Coverage matrix built with clause IDs (PASS3 partial estimates only)
- [ ] DISCREPANCIES.md
- [ ] Compliance report generated automatically
- [ ] **CI regenerates report on every PR**
- [ ] Fixture update SOP in CONTRIBUTING (golden program)

---

## 9. Out of scope (confirmed)

- Implementing any CI YAML or `generate-compliance-report` script
- Filing or closing beads
- Commits / branch switches
- Re-running full proof-pack or inventing pass counts (Passing columns in PASS3 remain assumed)
- Replacing fuzz or golden programs' CI designs
- Claiming product is "conformant" under skill ≥0.95 MUST rule

---

## 10. Deliverable

| Field | Value |
|-------|-------|
| **Path** | `/Users/aditya/Developer/ast-sgrep/tests/artifacts/conformance-audit/PASS6_COMPLIANCE_CI_GATE.md` |
| **Beads filed** | **0** (forbidden this pass) |
| **CI changed** | **0** |
| **Next pass hint** | PASS7 typically aggregates beads / maintain loop — use §5 four themes + non-overlap table |


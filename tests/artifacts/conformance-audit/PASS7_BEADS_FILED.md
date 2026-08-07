# Pass 7/10 — Aggregated World-Class Beads Filed

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Branch:** `perf/software-optimization`  
**Date:** 2026-08-07  
**Skill:** `testing-conformance-harnesses` (loop step: FILE beads)  
**Policy:** ONE epic + **5** children (6 issues total). No micro-finding beads. No product implementation this pass. No git commit (orchestrator).

**Inputs:** PASS1–PASS6 under `tests/artifacts/conformance-audit/`.  
**Overlap checked (cross-link only):**  
`ast-sgrep-golden-artifacts-program-nz7i` (+ `.1` assert_golden, `.4` extraction dumps, `.5` CI/CONTRIBUTING) ·  
`ast-sgrep-fuzz-program-maturity-b8q3` (+ `.1` CI/ops).

---

## 1. Filed inventory

| ID | Type | Pri | Title |
|----|------|:---:|-------|
| **`ast-sgrep-conformance-harness-program-ghiw`** | epic | P1 | Conformance harness program (specs, oracles, differentials, DISC/COVERAGE, report) |
| **`ast-sgrep-conformance-harness-program-ghiw.1`** | task | P1 | Harness shell + DISCREPANCIES.md + COVERAGE.md + XFAIL/verdict conventions |
| **`ast-sgrep-conformance-harness-program-ghiw.2`** | task | P1 | Query grammar + machine envelope MUST matrix + negative-path ledger |
| **`ast-sgrep-conformance-harness-program-ghiw.3`** | task | P1 | pattern: subset vs ast-grep differential + DISC for non-delegation |
| **`ast-sgrep-conformance-harness-program-ghiw.4`** | task | P2 | Fixture provenance + IVF/migration round-trip corpora + extraction dump path |
| **`ast-sgrep-conformance-harness-program-ghiw.5`** | task | P2 | Compliance report emitter + proof-pack runnable gate |

**Count:** 1 epic + 5 children = **6** (within anti-bloat max).

---

## 2. Dependency graph

```text
ghiw (epic)
├── ghiw.1  foundation DISC/COVERAGE/verdicts          [ready first]
│   ├── blocks → ghiw.2  MUST matrix (QG/MJ/NL)
│   ├── blocks → ghiw.3  pattern: × ast-grep differential
│   ├── blocks → ghiw.5  compliance report + proof-pack gate
│   └── related → ghiw.4 fixture provenance / RT corpora
│
ghiw.4 related → nz7i.1 (assert_golden) , nz7i.4 (extraction dumps)
ghiw.5 related → nz7i.5 (CI golden / CONTRIBUTING PR drift)
ghiw.5 related → b8q3.1 (fuzz CI bin + release-gate parity)
```

| Edge | Type | Meaning |
|------|------|---------|
| `ghiw.{1..5}` → `ghiw` | parent-child | Program children |
| `ghiw.2` → `ghiw.1` | blocks | Clause work uses DISC/COVERAGE vocabulary & paths |
| `ghiw.3` → `ghiw.1` | blocks | Pattern differential needs DISC + Skip/Not-run conventions |
| `ghiw.5` → `ghiw.1` | blocks | Report loads DISC/COVERAGE |
| `ghiw.4` → `ghiw.1` | related | Soft: cite DISC-extraction / IVF adaptive |
| `ghiw.4` → `nz7i.1`, `nz7i.4` | related | Dump pilot uses golden infra; no reimplement |
| `ghiw.5` → `nz7i.5`, `b8q3.1` | related | Cross-link only; do not own PR-trigger or fuzz bins |

**Cycles:** `br dep cycles` → none.

**Implementation order:** `.1` → (`.2` ‖ `.3`) → `.4` / `.5` (report can stub registry before full clause IDs; dump pilot waits on golden foundation).

---

## 3. Audit → child fold map

| Child | Primary audit findings folded |
|-------|-------------------------------|
| **ghiw.1** | PASS2 F1–F3 (shell, DISC/COVERAGE missing, panic-only verdicts); PASS2 F5 claim-class smell; PASS5 §6–§7 DISC seed + COVERAGE skeleton; PASS4 F2 jell/DISC honesty |
| **ghiw.2** | PASS3 B1 + S1/S2 tables + worst#2 query; PASS1 gaps #1–#2; negative ledger partial P4; PASS6 score policy (no fake MUST%) |
| **ghiw.3** | PASS4 F1 Pattern-1 highest ROI; PASS3 S3/B2/worst#1 pattern ~0.55; PASS1 gap #3; DISC-pattern-native-subset |
| **ghiw.4** | PASS5 findings #1–#2 RT corpora + PROVENANCE; PASS3 B4 migrations; PASS1 #6–#8; PASS4 F4 extraction dump-adjacent; PASS2 F4 loader fragmentation (registry not rewrite) |
| **ghiw.5** | PASS6 findings #1–#4 report+proof-pack+honesty; PASS2 F6; PASS3 B5; PASS5 COVERAGE skeleton consumer |

**Epic** carries maturity scores, program shape, epic-level acceptance, **Later phases**, and cross-program non-goals.

---

## 4. Intentionally NOT filed separately (folded / cross-linked)

### Folded into epic "Later phases" (no 7th child)

| Theme | Source | Why not a child |
|-------|--------|-----------------|
| Lexical `literal:` ⊆ ripgrep differential | PASS4 F3 | After DISC-lexical-not-rg; lower ROI than pattern: |
| Full multi-surface signal-provenance matrix | PASS3 B3 residual | Compact DISC in `.1`; extend COVERAGE in `.1`/`.2` |
| Symbol FQN / non-ASCII case-fold expansion | PASS1 gap #4 residual | DISC-casefold-ascii in `.1` is honesty first |
| Official MCP protocol compliance suite | PASS2/PASS3 S5 | DISC-mcp-not-full-suite |
| LSP method ⊆ LSP spec runner | PASS1 gap #10 | Medium risk; not worst MUST score |
| Full ConformanceTest trait rewrite of every suite | PASS2 F1 maximal | `.1` is thin shell only |
| PR-triggered CI for report ("every PR") | PASS6 skill item | Product cost; honesty with `nz7i.5`; `.5` docs T0–T4 only |

### Cross-linked (owned by other programs -- do not duplicate)

| Theme | Owner bead | How referenced |
|-------|------------|----------------|
| `assert_golden` + scrubber + `ASGREP_UPDATE_GOLDENS` | `nz7i.1` | Non-goals on epic, `.1`, `.4` |
| CLI machine hit dumps / format freezes | `nz7i.2` | Non-goal on `.2` (clause tags only) |
| Extraction full dumps (13 langs) | `nz7i.4` | `.4` soft related; design path only |
| CI golden hygiene + CONTRIBUTING PR-trigger drift + PR template | `nz7i.5` | `.5` related; no refile PR YAML claims |
| Fuzz bin rename, release-gate fuzz parity, PR/nightly fuzz | `b8q3.1` | `.5` related |
| Native language & pattern **fuzz** harnesses | `b8q3.2` | Different concern from Pattern-1 equality (`.3`) |

### Micro-findings deliberately not beadized

- Per-test panic UX nits, single missing `#[test]` rows from PASS3 tables  
- Individual IVF corrupt-case names already covered by suite inventory  
- Stale `comparison.md` sentence alone (folded into `.1` acceptance)  
- Bench / bakeoff latency gates as "conformance" (explicit anti-pattern in `.5` / Agents.md)  
- Re-publishing unreproducible parity_clean / MRR figures  

---

## 5. Child summaries (one line each)

1. **ghiw.1** — Land DISC seed (≥8 IDs), COVERAGE skeleton, verdict conventions, optional thin testkit pilot; proof-pack links.  
2. **ghiw.2** — Number QG/MJ/NL MUST/SHOULD clauses; table-drive query negatives + map machine_contracts; no fake ≥0.95 claims.  
3. **ghiw.3** — Supported-subset pattern: matrix + env-gated ast-grep match-set differential; never claim full parity.  
4. **ghiw.4** — PROVENANCE.md; IVF frames and/or pre-v7 migration DBs; extraction presence→dump design soft-dep golden.  
5. **ghiw.5** — Registry + report emitter + `run-proof-pack.sh`; T0–T4 honesty; release-path CONTRIBUTING fix (not PR-trigger ownership).  

---

## 6. Evidence commands (this pass)

```bash
# Overlap read
br show ast-sgrep-golden-artifacts-program-nz7i --json
br show ast-sgrep-golden-artifacts-program-nz7i.1 --json
br show ast-sgrep-golden-artifacts-program-nz7i.5 --json
br show ast-sgrep-fuzz-program-maturity-b8q3 --json
br show ast-sgrep-fuzz-program-maturity-b8q3.1 --json

# Create (description files under /tmp/conformance-beads/)
br create "…" -t epic -p 1 --slug conformance-harness-program --description-file epic.md --json
br create "…" -t task -p N --parent ast-sgrep-conformance-harness-program-ghiw --description-file cN.md --json

# Deps
br dep add ghiw.2 ghiw.1   # blocks (and .3, .5 similarly)
br dep add ghiw.4 nz7i.1 --type related  # + nz7i.4, and .5→nz7i.5 / b8q3.1
br dep cycles   # clean

br sync --flush-only
```

Audits read: PASS1_SPEC_SURFACE_INVENTORY through PASS6_COMPLIANCE_CI_GATE (full files under this directory).

---

## 7. Report card (this pass)

| Item | Value |
|------|--------|
| Beads filed | **6** (1 epic + 5 tasks) |
| Micro-beads | **0** |
| Product code / CI YAML implemented | **none** (file-only) |
| Git commit | **none** (orchestrator) |
| Cycles | **none** |
| Golden/fuzz duplication | **avoided** (related edges + Non-goals) |
| Deliverable | this file |

**Next skill steps (not this pass):** implement `.1` foundation first when ready; do not open all children in parallel without DISC paths.

---

## 8. Absolute ID list (copy-paste)

```
ast-sgrep-conformance-harness-program-ghiw
ast-sgrep-conformance-harness-program-ghiw.1
ast-sgrep-conformance-harness-program-ghiw.2
ast-sgrep-conformance-harness-program-ghiw.3
ast-sgrep-conformance-harness-program-ghiw.4
ast-sgrep-conformance-harness-program-ghiw.5
```

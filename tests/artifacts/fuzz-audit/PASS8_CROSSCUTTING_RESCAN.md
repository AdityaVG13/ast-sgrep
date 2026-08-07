# PASS 8 — Cross-cutting Gaps Rescan

**Workspace:** `/Users/aditya/Developer/ast-sgrep`  
**Date:** 2026-08-07  
**Mission:** Find gaps **missed by PASS1–7** on concurrency/TSan, FFI/native depth, competitor differential, stateful index sequences, CMPLOG/plateau, CVE classes, fuzz-dep isolation, Bolero/multi-engine.  
**Constraints:** No new micro-beads; no harness implementation; prefer amend over invent; max 1 new bead for whole campaign (prefer 0).

**Bead inventory under review:**

| ID | Title |
|----|-------|
| `ast-sgrep-fuzz-program-maturity-b8q3` | Epic — program maturity |
| `…-b8q3.1` | Harden existing fuzz + CI/ops |
| `…-b8q3.2` | Native language & pattern harnesses |
| `…-b8q3.3` | Semantic binary format harnesses |
| `…-b8q3.4` | Wire protocol parse harnesses |

**Prior artifacts:** `PASS1`–`PASS7` under `tests/artifacts/fuzz-audit/`.

---

## 1. Cross-cutting checklist

| # | Theme | Status | Citation / disposition |
|---|-------|--------|------------------------|
| 1 | **Concurrency / TSan** | **COVERED** (intentionally deferred) | PASS6 §3.3, G-CONCURRENCY-TSAN rank 9, P3; PASS4 WP-F + unit `kmeans_bit_identical_*` / `mr_kmeans_threads_bit_identical`; PASS7 §4 “TSan… Deferred program-wide”; child `.1` Out of scope (“MSan/TSan campaigns document only”); child `.3` OOS TSan later; **PASS8 amend epic** table restates |
| 2 | **FFI / tree-sitter native depth** | **COVERED** | Child `.2` full body: `ParserRegistry::parse`, `match_pattern`, `classify_native`, OnceLock, MSan plan note, polyglot seeds/dicts, size caps, historical CVE class; PASS1 #1–2 score 12; PASS3 §1.1–1.3 YES pure; PASS6 §4.3 OnceLock + TLS note |
| 3 | **Differential vs competitor (`ast-grep`)** | **COVERED** (intentionally deferred) | PASS4 §4: external ast-grep **gated offline only** (`ASGREP_ALLOW_AST_GREP` + abs path); process spawn kills exec/s; child `.2` OOS “External `ast-grep` subprocess”; PASS1 deprioritizes external subprocess; **in-tree** differential (ANN vs brute / RT) owned by `.3` |
| 4 | **Stateful index operation sequences** | **COVERED** (intentionally deferred L) | Epic Out of scope: full `index_content` / SQLite ingest; PASS3 NO/L; PASS4 §5 Index ops (TempDir + SQLite, slow); PASS7 §4 deferred L; child `.4` light Stateful only if pure / OOS sticky multi-turn `run_serve` |
| 5 | **Hybrid symbolic / CMPLOG plateau** | **PARTIAL → folded** | PASS5 §5 plateau ladder (stages 0–5: seeds → dict/value-profile → CMPLOG/AFL++ → Arbitrary → custom mutator → breadth); PASS7 §4 “CMPLOG / AFL++ engine switch \| docs inside harness work, **not a bead**” — but no child `done_when` previously required those docs. **Amended `.1`** (ops docs + plateau playbook). Structure-aware / mutator stages still live inside `.2`–`.4` harness design when targets ship |
| 6 | **Security CVE classes** | **COVERED** (high-ROI mapped; ReDoS deferred) | tree-sitter C grammars + dual untrusted pattern×source → `.2`; binary OOB/magic/length → `.3`; URI path escape + framing DoS → `.4`; ReDoS/`regex_pass` score 10 **explicitly deferred** PASS7 §4 (timeout-oracle heavy); FTS escape / low-score surfaces not filed (PASS1 low priority) |
| 7 | **Prod dep isolation / feature-flag leakage** | **PARTIAL → folded** | PASS2 workspace audit **PASS**: `exclude = ["fuzz"]`, no `libfuzzer`/`arbitrary`/`bolero` in product normal deps; `SECURITY.md` fuzz exclusion. Risk when children expand `fuzz/Cargo.toml` was not a regression note on any bead. **Amended `.1`**: keep fuzz-only crates out of product; cheap `cargo tree` check after dep edits. Optional slim core feature (PASS2 rec 11) remains optional |
| 8 | **Bolero / multi-engine** | **COVERED** (single stack intentional) | Live: `fuzz/Cargo.toml` = cargo-fuzz + `libfuzzer-sys` only. PASS7: CMPLOG/AFL++ not a bead. No Bolero dual harness in program. Offline engine experiments only as plateau docs under `.1` PASS8 amend |

---

## 2. Status legend applied

- **COVERED** — Fully owned by an existing bead *or* deliberately deferred with bead/PASS provenance (not a silent miss).
- **PARTIAL** — Theme exists in PASS docs / fold table but was missing from implementer-facing bead text; fixed by amend or left non-material.
- **UNCOVERED** — Material gap with no ownership and no intentional deferral.

**Material UNCOVERED after rescan:** **none.**

---

## 3. Material fold-ins applied

| Gap | Action | Target bead |
|-----|--------|-------------|
| Plateau/CMPLOG playbook not in any `done_when` / ops requirement | **`br update …-b8q3.1 --description-file`** — `## Amendment (PASS8)` ops docs + isolation note | `ast-sgrep-fuzz-program-maturity-b8q3.1` |
| Cross-cut deferrals scattered (TSan, competitor diff, stateful L, Bolero, CVE map) risk re-filing as micro-beads | **`br update …-b8q3 --description-file`** — `## Amendment (PASS8)` disposition table | `ast-sgrep-fuzz-program-maturity-b8q3` |

**New beads created:** **0**  
**Needs new bead:** **no** (inventory stays at 5)

### Non-material / do-not-file (reaffirm)

| Item | Why not a bead |
|------|----------------|
| Dedicated TSan nightly job | P3 after concurrency harness exists; unit oracles already present |
| Offline `ast-grep` subprocess differential CI | Flaky, slow, env-gated; not continuous-fuzz ROI |
| Full Indexer reindex sequence fuzz | PASS3 L effort; epic OOS |
| Bolero second harness framework | No product requirement; cargo-fuzz sufficient |
| ReDoS-only target | Deferred PASS7; reopen later if program expands |
| Per-grammar seed micro-bead | Deliverable inside `.2` |
| Feature-flag slim query/rank link only | Optional perf (PASS2 rec 11), not correctness |

---

## 4. “Checked, already covered” (≥5)

1. **Tree-sitter / FFI native harness program** — child `.2` (parse + pattern + classify, OnceLock, MSan note, CVE class).  
2. **Binary IVF/clusters/embed RT + optional in-tree differential** — child `.3` (not external competitor).  
3. **Wire LSP/MCP/CodeMode parse boundary** — child `.4`.  
4. **Broken CI bin name + seeds/dicts + size guards + query oracle + crash triage + ASan/UBSan smoke + PR/nightly skeleton** — child `.1` (pre-PASS8 body).  
5. **Workspace fuzz isolation (`exclude` + SECURITY.md) as architecture** — PASS2 PASS; epic Risk + `.1` PASS8 isolation note.  
6. **External `ast-grep` differential deferred** — `.2` Out of scope + PASS4 §4.  
7. **Full stateful index / `index_content` deferred** — epic Out of scope + PASS7 §4.  
8. **TSan concurrency campaign deferred P3** — PASS6 + PASS7 + `.1`/`.3` OOS.  
9. **Portfolio oracle strength goals + L0–L2 corpus policy** — epic Desired state / Harness design (PASS4–5 fold).  
10. **MSan plan for native C when harness lands** — `.2` desired state #6 + PASS6 §3.2 (not PR-required green).

---

## 5. Per-theme deep notes (why not invent work)

### 5.1 Concurrency / TSan

PASS4 archetype 7 is **P outside fuzz** with strong unit/MR oracles. PASS6 explicitly defers TSan until a concurrency-oriented harness exists (rayon kmeans, regex thread pool, CodeMode parallel). Filing a TSan bead now would either (a) invent a concurrent harness outside the pure-boundary program or (b) duplicate unit tests. **Correct inventory:** defer, no 6th bead.

### 5.2 FFI / tree-sitter native depth

Highest PASS1 scores already map to `.2`. Depth beyond crash (structural root, classify↔fallback consistency, polyglot seeds, MSan campaign notes, timeout for native) is in `.2` design. Reentrancy under multi-thread is product TLS (`TS_PARSERS`) + single-thread libFuzzer first (PASS6); concurrent registry races fold into deferred TSan, not a separate native gap.

### 5.3 Differential vs `ast-grep`

PASS4 ranks external differential **conditional medium–low** ROI. Product is native-first / fail-closed without external binary. In-tree differentials that *are* worth continuous fuzz (ANN full-probe vs brute, write→read RT) are already in `.3`. Competitor subprocess campaigns remain offline research — not program scope.

### 5.4 Stateful index sequences

PASS4 lists create→index→search→delete→reindex with shadow `BTreeSet` as the gold stateful model — blocked by SQLite + clock (PASS3). Epic correctly excludes full ingest. Wire-side sticky sessions are `.4` OOS. No silent miss.

### 5.5 CMPLOG / hybrid symbolic plateau

PASS5 §5 is complete as a **playbook**, not an implementation epic. PASS7 correctly refused a CMPLOG engine-switch bead. The only miss was **implementer checklist ownership** for documenting the ladder — fixed by `.1` PASS8 amend (docs-only; no tooling install requirement).

### 5.6 CVE classes

Scoring doctrine already embeds CVE surface in PASS1. High-ROI classes have homes (`.2` native C, `.3` binary parsers, `.4` path/framing). ReDoS is the main high-score deferred class — intentional (timeout oracles, store coupling for full `regex_pass`). Do not reopen as PASS8 micro-bead.

### 5.7 Feature-flag / dep leakage

Current tree is clean (PASS2). Failure mode is future accidental product dep on `libfuzzer-sys` when wiring lang/binary into fuzz. Structural preventer is workspace exclude; behavioral preventer is `.1` PASS8 regression note. Optional product feature flags for slim link are non-goals.

### 5.8 Bolero / multi-engine

Single engine (libFuzzer via cargo-fuzz) matches skill default and current `fuzz/Cargo.toml`. Multi-engine is plateau stage 2 **optional offline**, not inventory.

---

## 6. Verdict — bead inventory

| Question | Answer |
|----------|--------|
| Still **5** beads? | **Yes** — epic + `.1`–`.4` |
| Need 6th bead? | **No** |
| New beads this pass | **0** |
| Material amends | **2** (`b8q3` epic, `b8q3.1`) |
| Harnesses implemented | **0** (out of scope) |
| Production code / commits | **none** |

**Honest program shape after PASS8:** fix existing CI/ops (`.1`) → native C/polyglot (`.2`) → binary RT (`.3`) → wire parse (`.4`); cross-cuts either owned inside those children or explicitly deferred with provenance — no orphan themes left unowned.

---

## 7. Evidence commands (this pass)

```text
br show ast-sgrep-fuzz-program-maturity-b8q3 --json
br show ast-sgrep-fuzz-program-maturity-b8q3.{1,2,3,4} --json
# rg across tests/artifacts/fuzz-audit/PASS{1..7}*.md for TSan|FFI|differential|stateful|CMPLOG|CVE|Bolero|feature.?flag
br update ast-sgrep-fuzz-program-maturity-b8q3.1 --description-file …
br update ast-sgrep-fuzz-program-maturity-b8q3 --description-file …
# verified: "## Amendment (PASS8)" present on both
```

**Live stack snapshot:** `fuzz/Cargo.toml` still only `query_grammar` + `rank`; `libfuzzer-sys` only under `fuzz/`; root `exclude = ["fuzz"]`.

---

## 8. Uncovered material list

*(empty — none)*

---

*End of PASS 8. Artifact: `tests/artifacts/fuzz-audit/PASS8_CROSSCUTTING_RESCAN.md`.*

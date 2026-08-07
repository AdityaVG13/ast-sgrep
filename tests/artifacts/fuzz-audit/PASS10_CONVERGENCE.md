# PASS 10 — Absolute Convergence Rescan

**Workspace:** `/Users/aditya/Developer/ast-sgrep`  
**Date:** 2026-08-07  
**Mission:** Final (10/10) convergence of the testing-fuzzing audit. Re-run discovery + hard-rules checklist at high level. Answer: is there any **material** fuzz-program gap that is neither (a) already filed in the 5 maturity beads, nor (b) explicitly deferred with rationale in those beads / PASS docs?

**Constraints honored:** zero production code; zero harness implementation; zero commits; prefer zero `br update` (none performed); only this artifact under `tests/artifacts/fuzz-audit/`.

**Bead inventory (live, 2026-08-07):**

| ID | Title | Type | Pri | Status |
|----|-------|------|-----|--------|
| `ast-sgrep-fuzz-program-maturity-b8q3` | Fuzz program maturity epic | epic | P1 | open |
| `…-b8q3.1` | Harden existing fuzz + CI/ops pipeline | bug | P0 | open |
| `…-b8q3.2` | Native language & pattern fuzz harnesses | task | P1 | open |
| `…-b8q3.3` | Semantic binary format harnesses with strong oracles | task | P1 | open |
| `…-b8q3.4` | Wire protocol parse harnesses (LSP MCP CodeMode) | task | P2 | open |

**Fuzz beads total:** 5 (epic + `.1`–`.4`). No `.5`. No extra open issues labeled `fuzzing` outside this epic.  
**`br dep cycles`:** empty (re-verified this pass).

---

## 1. Method (what was re-checked live)

| Source | Action this pass |
|--------|------------------|
| PASS1–9 | Read inventory + fold maps + PASS8 disposition table + PASS9 scorecard |
| Live beads | `br show` epic + children; `br list` filtered for fuzz* |
| Live `fuzz/` | `Cargo.toml` bins = `query_grammar`, `rank` only; targets still thin |
| CI / gate | `ci.yml:166` still `parsed_query`; `local-release-gate.sh` still rank-only |
| Anchors | `rg` on crates for query/rank/lang/IVF/embed/LSP/MCP/CodeMode paths |
| Skill rule 3 | Re-map PASS1 top-N untrusted `&[u8]`/`&str`/`Read` scores → bead or deferral |

No new discovery pass rewrote the matrix; PASS1 remains the scored authority. This pass **re-validates ownership** and **live truth** of defects still present (expected — harness work not started).

---

## 2. Checklist re-run (must re-check 1–8)

### Check A — Skill rule 3: pub untrusted boundaries (high-score ownership)

PASS1 top scored surfaces remapped against beads / explicit deferrals:

| Score | Surface (PASS1 #) | Ownership |
|------:|-------------------|-----------|
| 12 | `ParserRegistry::parse` (#1) | **`.2`** |
| 12 | `match_pattern` (#2) | **`.2`** |
| 12 | IVF `read_header` / `map_and_parse` (#3) | **`.3`** (seam if needed) |
| 11 | `read_clusters_bounded` (#4) | **`.3`** |
| 11 | `Indexer::index_content` (#11) | **Deferred L** — epic OOS + PASS3 NO/L + PASS8 theme 4 |
| 10 | `regex_pass` / `Regex::new` (#5) | **Deferred** — PASS7 §4 (timeout-oracle heavy) + PASS8 CVE table |
| 10 | `search_pattern` orchestrator (#12) | **Not primary** — PASS3: fuzz pure lang APIs via **`.2`**; orchestrator NO/L |
| 9 | LSP `read_message` (#6) | **`.4`** |
| 9 | `classify_native` (#7) | **`.2`** |
| 9 | `try_apply_text_edit` (#8) | **`.4`** |
| 9 | MCP JSON-RPC / `handle_request` (#9) | **`.4`** parse seam; full handle OOS |
| 9 | CodeMode `run_serve` / `ServeRequest` (#10) | **`.4`** serde boundary; sticky serve OOS |
| 8 | URI helpers (#14) | **`.4`** (optional bin; done_when escape hatch) |
| 8 | N-API CodeModeSession (#22) | **Epic OOS** (prefer shared Rust, not Node process) |
| 7 | `embed_from_bytes` (#13) | **`.3`** (free RT) |
| 7 | UTF-16 / pos helpers (#15) | **`.4`** with edit harness |
| 7 | `ParsedQuery::parse` (#24) | **Existing + `.1`** harden |
| 7 | CodeMode `BatchRequest` (#31) | **`.4`** |
| 4 | `score_symbol` / `fuse_rrf` (#25) | **Existing + `.1`** guards/seeds |

**Mid/low (≤7) not filed as beads (anti-bloat, already in PASS7 §4):**  
`structural_term_signatures` / cached signatures, `IgnoreMatcher`/glob, `split_content_lines`, FTS escape, enum parsers — low ROI vs top CVE classes; reopen only if later campaign needs breadth after `.2`–`.4` ship.

**Result:** No high-score (score ≥9) untrusted boundary is **unowned**. Score-10 surfaces without beads are **explicitly deferred with rationale**.

### Check B — Existing fuzz/ hard-rules failures → all in `.1`?

Live still matches PASS2 defect set (not fixed yet — correct for pre-implementation):

| Defect | Live evidence | Bead |
|--------|---------------|------|
| D1 CI bin name `parsed_query` | `ci.yml:166` | **`.1`** done_when #1 |
| D2 no L1 seeds; corpus gitignored | no `fuzz/seed_corpus/`; `.gitignore:139` | **`.1`** |
| D3 no size/value guards | `query_grammar.rs:6–8`; `rank.rs:6` unbounded | **`.1`** |
| D4 no query dict | no `fuzz/dictionaries/` | **`.1`** |
| D5 dispatch-only + partial PR story | `ci.yml:140–141` workflow_dispatch | **`.1`** CI skeleton |
| D6 query oracle strength 1 | `let _ = ParsedQuery::parse` | **`.1`** ≥3 |
| D7 ASan-only / no UBSan campaign plan | no flags in CI/scripts | **`.1`** sanitizer smoke |
| D8 crash→regression convention | no `tests/**/fuzz*` fixtures | **`.1`** |
| Release rank-only | `local-release-gate.sh:14–17` | **`.1`** parity |
| G-CORPUS-OPS cmin/regen | no scripts | **`.1`** |
| Plateau playbook | PASS5 §5 | **`.1`** PASS8 amend |
| Prod dep isolation | workspace `exclude=["fuzz"]` PASS | **`.1`** regression note |

**Result:** All existing-harness hard-rule FAILs map to **`.1`**. None unfiled.

### Check C — Fuzzability seams → `.2`–`.4`?

| Seam class | PASS3 | Bead |
|------------|-------|------|
| Pure YES: parse / match / classify | §1.1–1.3 | **`.2`** |
| PARTIAL IVF private + mmap | §1.4–1.5 | **`.3`** seam + two-of-three done_when |
| Pure YES clusters / embed | §1.6 / §1.15 | **`.3`** |
| Pure YES LSP Cursor / edits / URI | §1.10–1.12 | **`.4`** |
| NO full MCP handle; PARTIAL JSON | §1.13 | **`.4`** parse-only seam + escape hatch |
| PARTIAL CodeMode serde YES / serve NO | §1.14 | **`.4`** |
| NO index_content / search_pattern / regex_pass full | L / NO | Epic / PASS7 deferred |

**Result:** Seam work is owned; blocked-on-seam paths have escape hatches in done_when (`.3` IVF, `.4` MCP/URI).

### Check D — Strong oracles unused → in beads?

| Free / strong oracle (PASS4) | Disposition |
|------------------------------|-------------|
| Query structural mode/terms (WP-A) | **`.1`** raise strength ≥3 |
| Rank finite + reverse-RRF (~4) | Keep in **`.1`** |
| embed LE round-trip | **`.3`** |
| clusters `write_to`↔`read_clusters_bounded` RT | **`.3`** |
| IVF save/load RT after seam | **`.3`** |
| In-tree ANN vs brute differential | **`.3`** optional; not external |
| External `ast-grep` differential | **Deferred** PASS8 (spawn kills exec/s) |
| kmeans bit-identical / TSan | **Deferred** program-wide P3; units exist |
| LSP frame RT / URI confinement | **`.4`** |

**Result:** Unused free oracles are either in `.1`/`.3`/`.4` or deferred with provenance. None require a 6th bead.

### Check E — Corpus / dict / CI / sanitizer / triage → `.1`?

| Area | Live gap? | Ownership |
|------|-----------|-----------|
| L0–L2 policy (PASS5) | design only | **`.1`** + per-target seeds in `.2`–`.4` |
| L1 seeds ≥5 / target | missing dirs | **`.1`** baseline; children for new bins |
| Dicts | missing | **`.1`** query; children for native/binary/wire |
| CI name + gate parity | broken | **`.1`** P0 (non-deferrable) |
| PR/nightly skeleton | absent | **`.1`** (dispatch-only OK if PR deferred *with comment*) |
| ASan+UBSan smoke docs | absent | **`.1`** |
| MSan for tree-sitter | plan only | **`.2`** note (not PR-green required) |
| TSan | deferred P3 | epic / PASS8 |
| Crash triage + regression path | absent | **`.1`** |
| cmin/tmin ops | absent | **`.1`** |
| Plateau CMPLOG/AFL++ | docs only | **`.1`** PASS8 amend |

**Result:** Ops surface fully folds into **`.1`** (plus target-local notes). No ops micro-bead needed.

### Check F — PASS8 cross-cuts: dispositions still hold?

| # | Theme | Live disposition | Still valid? |
|---|-------|------------------|--------------|
| 1 | Concurrency / TSan | Deferred P3; units cover kmeans bit-identical | **YES** |
| 2 | FFI / tree-sitter depth | Owned by **`.2`** | **YES** |
| 3 | External `ast-grep` differential | Deferred offline; in-tree in **`.3`** | **YES** |
| 4 | Stateful index sequences | Deferred L; epic OOS | **YES** |
| 5 | CMPLOG / plateau | Docs under **`.1`**; not a bead | **YES** |
| 6 | CVE classes | `.2` native, `.3` binary OOB, `.4` URI/frame; ReDoS deferred | **YES** |
| 7 | Fuzz-dep isolation | Architecture PASS; **`.1`** regression note | **YES** |
| 8 | Bolero / multi-engine | Single stack cargo-fuzz intentional | **YES** |

PASS8 §8 "Uncovered material list": **empty**. This pass found **no new** material uncovered theme.

### Check G — PASS9 bead quality still coherent?

| Criterion | Status |
|-----------|--------|
| Count still 5; no silent `.5` | **PASS** (`br list` fuzz filter = 5) |
| Parent-child deps only; no cycles | **PASS** |
| Priorities: `.1` P0 bug; `.2`/`.3` P1; `.4` P2; epic P1 | **PASS** |
| Soft order `.1` → (`.2` ∥ `.3`) → `.4` in descriptions | **PASS** |
| Path anchors still resolve (query/rank/lang/IVF/embed/LSP/MCP/CodeMode) | **PASS** (rg this pass) |
| PASS8 amends present on epic + all children | **PASS** (PASS9 rewrites still current `updated_at`) |
| done_when agent-checkable (`rg`, `fuzz list/run`, seed counts) | **PASS** (structure unchanged) |
| No invented perf numbers in beads | **PASS** (PASS2-only rule still stated) |
| Estimates set (30/180/300/240/180) | **PASS** |

**Result:** Quality bar from PASS9 remains intact; no description drift requiring amendment.

### Check H — Duplicate / overlapping beads?

| Pair | Overlap? | Action |
|------|----------|--------|
| `.1` vs `.2`–`.4` | Ops vs new surfaces; soft-block CI only | Keep separate |
| `.2` vs `.3` | Native C vs binary formats | Distinct crates/oracles — keep |
| `.3` vs `.4` | Binary ANN vs wire protocols | Distinct — keep |
| Epic vs children | Coordination only | Keep |
| External open issues | No other open `fuzzing`-labeled program beads | N/A |

**Result:** Report-only — **no merge required**. Portfolio shape remains anti-bloat optimal (1+4).

### Check I — Live program truth (smoke of audit claims)

| Claim | Live evidence |
|-------|---------------|
| Only two bins | `fuzz/Cargo.toml` `[[bin]]` `query_grammar`, `rank` |
| Query crash-only | `fuzz/fuzz_targets/query_grammar.rs` body = parse discard |
| Rank strong oracle | finite/range + reverse RRF in `rank.rs` |
| CI wrong name | `parsed_query` still on line 166 |
| Gate rank-only | `scripts/local-release-gate.sh:16` |
| No seed_corpus / dictionaries | dirs absent |
| Workspace exclude | root `Cargo.toml:17–19` |
| PASS1–9 artifacts present | `tests/artifacts/fuzz-audit/PASS{1..9}_*.md` |

Defects remain **open work under beads**, not unfiled discoveries.

### Check J — Anti-bloat / "would we file a 6th bead?"

Candidates re-considered and **rejected** as material unfiled gaps:

1. **ReDoS-only `regex_pass` target** — score 10 but deferred with timeout-oracle rationale (PASS7/8).  
2. **Stateful SQLite index fuzz** — L seam; epic OOS.  
3. **Bolero dual-engine** — intentional single stack.  
4. **TSan campaign bead** — P3 deferred.  
5. **N-API process fuzz** — epic OOS.  
6. **Separate bead per language seed** — deliverable inside `.2`.  
7. **CMPLOG install CI** — plateau docs only in `.1`.  
8. **exec/s CI floor assert** — P3 optional note, not a bead.  
9. **`structural_signatures` alone (score 7)** — below filing threshold vs portfolio.  
10. **Gitignore/glob alone (score 7)** — PASS7 not-filed; pure path partial.

**Result:** Prefer **zero** new beads and **zero** amendments.

---

## 3. Material unfiled gaps?

**None.**

Every material finding from PASS1–6 is either:

- owned by one of the five beads with path anchors + done_when, or  
- explicitly deferred with rationale in epic OOS / PASS7 §4 / PASS8 table.

PASS8 uncovered list remains empty. PASS9 quality still holds. Live tree has not grown new high-score pub untrusted parsers outside the scored matrix ownership map.

---

## 4. Mutations this pass

| Kind | Count | Detail |
|------|------:|--------|
| `br update` | 0 | Prefer zero |
| New beads | 0 | Prefer zero |
| Production / harness code | 0 | Forbidden |
| Commits | 0 | Forbidden |
| Artifacts | 1 | This file only |

---

## 5. Named checks that passed (≥10 required)

1. **Skill rule 3 ownership map** — all PASS1 scores ≥9 owned or explicitly deferred.  
2. **PASS2 D1–D8 + G-CI-*** all fold into **`.1`** with live anchors reconfirmed.  
3. **PASS3 pure/PARTIAL/NO seams** owned by **`.2`–`.4`** with escape hatches.  
4. **PASS4 free RT/differential oracles** owned by **`.1`/`.3`/`.4`** or deferred.  
5. **PASS5 L0–L2 + per-target budgets** owned by **`.1`** + children seed deliverables.  
6. **PASS6 sanitizer/CI/triage/ops** owned by **`.1`** (MSan note on **`.2`**).  
7. **PASS8 eight cross-cut themes** still COVERED/deferred; uncovered list empty.  
8. **PASS9 quality scorecard** still coherent (deps, priorities, anchors, amends).  
9. **Live fuzz bins** still exactly `query_grammar` + `rank` (no silent third bin drift).  
10. **CI still broken on `parsed_query`** — shipping defect remains tracked as P0 **`.1`**, not lost.  
11. **No duplicate fuzz-program open beads** outside the epic set of 5.  
12. **`br dep cycles` empty**; parent-child graph only.  
13. **Anti-bloat rejections** (ReDoS, index_content L, Bolero, TSan, N-API, per-lang seeds) still justified.  
14. **Workspace isolation** (`exclude = ["fuzz"]`) still architecture PASS; isolation regression owned by **`.1`**.  
15. **Provenance trail** PASS1–9 files present under `tests/artifacts/fuzz-audit/` for epic done_when.

---

## 6. Verdict

```
VERDICT: CONVERGED
```

No material unfiled fuzz-program gap remains after exhaustive re-check against skill rule 3, hard-rules failures, fuzzability seams, oracles, corpus/CI/ops, PASS8 cross-cuts, PASS9 bead quality, and duplicate portfolio shape. Orchestrator may mark **ZERO-CHANGE / convergence**. Next human/agent action is **implement** (claim `…-b8q3.1` first), not further audit expansion.

---

## 7. Hand-off

| Item | Value |
|------|-------|
| Artifact | `tests/artifacts/fuzz-audit/PASS10_CONVERGENCE.md` |
| Verdict | **CONVERGED** |
| Checks named | **15** |
| Mutations | **0** br / **0** beads / **1** artifact |
| Suggested next | `br update ast-sgrep-fuzz-program-maturity-b8q3.1 --claim` |

*End of PASS 10.*

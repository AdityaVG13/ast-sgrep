# PASS 9 — Bead Quality Hardening

**Workspace:** `/Users/aditya/Developer/ast-sgrep`  
**Date:** 2026-08-07  
**Mission:** Make the existing fuzz-program beads world-class (done_when, path anchors, priorities, labels, deps). No harness implementation. No new beads.

**Inventory (unchanged count = 5):**

| ID | Title | Type | Pri | Labels | Estimate (min) |
|----|-------|------|-----|--------|----------------|
| `ast-sgrep-fuzz-program-maturity-b8q3` | Fuzz program maturity epic | epic | P1 | P1, fuzzing, testing | 30 |
| `…-b8q3.1` | Harden existing fuzz + CI/ops | bug | P0 | P0, ci, fuzzing, testing | 180 |
| `…-b8q3.2` | Native language & pattern harnesses | task | P1 | P1, fuzzing, native, testing | 300 |
| `…-b8q3.3` | Semantic binary format harnesses | task | P1 | P1, binary, fuzzing, testing | 240 |
| `…-b8q3.4` | Wire protocol parse harnesses | task | P2 | P2, fuzzing, testing, wire | 180 |

**Deps:** parent-child only (children → epic). Soft order `.1` → (`.2` ∥ `.3`) → `.4`.  
**`br dep cycles`:** empty (verified PASS9).

---

## Quality bar (10 criteria)

| # | Criterion |
|---|-----------|
| C1 | Problem concrete with evidence |
| C2 | Current state path anchors exist in repo |
| C3 | Desired state testable |
| C4 | Harness design names archetype + oracle strength |
| C5 | done_when items binary and agent-checkable |
| C6 | Risk + out of scope present |
| C7 | Depends correct (children → epic; soft deps noted) |
| C8 | No invented performance numbers |
| C9 | PASS8 amendments preserved (or added where missing) |
| C10 | Description length appropriate (deep, not novel-length) |

**Score scale per criterion:** 0 = missing/wrong, 1 = partial, 2 = solid. **Max 20.**

---

## Path-anchor spot-check (live tree)

Verified 2026-08-07 against workspace:

| Claimed anchor | Live? | Notes |
|----------------|-------|-------|
| `fuzz/Cargo.toml` bins `query_grammar`, `rank` | YES | lines 14–22 |
| `fuzz/fuzz_targets/query_grammar.rs:6–8` | YES | crash-only parse |
| `fuzz/fuzz_targets/rank.rs` metamorphic | YES | lines 6–19 |
| `.github/workflows/ci.yml:164–166` `parsed_query` | YES | still wrong bin name |
| `ci.yml:140–141` workflow_dispatch | YES | |
| `scripts/local-release-gate.sh:14–17` rank only | YES | |
| `.gitignore` fuzz corpus | YES | **:139** (not 138–139); target **:105–106** |
| `CONTRIBUTING.md` release-gate / dispatch | YES | **:39–48** (was 40–45) |
| `query.rs` `ParsedQuery::parse` | YES | struct :2, parse :20 |
| `rank.rs` `fuse_rrf` / `score_symbol` | YES | :16 / :54 |
| `lang` `ParserRegistry::parse` | YES | `lib.rs:207–219` |
| `pattern.rs` match/classify | YES | :83 / :105 / :138; fallback :47 |
| `semantic_ann.rs` clusters | YES | `read_clusters_bounded` :104; `write_to` :57 |
| `semantic_ivf.rs` magic/header | YES | magic :12; `map_and_parse` :323; `read_header` :388 |
| `embed` LE codec | YES | `lib.rs:40` / `:43` |
| LSP `support.rs` framing/edit/URI | YES | :14, :16, :33, :194, :209, :277, :281 |
| MCP `handle_request` | YES | `lib.rs:153` private |
| CodeMode batch types | YES | `batch.rs:16`, :65, :85 |
| `Cargo.toml` exclude fuzz | YES | :17–19 |
| PASS1–8 artifacts | YES | under `tests/artifacts/fuzz-audit/` |

**Stale anchors fixed in PASS9 rewrites:** gitignore line range; CONTRIBUTING line range; approximate `lib.rs (~219)` → exact; binary/wire APIs without file:line → pinned; epic “children 0–3” / “child 0” → `.1`–`.4`.

---

## Scorecard before → after

### Epic `…-b8q3`

| Crit | Before | After | Notes |
|------|--------|-------|-------|
| C1 | 2 | 2 | Already strong problem statement |
| C2 | 1 | 2 | Table + corrected gitignore/CI/gate lines |
| C3 | 2 | 2 | Program outcomes clear |
| C4 | 2 | 2 | Archetype portfolio + stack pin (cargo-fuzz only) |
| C5 | 1 | 2 | Fixed “children 0–3”; agent checks (`fuzz list`, `rg parsed_query`) |
| C6 | 2 | 2 | Risk/OOS + PASS8 deferrals |
| C7 | 2 | 2 | Soft landing order documented |
| C8 | 2 | 2 | PASS2-only throughput rule |
| C9 | 2 | 2 | PASS8 table preserved + PASS9 zero-new-beads note |
| C10 | 2 | 2 | ~8.1k chars, structured |
| **Σ** | **18** | **20** | **Updated** |

### Child `…-b8q3.1` (CI/ops harden)

| Crit | Before | After | Notes |
|------|--------|-------|-------|
| C1 | 2 | 2 | Broken CI + hollow ops |
| C2 | 1 | 2 | Fixed gitignore/CONTRIBUTING; pinned query/rank APIs |
| C3 | 2 | 2 | CI/gate/guards/seeds/oracles/ops |
| C4 | 2 | 2 | Crash + invariant/metamorphic |
| C5 | 1 | 2 | Explicit `rg` / smoke / seed counts; plateau + isolation in done_when |
| C6 | 2 | 2 | |
| C7 | 2 | 2 | Soft-blocks siblings |
| C8 | 2 | 2 | PASS2-only |
| C9 | 2 | 2 | PASS8 plateau + dep isolation preserved |
| C10 | 2 | 2 | ~8.2k |
| **Σ** | **18** | **20** | **Updated** |

### Child `…-b8q3.2` (native)

| Crit | Before | After | Notes |
|------|--------|-------|-------|
| C1 | 2 | 2 | Top-ROI CVE class named |
| C2 | 1 | 2 | Exact `lib.rs` / `pattern.rs` lines |
| C3 | 2 | 2 | |
| C4 | 2 | 2 | |
| C5 | 1 | 2 | OnceLock checkable; smoke; seed counts; CVE note |
| C6 | 2 | 2 | |
| C7 | 1 | 2 | Soft-dep `.1` explicit; independent of `.3`/`.4` |
| C8 | 2 | 2 | |
| C9 | 0 | 2 | **Added** PASS8 ownership amend (was epic-only) |
| C10 | 2 | 2 | ~6.1k |
| **Σ** | **16** | **20** | **Updated** |

### Child `…-b8q3.3` (binary)

| Crit | Before | After | Notes |
|------|--------|-------|-------|
| C1 | 2 | 2 | |
| C2 | 1 | 2 | Pinned `semantic_ann` / `semantic_ivf` / embed lines |
| C3 | 2 | 2 | |
| C4 | 2 | 2 | RT + custom mutator |
| C5 | 1 | 2 | Two-of-three rule; blocked-on-seam; no “crash-only only” |
| C6 | 2 | 2 | |
| C7 | 1 | 2 | Soft-dep `.1`; independent of native/wire |
| C8 | 2 | 2 | |
| C9 | 0 | 2 | **Added** PASS8 binary OOB amend |
| C10 | 2 | 2 | ~5.6k |
| **Σ** | **16** | **20** | **Updated** |

### Child `…-b8q3.4` (wire)

| Crit | Before | After | Notes |
|------|--------|-------|-------|
| C1 | 2 | 2 | |
| C2 | 1 | 2 | Pinned LSP/MCP/CodeMode lines |
| C3 | 2 | 2 | |
| C4 | 2 | 2 | |
| C5 | 1 | 2 | Conditional URI done_when; MCP seam escape hatch |
| C6 | 2 | 2 | |
| C7 | 1 | 2 | P2 soft-dep `.1`; independent of `.2`/`.3` |
| C8 | 2 | 2 | |
| C9 | 0 | 2 | **Added** PASS8 URI/framing amend |
| C10 | 2 | 2 | ~5.7k |
| **Σ** | **16** | **20** | **Updated** |

### Portfolio summary

| Bead | Before Σ | After Σ | Action |
|------|----------|---------|--------|
| epic `b8q3` | 18 | 20 | description rewrite |
| `.1` | 18 | 20 | description rewrite |
| `.2` | 16 | 20 | description rewrite + PASS8 amend |
| `.3` | 16 | 20 | description rewrite + PASS8 amend |
| `.4` | 16 | 20 | description rewrite + PASS8 amend |
| **Mean** | **16.8** | **20.0** | all 5 updated |

**New beads created:** 0  
**Production code changed:** 0  
**Commits:** 0 (per mission constraints)

---

## Material improvements applied (all beads)

1. **Agent-checkable done_when** — concrete commands (`rg`, `cargo +nightly fuzz list/run`, seed file counts, OnceLock/guards greppable).
2. **Path anchors re-verified** — corrected gitignore/CONTRIBUTING ranges; pinned API file:line for query/rank/lang/IVF/embed/LSP/MCP/CodeMode.
3. **Child naming consistency** — “child 0” / “children 0–3” → `.1`–`.4`.
4. **PASS8 on all children** — epic + `.1` already had amends; `.2`–`.4` gained ownership/CVE disposition blocks so implementers do not re-file micro-beads.
5. **Soft dependency clarity** — `.1` soft-blocks CI registration; `.2`/`.3`/`.4` independent for local merge; `.4` remains P2.
6. **Estimates set** — 30 / 180 / 300 / 240 / 180 minutes.
7. **Priorities/labels unchanged** (already correct: P0 bug for CI break; P1 native/binary; P2 wire).

---

## Constraints checklist

| Constraint | Status |
|------------|--------|
| No new beads | PASS (0) |
| No production code | PASS |
| No commits | PASS |
| Prefer `br update` | PASS (all 5 via `--description-file`) |
| `br dep cycles` empty | PASS |
| `br sync --flush-only` at end | see session end |
| PASS8 substance preserved | PASS (epic + `.1` full text retained; children extended) |
| No invented perf numbers | PASS |

---

## Hand-off

- **Updated:** all five beads (descriptions + estimates).
- **Already excellent (no rewrite needed for labels/pri/deps graph):** priority ladder P0→P1→P2; parent-child only; label sets.
- **Scorecard path:** `tests/artifacts/fuzz-audit/PASS9_BEAD_QUALITY.md`
- **Next work (not this pass):** implement `.1` first (CI bin rename + gate parity), then harnesses.

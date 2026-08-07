# PASS 7 — Bead Aggregation (World-Class, Anti-Bloat)

**Workspace:** `/Users/aditya/Developer/ast-sgrep`  
**Date:** 2026-08-07  
**Mission:** Collapse PASS1–6 findings into **exactly 1 epic + 4 children** (5 issues total). No harness implementation. No one-bead-per-nit.  
**Search first:** `br search fuzz` / `fuzzing` — no pre-existing open fuzz-program beads (only unrelated golden-artifacts hits on "PASS"). Fresh epic filed.

---

## 1. Bead set (created)

| ID | Title | Type | Priority | Labels |
|----|-------|------|----------|--------|
| `ast-sgrep-fuzz-program-maturity-b8q3` | Fuzz program maturity: from two thin targets to oracle-strong continuous fuzzing | epic | **P1** | fuzzing, testing, P1 |
| `ast-sgrep-fuzz-program-maturity-b8q3.1` | Harden existing fuzz + CI/ops pipeline | bug | **P0** | fuzzing, testing, P0, ci |
| `ast-sgrep-fuzz-program-maturity-b8q3.2` | Native language & pattern fuzz harnesses | task | **P1** | fuzzing, testing, P1, native |
| `ast-sgrep-fuzz-program-maturity-b8q3.3` | Semantic binary format harnesses with strong oracles | task | **P1** | fuzzing, testing, P1, binary |
| `ast-sgrep-fuzz-program-maturity-b8q3.4` | Wire protocol parse harnesses (LSP MCP CodeMode) | task | **P2** | fuzzing, testing, P2, wire |

**Shape:** 1 + 4 = 5 (within hard max 6; preferred size).  
**Why not 3 children?** Wire (P2) is a distinct boundary class (editor/agent protocols) with different crates, size budgets, and seam rules from native C grammars and binary ANN/IVF. Folding wire into "binary" or "native" would produce unimplementable mega-beads.  
**Why not 5 children?** CI/ops + query oracle + seeds/dicts + sanitizers + crash triage + cmin are **one** shipping-defect surface on *existing* bins (child `.1`).

---

## 2. Dependency graph

```
ast-sgrep-fuzz-program-maturity-b8q3          (epic, P1)
├── .1 Harden existing fuzz + CI/ops          (bug, P0)   parent-child
├── .2 Native language & pattern harnesses    (task, P1)  parent-child
├── .3 Semantic binary format harnesses       (task, P1)  parent-child
└── .4 Wire protocol parse harnesses          (task, P2)  parent-child
```

**Edges:** All children have `dependency_type: parent-child` on the epic (verified via `br show`).  
**No hard sibling deps:** `.2`/`.3`/`.4` may merge harnesses before CI lists them; `.1` should land first for *truthful* CI/gate but does not block local target work. Soft order in descriptions: prefer `.1` → then expand bins into dispatch matrix.  
**Cycles:** none (`br dep cycles` after create).

---

## 3. Fold map (PASS micro-findings → beads)

### Epic `…-b8q3` — program coordination

| Folded in | Source |
|-----------|--------|
| Overall "two thin targets → continuous oracle-strong fuzz" narrative | PASS1–6 roll-ups |
| Portfolio oracle strength goals | PASS4 WP-A–F, §10 |
| L0–L2 corpus policy at program level | PASS5 §0, §4 |
| Explicit non-goals: index_content L, N-API, network embed, replace ranking goldens | PASS1 §5, PASS3, PASS4 |

### Child `.1` — Harden existing + CI/ops (P0)

| Micro-finding / Gap ID | Source |
|------------------------|--------|
| **G-CI-NAME / D1** `parsed_query` → `query_grammar` | PASS2 D1, PASS6 §2 |
| **G-CI-TRIGGERS** dispatch-only; PR/nightly skeleton | PASS2 D5, PASS6 §6 |
| Release gate rank-only parity | PASS6 §1.2, gap table |
| **D2** no seed corpus; regenerable policy without regenerator | PASS2, PASS5 §0 |
| **D3** size/value guards (query + rank ranks vec) | PASS2, PASS5 §2 |
| **D4** query mode dictionary | PASS2, PASS5 query dict |
| **D6** query_grammar oracle strength 1 → ≥3 | PASS2, PASS4 WP-A |
| **D7 / ASan+UBSan** smoke + campaign env | PASS2, PASS6 §3 |
| **D8** crash → tmin → regression fixture convention | PASS2, PASS6 §5 |
| **G-CORPUS-OPS** cmin/regen docs/scripts | PASS5 §4, PASS6 G-CORPUS-OPS |
| CONTRIBUTING stale PR claims (docs fix alongside CI) | PASS6 §1.3 |
| Rank keep strong oracle; no rewrite | PASS2 grade C+, PASS4 |
| Optional exec/s floor assert (P3 note only) | PASS6 gap table |

### Child `.2` — Native lang & pattern (P1)

| Micro-finding | Source |
|---------------|--------|
| `ParserRegistry::parse` score 12 | PASS1 #1 |
| `match_pattern` score 12 | PASS1 #2 |
| `classify_native` score 9 | PASS1 #7 |
| Fuzz pure lang APIs not `search_pattern` orchestrator | PASS3 §1.9 |
| OnceLock registry init outside body | PASS3 §1.1, PASS6 §4.3 |
| Polyglot seeds + `tree_sitter_source.dict` + pattern tokens | PASS5 §2 |
| Size budgets 4–64 KiB source / pattern 256 | PASS5, PASS6 §4.2 |
| MSan plan note for tree-sitter C (not PR-required) | PASS6 §3.2, PASS3 |
| Optional classify ↔ fallback consistency oracle | PASS3 §1.3 |
| **G-NATIVE-HARNESS-OPS** | PASS6 gap rank 8 |

### Child `.3` — Semantic binary + strong oracles (P1)

| Micro-finding | Source |
|---------------|--------|
| IVF load score 12; `read_clusters_bounded` 11 | PASS1 #3–4 |
| `embed_from_bytes` score 7 + free RT | PASS1 #13, PASS4 |
| Pure YES on clusters/embed; IVF PARTIAL → seam | PASS3 §1.4–1.6, §1.15 |
| Port unit RT / differential into fuzz where pure | PASS4 free oracles, WP-C/D |
| Cluster k/dim/chunk caps; IVF image ≤128 KiB | PASS5 §2 |
| Prefer parser-of-bytes over fuzzing `map_readonly` alone | PASS1 #23 |
| Custom mutator / magic dict `ASIVF\0` | PASS1 archetype, PASS5 |

### Child `.4` — Wire protocols (P2)

| Micro-finding | Source |
|---------------|--------|
| LSP `read_message` / edits / URI scores 8–9 | PASS1 #6–8, #14–15 |
| MCP JSON-RPC score 9; full handle NO → parse seam | PASS1 #9, PASS3 §1.13 |
| CodeMode ServeRequest/BatchRequest serde YES | PASS1 #10/#31, PASS3 §1.14 |
| Body/line size << product max (64 KiB / 8 KiB) | PASS5 LSP/MCP/CodeMode |
| Confinement oracle on URI | PASS3 §1.12 |
| Dict: Content-Length, JSON methods, tool names | PASS5 |

---

## 4. NOT filed as beads (nits folded or deferred)

Per anti-bloat law — **do not** create standalone issues for:

| Nit | Disposition |
|-----|-------------|
| Size guards alone | Folded → `.1` (existing) and per-target in `.2`–`.4` |
| Dict tokens / mode prefixes alone | Folded → `.1` + PASS5 recipes in children |
| Rename CI string alone (`parsed_query`) | Folded → `.1` P0 (not its own bead) |
| OnceCell/OnceLock alone | Folded → `.2` harness design |
| cmin/tmin scripts alone | Folded → `.1` ops |
| CONTRIBUTING stale sentence alone | Folded → `.1` docs |
| UBSan flag env alone | Folded → `.1` sanitizer smoke |
| RSS limit `-rss_limit_mb` | Folded → `.1` / perf notes |
| exec/s CI floor assert | Explicitly **not** a bead; P3 note in PASS6 / `.1` out-of-scope optional |
| TSan concurrency campaign | Deferred program-wide (PASS6 P3); unit kmeans oracles already exist (PASS4) — mention only in epic/out-of-scope |
| `Indexer::index_content` full ingest fuzz | Deferred L (PASS3); epic out-of-scope |
| `regex_pass` / ReDoS dedicated target | Folded as optional future under epic non-goals for this 5-bead set (score 10 but timeout-oracle heavy; can reopen later) |
| N-API / Node process fuzz | Epic out-of-scope |
| `split_content_lines`, enum parsers, FTS escape, gitignore | PASS1 low-priority; not filed |
| `map_readonly` isolation | Prefer IVF/clusters in `.3` |
| Separate bead per language seed file | Seeds are deliverables inside `.2` |
| Separate bead for MCP seam vs CodeMode serde | One wire child `.4` |
| Separate bead for embed vs clusters vs IVF | One binary child `.3` |
| CMPLOG / AFL++ engine switch | PASS5 plateau step; docs inside harness work, not a bead |
| Crash artifact upload YAML alone | Folded → `.1` CI skeleton |

---

## 5. Priority rationale

| Bead | Priority | Why |
|------|----------|-----|
| Epic | P1 | Program coordination; not a shipping break by itself |
| `.1` | **P0** | Broken CI bin name + gate lies about query coverage = shipping defect on *claimed* fuzz |
| `.2` | P1 | Highest ROI new pure surface (native CVE class) |
| `.3` | P1 | Custom binary + free RT oracles; pure clusters today |
| `.4` | P2 | Important wire surface; after native/binary investment order (PASS1 §8) |

---

## 6. Evidence index (inputs read)

| Artifact | Role |
|----------|------|
| `tests/artifacts/fuzz-audit/PASS1_TARGET_DISCOVERY.md` | Scored matrix, top-N, gaps |
| `tests/artifacts/fuzz-audit/PASS2_HARNESS_HARD_RULES_AUDIT.md` | D1–D9, grades, hard rules |
| `tests/artifacts/fuzz-audit/PASS3_FUZZABILITY_GAPS.md` | YES/PARTIAL/NO, seams |
| `tests/artifacts/fuzz-audit/PASS4_ORACLE_ARCHETYPE_COVERAGE.md` | Oracle strengths, free RTs |
| `tests/artifacts/fuzz-audit/PASS5_CORPUS_DICT_STRUCTURE.md` | Seeds, dicts, budgets, L0–L2 |
| `tests/artifacts/fuzz-audit/PASS6_SANITIZERS_PERF_CI.md` | CI tiers, sanitizers, triage |
| `fuzz/Cargo.toml`, `fuzz/fuzz_targets/*.rs` | Live bins |
| `.github/workflows/ci.yml` ~140–170 | Broken `parsed_query` |
| `scripts/local-release-gate.sh` | Rank-only |

---

## 7. Constraints honored

- Created **5** beads (≤6); no production source/workflow edits.
- Descriptions use required sections: Problem, Current state, Desired state, Harness design, done_when, Evidence, Risk, Out of scope, Depends on.
- No commit (orchestrator).
- `br sync --flush-only` after creates (see session report).
- Pre-search found no duplicate fuzz-program open beads.

---

## 8. Implementer quickstart (next agents)

```bash
# Claim order suggestion
br update ast-sgrep-fuzz-program-maturity-b8q3.1 --claim   # P0 first
# then either:
br update ast-sgrep-fuzz-program-maturity-b8q3.2 --claim   # native
# or
br update ast-sgrep-fuzz-program-maturity-b8q3.3 --claim   # binary pure
# wire last
br update ast-sgrep-fuzz-program-maturity-b8q3.4 --claim
```

Each bead body is self-contained; cite PASS paths for deep detail only.

---

*End of PASS 7. Artifact: `tests/artifacts/fuzz-audit/PASS7_BEAD_AGGREGATION.md`.*

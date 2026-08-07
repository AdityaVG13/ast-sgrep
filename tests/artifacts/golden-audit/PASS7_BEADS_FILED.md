# Pass 7 — Aggregated beads filed

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Branch:** `perf/software-optimization` (beads + this index only; no product implementation, no git commit by this pass)  
**Date:** 2026-08-07  
**Skill:** `testing-golden-artifacts`  
**Sources:** PASS1–PASS6 under `tests/artifacts/golden-audit/`  
**Policy:** one epic + 5 deep children (max 7 including epic); no micro-finding beads.

---

## Epic

| ID | Title | Pri | Type |
|----|-------|-----|------|
| **`ast-sgrep-golden-artifacts-program-nz7i`** | Golden artifact testing program (infra + freezes + CI) | P2 | epic |

One-line: Coherent program spanning testkit compare/update + scrubbers, high-value freezes, and CI/SOP hygiene; aggregates PASS1–6 audits.

---

## Children

| ID | Title | Pri | One-line |
|----|-------|-----|----------|
| **`ast-sgrep-golden-artifacts-program-nz7i.1`** | testkit assert_golden + Scrubber registry + ASGREP_UPDATE_GOLDENS | **P1** | Foundation: custom `assert_golden`, scrub presets, chain canonicalize, PROVENANCE, migrate one existing CLI fixture (PASS2 P0/P1 + PASS5 B1/B4). |
| **`ast-sgrep-golden-artifacts-program-nz7i.2`** | CLI machine golden expansion: hits, formats, teaching, capsule_format | **P1** | Freeze agent/agent-capsule/compact hit dumps; native/github/gitlab shapes; teaching messages; capsule_format file goldens (PASS3 F1–F5 + PASS5 B3). |
| **`ast-sgrep-golden-artifacts-program-nz7i.3`** | Agent/protocol dump goldens: handbook, MCP tools/list, codemode catalog | **P2** | Exact handbook body; full MCP tools/list schemas; codemode ToolDef (+ adapters) freezes (PASS4 G2/G4/G16–G17, PASS3 F6). |
| **`ast-sgrep-golden-artifacts-program-nz7i.4`** | Lang extraction full dumps (13) + chain expand machine JSON | **P2** | Canonicalized ExtractionResult goldens per lang + sorted chain expand page (PASS4 G1/G10 + PASS5 §4.2/B4 consumer). |
| **`ast-sgrep-golden-artifacts-program-nz7i.5`** | CI golden hygiene + update SOP + CONTRIBUTING drift + PR template | **P2** | `ASGREP_UPDATE_GOLDENS=0` + `*.actual` upload; in-tree SOP; CONTRIBUTING accuracy; PR template; B4 decision note (PASS6 B1–B4). |

---

## Dependency graph

```
epic nz7i
├── .1 foundation (assert_golden + Scrubber + chain canonicalize)   [ready]
│     ├── blocks → .2 CLI machine expansion
│     ├── blocks → .3 agent/protocol dumps
│     └── blocks → .4 extraction + chain dumps
└── .5 CI + SOP (+ soft-depends .1 for meaningful dumps; no hard block)  [ready]
```

### Edges (br)

| Issue | Depends on | Type |
|-------|------------|------|
| `.1` … `.5` | `nz7i` | parent-child (via `--parent`) |
| `.2` | `.1` | blocks |
| `.3` | `.1` | blocks |
| `.4` | `.1` | blocks |
| `.5` | — (soft: `.1`) | no hard blocks edge |

`br dep cycles` → none.  
`br ready` after filing includes **`.1`** and **`.5`** (freeze children blocked until foundation closes).

---

## Aggregation map (what was NOT filed separately)

| Audit item | Merged into |
|------------|-------------|
| PASS2 P0 assert_golden + update + `*.actual` UX | **`.1`** |
| PASS2 P0 scrubber registry | **`.1`** |
| PASS2 P1 unified diff in panic | **`.1`** |
| PASS2 P1 `tests/golden/` + PROVENANCE | **`.1`** |
| PASS2 P3 cross-platform path canonicalize helper | **`.1`** (standard scrub rules R5–R8) |
| PASS5 B1 Scrubber presets | **`.1`** |
| PASS5 B2 doctor/status/search_dump path+TTY+cache recipes | **`.1`** (presets) + wired by **`.2`/later freezes** |
| PASS5 B4 chain/set array canonicalize helper | **`.1`** (helper) + **`.4`** (consumer) |
| PASS5 B3 stop over-scrubbing usage messages | **`.2`** (with F4) |
| PASS3 F1 hit payloads | **`.2`** |
| PASS3 F2 format matrix native/github/gitlab | **`.2`** |
| PASS3 F3 capsule_format file goldens | **`.2`** |
| PASS3 F4 teaching/usage goldens | **`.2`** |
| PASS3 F5 pretty envelopes + compare UX | **`.1`** stretch / **`.2`** migration (not separate bead) |
| PASS3 F6 robot-docs handbook | **`.3`** |
| PASS4 G2 MCP tools/list | **`.3`** |
| PASS4 G4 handbook | **`.3`** |
| PASS4 G16–G17 codemode catalog/adapters | **`.3`** |
| PASS4 G1 extraction dumps | **`.4`** |
| PASS4 G10 chain expand | **`.4`** |
| PASS6 B1 CI no-update + upload | **`.5`** |
| PASS6 B2 written SOP | **`.5`** |
| PASS6 B3 PR template | **`.5`** |
| PASS6 B4 optional PR-path contract slice | **`.5`** (decision record only, not a 6th bead) |

### Intentionally not filed (policy / later / anti-golden)

- Per-language micro-beads for 13 extract freezes  
- Separate beads for status/doctor value goldens (PASS4 G8/G9) — epic “later phase”; scrub presets live in `.1`  
- MCP tool **call** body dumps (PASS4 G3), LSP dumps (G14/G15), Pi TS schemas (G19)  
- Ranking full ordered lists / checked-in eval gold (PASS1 #8, PASS4 G13)  
- insta adoption; baselines-as-CI-goldens; CI auto-update  
- Bench wall-time freezes; embed-on dumps; cascade timing reports  

---

## Ready order (implementation)

1. **`.1` foundation** (P1) — unblocks freezes  
2. **`.5` CI/SOP** can parallel with `.1` (env guard + docs)  
3. **`.2` CLI freezes** (P1) after `.1`  
4. **`.3` agent dumps** (P2) after `.1`  
5. **`.4` extract+chain** (P2) after `.1` (largest review load)

---

## Sync / commit

- `br sync --flush-only` run after create (this pass).  
- **No git commit** by Pass 7 agent (orchestrator owns commits).  
- **No product code** in this pass.

---

## Method

- Read PASS1–PASS6 fully (headings + findings/design sections).  
- Aggregated per mission map (5 children, not 6).  
- Descriptions use Goal / Why / Acceptance / Non-goals / Context / Depends / Risk / Provenance.  
- Created via `br create --description-file` + `--parent` + `br dep add` blocks edges.

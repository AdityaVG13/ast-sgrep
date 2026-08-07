# Pass 1/16 — Project Class + Multi-Reference Pin Inventory

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Branch:** `perf/software-optimization` (no switch)  
**Date:** 2026-08-07  
**Skill:** `running-the-gauntlet-on-your-rust-port`  
**Mode constraint:** audit-only (no product code, no beads, no commit)  
**Hard rule:** read/rg only; no workspace `cargo test` / full build / full bench  

---

## 0. Executive summary

| Field | Value |
|-------|--------|
| **Recommended mode** | `audit-only` |
| **Recommended tier** | **T3 — Workspace** |
| **Project class (skill map)** | **Greenfield-Rust-class** with **multi-reference External-tool oracles** (hybrid multi-oracle) |
| **Single-port?** | **No** — not FrankenSQLite-class 1:1 differential |
| **Product shape** | Hybrid code search: lexical + AST/graph + semantic + agent surfaces |
| **Rust surface (approx.)** | ~37k LOC across 11 workspace crates (`find crates -name '*.rs' \| xargs wc`) |
| **Workspace version** | `1.4.0` (`Cargo.toml` workspace.package) |

**One-line class statement:** ast-sgrep is a **greenfield multi-reference hybrid product**, not a port of a single upstream. Oracles are **composed** (spec/fixture/self/round-trip + optional external tools for *subset* behaviors), never a single canonical engine identity.

---

## 1. Recommended gauntlet mode + tier

### 1.1 Mode: `audit-only` (justified)

| Criterion | This run |
|-----------|----------|
| User/orchestrator intent | Report + pin inventory + gap questions; **do not implement** product code |
| Skill mode table | `audit-only` → phases **0–9** style (recon, contracts, ledgers, baseline inventory); **not** remediation/beads/cert ship |
| Product maturity | Existing multi-crate product with in-tree oracles, benches, validation docs — not a first-time port scaffold |
| Cargo constraint | Explicit ban on full workspace test/build/bench; audit does not need Phase 11 convergence loop |
| Mode router | "Existing port; want a report + plan, not code changes" → `audit-only` |
| Explicit non-modes | Not `gauntlet-full` (no ≥10-round remediation), not `gauntlet-greenfield` workspace bootstrap (class is greenfield-shaped but mode is audit), not `harden-pillar` (no single pillar regression charter) |

**Exit criteria for this campaign (orchestrator):** 16 condensed audit passes under `tests/artifacts/gauntlet-audit/`; beads only in later aggregation pass (mission 11); two consecutive ZERO-CHANGE stops. This Pass 1 seeds class + pins only.

### 1.2 Tier: T3 — Workspace

| Rubric signal | Evidence |
|---------------|----------|
| LOC band 20k–200k | ~37k Rust LOC in `crates/` |
| Multi-crate | 11 members: core, cli, lang, embed, lsp, mcp, plugins, testkit, mmap, codemode, codemode-napi; `fuzz/` excluded |
| Multi-surface product | CLI + MCP + LSP + Pi package + Code Mode |
| Multi-oracle domains | Lexical / structural / graph / semantic / machine JSON / index durability |
| Complexity overlays | Dual-mode (indexed hybrid vs optional external structural tool); ANN/IVF; process supervisor; agent formats — document, do **not** auto-bump to T4 for audit-only |
| Skill examples | Aligns with T3 examples (`fastmcp_rust`, workspace ports), not T1/T2 single-crate, not T4 platform-scale FrankenSQLite |

**Tier implication for later passes:** squad-scale thinking for pillar inventories is fine; full swarm + multi-day soak is out of scope for `audit-only`.

---

## 2. Project class detection

### 2.1 Skill class router (what it is not)

| Skill class | Match? | Why |
|-------------|:------:|-----|
| SQL-class | No | Uses SQLite as **storage** for `.asgrep/index.db`; product is not a SQLite engine port; no `rusqlite` oracle differential vs C-SQLite semantics as the product claim |
| RESP-class | No | Not a Redis/RESP protocol port |
| Numerical-Python-class | No | No NumPy/pandas surface parity |
| ML-System-class | No | Embeddings exist, but product is not a PyTorch/JAX/Whisper port; no ULP matmul certification surface |
| HTTP-Protocol-class | No | MCP/LSP are product surfaces with **own** protocol tests, not FastAPI/FastMCP framework ports |

### 2.2 Positive classification

**Primary:** **Greenfield-Rust-class**  
(`PROJECT-CLASSES.md` § Greenfield-Rust-class; `methodology/GREENFIELD-ADAPTATION.md`)

| Greenfield trait | ast-sgrep instantiation |
|------------------|-------------------------|
| No single upstream reference | Explicit product positioning: complements ripgrep + ast-grep; does not replace either (`docs/comparison.md`, README) |
| Spec / docs as oracle | Query grammar, machine JSON schema, feature universe, engine identity, negative ledgers under `docs/` + `docs/validation/` |
| Property / metamorphic | In-tree metamorphic tests; ranking soft oracles; IVF CE-003 vs brute force |
| Self-oracle | Peer HitKey parity (CLI/core/LSP); determinism loops; prior-commit bench ratchet intent (honestly incomplete — later passes) |
| Round-trip | IVF/index sidecars, schema migrations (`user_version`), machine envelope scrub/goldens |
| External-tool oracle | **Optional, partial-domain:** `rg` (lexical timing + future match-set), `ast-grep`/`sg` (structural **subset**), historical `semgrep` bake-off; tree-sitter **library** is the parse substrate, not a second product |

**Skill mapping label for this campaign:**

> **`greenfield multi-reference hybrid`**  
> (Greenfield-Rust-class + multi External-tool oracles; **not** a new sixth FrankenSuite port class)

`scripts/detect-project-class.sh` (if run) would be expected to return **UNKNOWN** / greenfield path — no SQL/RESP/numpy/torch/HTTP signature as the primary product.

### 2.3 Hybrid multi-oracle model (Subject / Oracle / Comparator)

```
Subject  = asgrep / ast-sgrep workspace @ HEAD (v1.4.0)
Oracle   = composite, scenario-dispatched:
             Spec | Fixture ranking/graph | Peer surface | Math (IVF brute) |
             Prior-commit self | External tool (rg / ast-grep subset) | External tool (miri/clippy/forbid)
Comparator = scenario-specific:
             HitKey set equality | soft must_include+max_rank | recall@k SLO |
             byte identity (sidecars) | latency ledger (NOT correctness) |
             machine JSON scrubbed equality
```

**Engine identity (product, not FrankenSQL):** `docs/validation/engine-identity.md` — tool=`asgrep`, schema_version, embed_backend, index_format. Distinct from any external competitor identity.

---

## 3. Reference pin table (what can be pinned today)

Host probes and lockfile reads: **2026-08-07**, machine under this audit. No `cargo` builds.

### 3.1 Host binaries (PATH)

| Tool | On PATH? | Version (this host) | Gauntlet role | Product dependency? |
|------|:--------:|---------------------|---------------|:-------------------:|
| **ripgrep** (`rg`) | Yes | **15.1.0** (rev `48a6ad93f1`) | Lexical latency competitor; candidate match-set oracle for `literal:`/`word:`-like modes | No (optional) |
| **ast-grep** / **sg** | Yes | **0.45.0** (Homebrew) | Structural latency competitor; candidate match-set oracle for **supported pattern subset** | No for search path (native patterns; spawn only under trusted env — see env-trust / structural-patterns) |
| **semgrep** | Yes | **1.172.0** | Historical quality bake-off competitor only | No |
| **hyperfine** | Yes | **1.20.0** | Latency driver (`scripts/run-benchmarks.sh`) | No |
| **tree-sitter** CLI | **No** | — | Potential AST dump / query oracle if installed later | N/A |
| **asgrep** (subject) | Built artifact | `target/debug/asgrep` present; release-perf not required for this pass | DUT | Yes |

### 3.2 In-tree library pins (always available)

| Surface | Pin | Source |
|---------|-----|--------|
| **tree-sitter** (Rust crate) | **0.26.10** | `Cargo.lock` (`name = "tree-sitter"`) |
| workspace dep range | `tree-sitter = "0.26"` | root `Cargo.toml` |
| tree-sitter-rust | 0.24.x family (workspace) | `Cargo.toml` / lock |
| tree-sitter-python / go / javascript | 0.25.x | workspace |
| tree-sitter-typescript / java / ruby / cpp | 0.23.x | workspace |
| tree-sitter-c | 0.24.x | workspace |
| Additional langs (php, swift, kotlin-ng, c-sharp) | see `ast-sgrep-lang` Cargo.toml | crate-level |
| **rusqlite** | 0.40 (bundled) | storage only — **not** SQL-class product oracle |
| workspace package version | **1.4.0** | `Cargo.toml` |
| criterion | `=0.5.1` | benches |
| Machine JSON schema | `1.0.0` | engine-identity / CLI goldens |
| MCP protocolVersion | `2024-11-05` | mcp protocol tests |
| IVF wire | magic `ASIVF\0`, version `2` | conformance inventory (Pass4) |
| SQLite schema | `user_version = 7` | index integrity |

### 3.3 Historical competitor pins (docs only — do not treat as live CI)

From `benchmarks/results/baselines.md` provenance (UNREPRODUCIBLE harness/corpus in-tree):

| Competitor | Published pin examples | In-tree correctness differential? |
|------------|------------------------|:---------------------------------:|
| ripgrep | 14.1.1 (corpus), 15.1.0 (competitors row) | **No** match-set suite |
| ast-grep | 0.44.1 (provenance), 0.45.0 (host/current) | **No** match-set suite |
| semgrep | 1.168.0 (ledger) / host 1.172.0 | **No** |

**Honesty:** Host versions may **drift** from published ledger pins. Any future differential must record **both** DUT git SHA and competitor `--version` in the envelope.

### 3.4 What is pin-ready vs pin-deferred

| Pin target | Ready today? | Notes |
|------------|:------------:|-------|
| `rg` binary version contract | Yes (host) | Add `docs/contracts/ripgrep_version_contract.toml` in a later implement pass if desired |
| `ast-grep` binary version contract | Yes (host) | Same; domain limited to native pattern subset |
| tree-sitter crate/grammar set | Yes (lock) | Prefer lock + grammar crate versions as the parse contract |
| semgrep | Optional | Quality ledger only; not correctness gate |
| Full `jell` cross-engine hit-ID equality | **Deferred** | `docs/validation/jell-deferral.md` |
| Published MRR/nDCG fingerprints | Ledger only | UNREPRODUCIBLE without gold harness; see Agents.md benchmark rules |

---

## 4. What is NOT a single-port (explicit non-goals)

These are **out of scope** for full FrankenSQLite-style treatment:

1. **No single upstream SQLite differential** — index durability uses SQLite; product claims are hybrid retrieval, not C-SQLite SQL semantics parity.
2. **No bit-identical result sets vs ripgrep** — lexical path is FTS/trigram/index-backed, not streaming rg-compatible (`jell-deferral.md`).
3. **No full surface parity vs ast-grep** — structural is a **native subset**; rewrites/codemods remain ast-grep's job; production search does not require spawning ast-grep.
4. **No official MCP/LSP conformance suite adoption as the sole oracle** — own process tests exist; not Model Context Protocol / LSP official runners as release oracles today.
5. **No "port of tree-sitter"** — tree-sitter is an embedded dependency for extraction; differential is extraction goldens / presence tuples, not reimplementing the C runtime.
6. **No claim that bench "parity clean" = correctness differential** — speed/head-to-head ledgers are latency/quality history (conformance Pass4: maturity external Pattern-1 ≈ 3/10).
7. **No certification-bundle ship in audit-only** — no release certificate from Pass 1; later scorecard pass invents nothing green.
8. **No gauntlet workspace sibling init required for this pass** — artifacts live under `tests/artifacts/gauntlet-audit/` per skill-loop progress.

---

## 5. Existing gauntlet-adjacent artifacts already in-tree

### 5.1 `docs/validation/*` (product validation surface)

| Artifact | Role |
|----------|------|
| `proof-pack.md` | Minimal reproducible gate command list (ranking/graph oracles, machine_contracts, mcp protocol, embed math, forbid-soundness) |
| `feature-universe.md` | Canonical feature IDs (hybrid/semantic/keyword/pattern/graph/chain/compact/doctor/mcp/forbid) |
| `engine-identity.md` | EngineIdentity + FailureBundle exit/envelope map |
| `surface-parity.md` | CLI / MCP / LSP / Pi capability matrix + intentional deltas |
| `negative-ledgers.md` | Fail-closed cases (missing root, empty index, embed URL, panic→error) |
| `jell-deferral.md` | **Authoritative** deferral of full cross-engine differential |
| `cargo-geiger-baseline.txt`, `childguard.md`, `machine-json-schema.md`, `scored-property.md`, `semantic-ivf-mmap.md`, `compact-output.md`, etc. | Domain-specific validation notes |

### 5.2 `benchmarks/results/*` (honesty ledgers)

| File | Role |
|------|------|
| `baselines.md` | **Canonical fingerprint table** for MRR/Recall/nDCG; UNREPRODUCIBLE status; Agents.md provenance root |
| `speed.md` | Latency vs rg / ast-grep on self corpus (historical / script-driven) |
| `head-to-head.md`, `bakeoff.md`, `losses.md` | Competitor quality/latency narratives; not CI match-set proofs |
| `benchmarks/README.md`, `docs/benchmarks.md` | How to interpret / run |

### 5.3 Proof pack / scripts

| Path | Role |
|------|------|
| `docs/validation/proof-pack.md` | Named command list for ranking honesty gates |
| `scripts/run-benchmarks.sh` | Requires hyperfine + rg + ast-grep; warm literal + structural timing |
| `scripts/verify-forbid-soundness` | First-party unsafe ban gate |
| `scripts/local-release-gate.sh`, `check-bench-output.py`, `check-error-budget.py` | Release/bench hygiene |

### 5.4 In-tree oracles & differentials (non-exhaustive; see conformance Pass4)

| Kind | Location | External? |
|------|----------|:---------:|
| Ranking soft oracle | `crates/ast-sgrep-core/tests/ranking_oracle.rs` + `tests/fixtures/ranking/cases.json` | No |
| Graph oracle | `crates/ast-sgrep-core/tests/graph_oracle.rs` | No |
| HitKey peer parity | `crates/ast-sgrep-cli/tests/no_embed_hit_key_parity.rs` | No (peer) |
| IVF CE-003 vs brute | `semantic_ivf_roundtrip.rs` | Math ref |
| Metamorphic | `crates/ast-sgrep-core/tests/metamorphic.rs` | No |
| Extraction presence goldens | `ast-sgrep-lang/tests/extraction_goldens.rs` | Same engine family |
| Machine contracts / CLI goldens | `ast-sgrep-cli/tests/` | Self |
| MCP protocol | `ast-sgrep-mcp/tests/protocol.rs` | Own server |
| Fuzz program | `fuzz/` + `tests/artifacts/fuzz-audit/` | Self + sanitizers |

### 5.5 Parallel skill-loop audits (do not re-file; cross-link later)

| Program | Epic id (progress files) | Artifacts |
|---------|--------------------------|-----------|
| Golden artifacts | `ast-sgrep-golden-artifacts-program-nz7i` | `tests/artifacts/golden-audit/PASS1…7` |
| Conformance harnesses | `ast-sgrep-conformance-harness-program-ghiw` | `tests/artifacts/conformance-audit/PASS1…7` |
| Fuzzing | `ast-sgrep-fuzz-program-maturity-b8q3` | `tests/artifacts/fuzz-audit/PASS1…10` |
| Mock-free / bug-hunt / perf | various | `tests/artifacts/mock-free-audit/`, `bug-hunt/`, `perf/` |
| Gauntlet (this skill) | progress: `.skill-loop-progress-gauntlet.md` | `tests/artifacts/gauntlet-audit/` (this file = Pass 1) |

### 5.6 Product docs anchoring multi-reference story

- `docs/comparison.md` — vs ast-grep vs ripgrep positioning  
- `docs/ARCHITECTURE.md` — hybrid pipeline + crate map  
- `docs/structural-patterns.md`, `docs/env-trust.md` — when external ast-grep may be used  
- `README.md` — intent layer claim; competitor complementarity  

---

## 6. Top 8 questions the rest of the gauntlet must answer for THIS project

These are the **gauntlet-driving questions** (not a backlog dump). Later passes must answer with evidence paths, not slogans.

1. **Composite oracle completeness:** For each product channel (lexical, graph, structural-native, semantic/ANN, fused hybrid, agent machine JSON), which of the five greenfield oracle modes is **authoritative**, and where is the scenario→mode dispatch written down?

2. **External differential honesty:** Given `jell-deferral.md`, what **minimal** external Pattern-1 suites (if any) should exist for (a) `rg` file:line subsets and (b) `ast-grep` pattern subset, with explicit non-goals and XFAIL vocabulary — without pretending full hit-ID identity?

3. **Perf keep-gate reality:** Do in-tree benches + `.bench-history` (or equivalent) support pass-over-pass ratchets vs self, and how do competitor timings in `speed.md` avoid being misread as keep-gate oracles when harnesses are UNREPRODUCIBLE?

4. **Published metric provenance:** Which fingerprint rows in `baselines.md` remain the sole canonical quotes, which are SUPERSEDED, and what is the remediation path to either restore harness+gold or permanently label historical?

5. **Surface FeatureUniverse expansion:** How do CLI/MCP/LSP/Pi/Code Mode/lang extractors map to `present|partial|missing|excluded` against **product promises** (not against full ast-grep or full ripgrep surfaces)?

6. **Negative-ledger discipline:** Are fail-closed cases (`negative-ledgers.md`) and rejected perf/conformance hypotheses tracked with **retry-condition predicates**, or only as static docs / soft tests?

7. **Cross-program de-dupe:** Which gaps are already owned by golden **nz7i**, conformance **ghiw**, fuzz **b8q3** (and mock-free/perf), so the gauntlet epic does **not** re-file micro-beads for the same hole?

8. **Certification readiness (honest no):** What would a release-certification bundle require for a multi-reference hybrid (spec SHA, property suite version, competitor pins, unreproducible ledger policy), and which required-pass constants are currently **red** vs **yellow** without inventing green?

---

## 7. Implications for Pass 2+ (pointer only)

| Next mission | Class-aware focus |
|--------------|-------------------|
| Pass 2 — Three pillars | Score (a) perf vs self + optional competitors; (b) composite conformance; (c) multi-surface FeatureUniverse — never one pillar alone |
| Pass 3 — Evidence honesty | Inventory baselines/oracles/fuzz/goldens; cite UNREPRODUCIBLE where true |
| Pass 7 — Oracle readiness | Cross-link ghiw Pass4 differential maturity; close `jell` decision as scope not silence |
| Pass 11 — Beads | Aggregate epic only; cross-link nz7i / ghiw / b8q3; no Pass 1 beads |

---

## 8. Evidence log (what this pass actually ran)

- Read skill `SKILL.md` mode/tier/class routers; `PROJECT-CLASSES.md` Greenfield section; `GREENFIELD-ADAPTATION.md`; `MODE-ROUTER.md` / `TIER-TRIAGE.md` excerpts  
- Repo inventory: `Cargo.toml` workspace members, `docs/validation/*`, `benchmarks/results/*`, `tests/artifacts/*`, progress files  
- Host versions: `rg --version`, `ast-grep --version`, `semgrep --version`, `hyperfine --version`  
- Lock pin: `Cargo.lock` tree-sitter **0.26.10**  
- LOC: `find crates -name '*.rs' | xargs wc -l` → **37454** total  
- Prior audits: conformance `PASS4_DIFFERENTIAL_REFERENCE.md`, golden `PASS1_GOLDEN_INVENTORY.md`  
- **Did not run:** workspace cargo test/build/bench; did not file beads; did not commit  

---

## 9. Verdict block (for orchestrator report)

| Item | Value |
|------|--------|
| **Artifact path** | `/Users/aditya/Developer/ast-sgrep/tests/artifacts/gauntlet-audit/PASS1_PROJECT_CLASS_REFERENCES.md` |
| **Class** | Greenfield-Rust-class / **greenfield multi-reference hybrid** (multi-oracle) |
| **Mode** | **`audit-only`** (justified) |
| **Tier** | **T3 Workspace** |
| **Top 3 questions** | (1) composite oracle completeness per channel; (2) external differential honesty under jell-deferral; (3) perf keep-gate vs UNREPRODUCIBLE competitor ledgers |

**DONE** — Pass 1 complete; audit-only; no beads; no commit.

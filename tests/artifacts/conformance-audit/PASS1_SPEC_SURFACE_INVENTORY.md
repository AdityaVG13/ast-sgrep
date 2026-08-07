# Pass 1/10 — Spec Surface Inventory (IDENTIFY only)

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Branch:** `perf/software-optimization` (inventory only; no beads, no implementation, no commits)  
**Date:** 2026-08-07  
**Skill:** `testing-conformance-harnesses` (loop step 1: IDENTIFY)  
**Scope:** What is the specification for each product surface? Do not audit harness quality in depth (pass 2).

**Search coverage:** `crates/**/tests`, `docs/**`, `docs/validation/**`, `CONTRIBUTING.md`, `benchmarks/**`, `packages/pi/**`, `tests/fixtures/**`, `scripts/**`, prior `tests/artifacts/golden-audit/`. Shell + `rg`; ZeroStack available (`zs 1.3.0`) but not required for this inventory.

---

## 1. Executive summary

This repo is **oracle / parity / machine-contract culture**, not RFC-style **spec-conformant** culture.

- Correctness is enforced by **fixture oracles** (`must_include` ranking, graph case-fold), **cross-surface hit-key parity** (CLI / core / LSP), **metamorphic relations** (no absolute oracle for hybrid rank / ANN), and **frozen machine JSON goldens** (CLI envelopes / capabilities).
- Product behavior is documented as **internal contracts** (`docs/QUERY_GRAMMAR.md`, `docs/validation/*`, `docs/signal-provenance.md`, index DDL + IVF format in code). These are mostly informal or machine contracts, not external RFCs with MUST clause matrices.
- Names that sound like full conformance (`parity`, `oracle`, `extraction_goldens`, `assert_language_conformance`) are usually **presence / structural / peer-surface gates**, not Pattern-1 differential suites or Pattern-4 clause matrices.
- External competitors (**ast-grep**, **ripgrep**, **tree-sitter**, **semgrep**) appear in **docs + bench ledgers** and optional speed comparisons; there is **no in-tree CI differential correctness suite** that asserts byte/structure equality against those tools for search results.
- Closest things to true conformance harnesses: CLI machine goldens (P2/P5), MCP process tests against protocol pin `2024-11-05` (P6), IVF wire round-trip (P3), lang presence tuples misnamed "conformance" (weak P4), Pi `release-contract.json` (P5).

| Dimension | Score (1–10) | Notes |
|-----------|:------------:|-------|
| Spec sources written down | 7 | Rich docs + validation pack + DDL in code |
| Spec version pins | 6 | Machine schema `1.0.0`, IVF v2, SQLite user_version 7, MCP `2024-11-05`, Pi npm `1.4.0` |
| Clause / MUST extraction | 2 | Negative ledgers and feature universe exist; no full MUST matrix |
| Differential vs reference impl | 2 | Bench-only / historical; production pattern path intentionally native-only |
| Oracle / parity gates | 7 | Ranking, graph, surface hit-keys, metamorphic, proof-pack |
| Golden / freeze infrastructure | 4 | CLI JSON goldens yes; no shared `assert_golden` / DISCREPANCIES / COVERAGE |
| **Overall conformance maturity** | **5** | Strong correctness culture under different names; not yet a conformance program |

**Verdict:** Maturity **5/10**. Treat as **"oracle-first product"** with emerging contract freezes, not as "implements Spec X with ≥95% MUST coverage."

---

## 2. Spec surface table

Skill patterns: **1** differential · **2** golden · **3** round-trip · **4** spec-derived matrix · **5** contract · **6** process/external runner.

| Surface | Spec source (docs / code / external) | Spec kind | Version pin? | Skill pattern 1–6 | Existing tests (paths) | Strength |
|---------|--------------------------------------|-----------|--------------|-------------------|------------------------|----------|
| **CLI machine envelope** (`tool`, `schema_version`, `ok`, exit codes) | `docs/validation/machine-json-schema.md`, `docs/validation/engine-identity.md`, `crates/ast-sgrep-cli/src/machine.rs` (`MACHINE_SCHEMA_VERSION = "1.0.0"`) | Machine contract (project-owned protocol) | **Yes** — `1.0.0`; Pi rejects mismatch | **2 + 5** | `crates/ast-sgrep-cli/tests/machine_contracts.rs`; fixtures `capabilities.json`, `envelopes.json`, `machine_shapes.json`; Pi `packages/pi/extension/src/runtime.ts` | **strong** (envelopes/shapes); **ok** (full hit dumps not frozen) |
| **CLI agent formats** (native / agent / agent-capsule / compact / github / gitlab) | `docs/validation/compact-output.md`, `docs/signal-provenance.md`, `docs/mcp.md` (compact notes), `crates/ast-sgrep-plugins` formatters | Informal + machine shapes | Compact schema fields `h,p,q,v,zb,zn,zt`; no separate semver for compact | **2 + 5** (partial) | `machine_contracts.rs` (agent modes, format aliases); `cli_smoke.rs`; `plugins/tests/capsule_format.rs` | **ok** — shape/budget identity strong; full dumps sparse |
| **Agent fail-closed / negative ledger** | `docs/validation/negative-ledgers.md`, `docs/validation/engine-identity.md` FailureBundle | Informal MUST-not list | Exit 1 usage / 2 operational documented | **4** (partial, short ledger) | `machine_contracts.rs` operational/usage; MCP sandbox tests; doctor unhealthy | **ok** |
| **Capabilities / discovery** (`asgrep capabilities --json`) | `docs/ARCHITECTURE.md` (self-describing surfaces), `docs/validation/machine-json-schema.md` | Machine contract | Tied to machine schema + clap surface | **2 + 5** | `machine_contracts.rs` (`capabilities_and_version_match_goldens`, subcommand list) | **strong** for key freeze |
| **Query prefix grammar** | `docs/QUERY_GRAMMAR.md` (normative for `ParsedQuery::parse`), `docs/ARCHITECTURE.md` | Informal normative doc + code | No external version; grammar is product-owned | **4** thin | `properties.rs` (`parse_never_panics`); routing in `pattern_routing.rs`, `cascade_planner.rs`, CLI/MCP smoke | **ok** for happy prefixes; **weak** as clause matrix (unsupported forms mostly prose) |
| **Hybrid ranking / fusion** | `docs/fusion-ranking.md`, `docs/how-it-works.md`, `docs/signal-provenance.md`, `crates/ast-sgrep-core/src/fusion.rs` | Informal math contract (RRF k=60, weights, margin rules) | Weights via `ASGREP_INTENT_WEIGHTS`; clamp `[0.25, 2.0]` | **4** informal; metamorphic **not** conformance | `ranking_oracle.rs` + `tests/fixtures/ranking/cases.json` (12 cases); `signal_provenance.rs`; `search_correctness_epics.rs`; `metamorphic.rs` | **ok** oracle bounds; **weak** absolute rank order / learned weights |
| **Ranking fixtures (sample corpus)** | `tests/fixtures/ranking/cases.json` + `tests/fixtures/sample/**` | Reference fixture oracle (must_include + max_rank) | Fixture-local; sample corpus not versioned as external gold | **2** sparse / **oracle** | `ranking_oracle.rs` | **ok** (12 cases, multi-lang defs/callers + 1 semantic synonym) |
| **Graph / defs / callers / imports** | `docs/symbol-normalization.md`, `docs/how-it-works.md`, SQL predicates in store | Informal + SQL contract (ASCII case-fold) | Schema indexes `idx_symbols_name_lower` (v6+) | **oracle** (~P4) | `graph_oracle.rs`; `parity.rs`; `chain_case.rs`; symbol normalization notes | **ok** for mixed-case ASCII; **weak** for FQN / non-ASCII |
| **Chain traversal** | Code `chain.rs` + `docs/use-cases.md` / feature universe | Informal product behavior | None external | **oracle** | `graph_oracle.rs` (chain non-empty); `chain_case.rs`; `downstream_correctness.rs` | **ok** case-fold fix; **weak** as full edge/node golden |
| **Lang extraction (13 languages)** | tree-sitter grammars (external), hand fixtures `crates/ast-sgrep-lang/tests/fixtures/extract/*`, `testkit` `LanguageConformanceCase` | Presence tuples over tree-sitter reference parsers; **not** full extract dumps | tree-sitter crate versions via Cargo.lock (implicit pin) | **4** misnamed / **presence oracle** | `extraction_goldens.rs` → `assert_language_conformance`; `pattern.rs` | **ok** regression net; **weak** full span/order dumps |
| **Pattern search (`pattern:`)** | `docs/structural-patterns.md` (native-only; no external ast-grep in prod) | Informal product subset of ast-grep-like syntax | Explicitly **diverges** from full ast-grep (no YAML/rules/rewrites) | Product oracle; **not** P1 vs ast-grep | `pattern_routing.rs`; `lang/tests/pattern.rs`; ranking/pattern modes; fail-closed exotic in `search_correctness_epics.rs` | **ok** for supported subset; **gap**: no differential suite vs ast-grep CLI for shared patterns |
| **Index / store schema (SQLite)** | `crates/ast-sgrep-core/src/store/sql.rs` DDL; `sqlite.rs` `SCHEMA_VERSION = 7`; `docs/how-it-works.md`, `docs/ARCHITECTURE.md` | Machine schema (DDL + migrations) | **Yes** — `PRAGMA user_version = 7` | **3** migrations / open integrity | `store_pragmas.rs`, `store_delete.rs`, `durability_epics.rs`, `semantic_chunk_migration.rs`, `semantic_v1_rewrite.rs` | **ok–strong** for open/migrate/delete; no golden SQL dumps |
| **Store durability / pragmas** | `docs/index-consistency.md` | Operational contract | WAL + `synchronous=NORMAL` + busy_timeout 5000 | **4** short | `store_pragmas.rs`; durability epics | **strong** for pragma values |
| **Index consistency / generations** | `docs/index-consistency.md` (`index_data_version`, `semantic_data_version`, ResponseCache keys) | Informal consistency model | Meta keys + PRAGMA data_version | property/regression | `freshness_identity.rs`, `response_cache_version.rs`, `semantic_cache_version.rs`, `cache_index_home.rs` | **ok** |
| **Semantic IVF / ANN sidecar** | `docs/validation/semantic-ivf-mmap.md`, `semantic_ivf.rs` (`ASIVF\0`, VERSION=2, HEADER 80, align 4096) | Binary wire format (project-owned) | **Yes** — format 2 | **3** round-trip + corrupt reject | `semantic_ivf_roundtrip.rs`; `semantic_ann_locality.rs`; mmap crate sealed unsafe | **strong** wire/round-trip; recall is threshold not exact |
| **Embed math / scoring** | `docs/validation/scored-property.md`, `ast-sgrep-embed` `math.rs` | Closed-form / property contracts (NaN, L2, cosine) | None external | unit + proptest (~P4) | `embed/src/math.rs` `contract_tests` / `property_tests`; proof-pack runs `math::` | **ok–strong** for pure math |
| **Semantic search backends** | `docs/semantic-search.md`, `docs/validation/neural-trust.md` | Informal + fail-closed preference | Embed backend identity in meta / fingerprint | unit + epic | `semantic_cache_version.rs`, `p1_correctness_batch.rs`, `cloud_feature_gate.rs`, ranking synonym case | **ok** hashed/semantic fixture; neural/cloud mostly gated |
| **Env trust / SSRF / boolish** | `docs/env-trust.md` | Informal security contract | Allowlists documented | **4** partial | MCP/CLI env tests; machine_contracts boolish; embed URL unit paths | **ok** |
| **MCP protocol** | External MCP (JSON-RPC over stdio); `docs/mcp.md`; code pin `protocolVersion: "2024-11-05"` | External protocol + product tool schema | **Yes** — MCP `2024-11-05`; tools list ordered freeze | **6** process-based + **5** schemas | `crates/ast-sgrep-mcp/tests/protocol.rs` | **ok–strong** for initialize/tools/list/sandbox; **not** full MCP compliance suite |
| **MCP tool semantics** (no auto-fusion; keyword/ast/semantic channels) | `docs/mcp.md`, `docs/validation/surface-parity.md` | Informal product delta vs CLI hybrid | Tool names frozen in test | **5** | `protocol.rs` hierarchical searches, compact IDs, `code_read` | **ok** |
| **LSP standard methods** | Microsoft LSP (informal adoption); `crates/ast-sgrep-lsp/README.md`, `docs/use-cases.md` | Subset of LSP 3.x-style methods | No explicit LSP version constant in inventory | **6** light / unit backend | `crates/ast-sgrep-lsp/tests/lsp.rs` | **ok** smoke + UTF-16/case; **weak** full LSP wire compliance |
| **LSP asgrep commands** (`asgrep.search`, etc.) | LSP README experimental provider; surface-parity table | Product extension over LSP | none | contract smoke | `lsp.rs`; hit-key parity with CLI/core | **ok** |
| **Surface parity CLI↔core↔LSP** | `docs/validation/surface-parity.md` | Intentional product matrix (with MCP deltas) | Documented deltas (MCP no auto-fusion) | peer **parity** (not P1 external) | `no_embed_hit_key_parity.rs`; embed-on variant | **ok–strong** for HitKey identity; MCP not in same key set |
| **Pi package / release contract** | `docs/pi-package.md`, `docs/RELEASING.md`, `packages/pi/release-contract.json` | Machine release contract | **Yes** — npm `1.4.0`, native CLI `1.4.0`, machine schema `1.0.0`, Pi agent `>=0.80.6 <1` | **5** | `packages/pi/scripts/check-contract.mjs`, release-preflight/acceptance/gate-e2e, extension tests | **strong** version coupling |
| **Pi / Code Mode catalog** | `docs/codemode.md`; Cloudflare/Anthropic/OpenAI **ideas** (not formal pin); Rust catalog schemas | Informal adapter contracts | Catalog names in tests | **5** structural | `codemode/tests/catalog.rs`, `session_plan.rs`, `batch.rs`; `packages/pi/extension/test/codemode*.ts` | **ok** tool names/adapters; no external runner suite |
| **Plugins capsule / compact identity** | `docs/validation/compact-output.md`, plugins format code | Machine format + budget policy | Compact keys alphabetical accounting tail | exact synthetic | `plugins/tests/capsule_format.rs` | **strong** for synthetic responses |
| **Forbid-unsafe / mmap exception** | workspace lints, `scripts/verify-forbid-soundness`, SECURITY | Policy contract | mmap sealed exception only | policy gate | `scripts/verify-forbid-soundness`; CI PR check | **strong** for policy |
| **Benchmark identity / latency budgets** | `benchmarks/README.md`, `docs/benchmarks.md`, `docs/validation/proof-pack.md` | Gate thresholds + identity oracles (product) | Budgets in docs; historical numbers UNREPRODUCIBLE ledger | **not** correctness conformance | `asgrep bench` + `scripts/check-bench-output.py`; identity oracles in bench suite | **ok** as release gate; **not** competitor result equality |
| **Feature universe** | `docs/validation/feature-universe.md` | Product feature ID list | bead-tagged | catalog only | proof-pack references; ranking/oracle coverage partial | **weak** as test matrix (IDs not 1:1 tests) |

**Row count:** 28 surfaces in the table above.

---

## 3. False friends

Things named parity / oracle / conformance / golden that are **not** full conformance harnesses (skill: Pattern 1–6 with clause matrix + DISCREPANCIES + coverage report).

| Name | Path | What it actually is | Why not full conformance |
|------|------|---------------------|--------------------------|
| **`parity` suite** | `crates/ast-sgrep-core/tests/parity.rs` | Thin e2e smoke: option wiring, IVF preserve, index/defs/hybrid/chain on sample | Self-consistency on one corpus; no external reference; no clause IDs |
| **`ranking_oracle`** | `ranking_oracle.rs` + `cases.json` | `must_include` + `max_rank` constraints | Soft rank bounds, not full ordered ranking or competitor differential |
| **`graph_oracle`** | `graph_oracle.rs` | Non-empty retrieval + case-fold for known symbols | Fixture oracle, not graph formal semantics / multi-lang FQN |
| **`extraction_goldens` / `assert_language_conformance`** | `lang/tests/extraction_goldens.rs`, `testkit/src/lang.rs` | Presence/forbid/call/pattern **tuples** | Named conformance; expectations are hand lists, not dumps vs tree-sitter reference AST, no DISCREPANCIES for grammar drift |
| **`no_embed_hit_key_parity` / surface equivalence** | `cli/tests/no_embed_hit_key_parity.rs` | CLI vs core vs LSP sorted HitKeys | Peer-surface parity; all three can share a bug; not external oracle |
| **`surface-parity.md`** | `docs/validation/surface-parity.md` | Capability matrix CLI/MCP/LSP/Pi | Documentation of intentional deltas, not an automated matrix generator |
| **`metamorphic` suite** | `core/tests/metamorphic.rs` | Relations under transforms (oracle problem explicit) | Correctness technique when no oracle exists; **anti-pattern** to call "conformance" |
| **Bench "parity clean" / speed "≈ ripgrep"** | `benchmarks/results/*.md` | Historical latency/quality ledgers | Performance/quality, often UNREPRODUCIBLE; not CI result-equality suite |
| **`IndexSnapshot` / structuredClone "snapshot"** | testkit / Pi tests | Logging / mutability helpers | Golden-audit already flagged; not approval testing |
| **Proof pack** | `docs/validation/proof-pack.md` | Curated gate command list | Merge discipline, not a compliance report |
| **Feature universe IDs** | `feature-universe.md` | Named product features | Inventory, not MUST clause coverage accounting |
| **eval gold** | CLI eval + ephemeral temp gold in tests | Metric harness | No checked-in multi-query eval gold for published MRR claims |

---

## 4. Competitor / reference implementations

| Reference | Role in product | In-tree differential correctness suite? | Notes |
|-----------|-----------------|----------------------------------------|-------|
| **tree-sitter** (+ language grammars) | Primary parse/extract engine | **No** AST dump differential; presence oracles only | Spec is "whatever tree-sitter emits"; locked via Cargo |
| **ast-grep CLI** | Documented complement; **production `pattern:` never spawns it** (`docs/structural-patterns.md`, `docs/env-trust.md`) | **No** result-equality suite | Optional bench compare only if `ASGREP_ALLOW_AST_GREP=1` + absolute `ASGREP_AST_GREP` |
| **ripgrep** | Lexical competitor in docs/benches | **No** match-set differential | `scripts/run-benchmarks.sh` warm literal vs rg; speed ledgers |
| **semgrep** | Historical bake-off competitor | **No** | `benchmarks/results/bakeoff.md` / baselines (quality ledger) |
| **MCP host ecosystem** | Transport consumers | Process tests against **own** server only | Pin `2024-11-05`; not official MCP conformance runner |
| **LSP clients (VS Code etc.)** | IDE consumers | Backend unit/integration only | No `vscode-languageclient` protocol suite |
| **Cloudflare / Anthropic / OpenAI Code Mode shapes** | Adapter inspiration | Structural adapter asserts in catalog tests | Not vendor conformance runners |
| **Pi coding agent** | Primary agent package host | Release contract + extension tests | Pins agent range; not Pi's own conformance suite |

**Differential summary:** The only systematic "run two paths and compare" style in-core is **internal** (e.g. metamorphic notes on ANN brute-force vs probed, CLI vs core vs LSP HitKeys, IVF save/load). **External competitor differential for correctness is absent** from the default/proof-pack test set.

---

## 5. Top 10 highest-value conformance gaps

Ranked by **(spec clarity × user risk × current weakness)**. Higher = fix first for a future conformance program.

| Rank | Gap | Spec clarity | User risk | Current weakness | Why it scores high |
|------|-----|:------------:|:---------:|:----------------:|--------------------|
| **1** | **Machine JSON schema as single frozen schema artifact + multi-surface consumers** | High (`1.0.0` fields documented) | High (agents parse envelopes; Pi rejects mismatch) | CLI goldens strong; full hit/agent dumps and MCP non-envelope path less unified | One drift breaks agents/Pi; already partial goldens → highest ROI to complete |
| **2** | **Query grammar MUST/SHOULD matrix** (supported prefixes, fail modes, no silent hybrid misroute) | High (QUERY_GRAMMAR.md is short and normative) | High (wrong mode → wrong evidence) | Parse never-panics + spot tests; no clause IDs / unsupported matrix | Spec is small enough to fully extract |
| **3** | **Native `pattern:` vs documented subset + intentional non-delegation to ast-grep** | High (structural-patterns.md) | High (users expect ast-grep power) | Smoke + routing; **no** DISCREPANCIES.md vs full ast-grep | Users confuse with ast-grep; divergence must be explicit + tested |
| **4** | **Symbol/graph normalization contract** (ASCII case-fold, stored spelling, imports LIKE) | High (symbol-normalization.md) | High (Issue #12 class: indexed but not retrieved) | graph_oracle + parity cover happy path; FQN/non-ASCII called out as out of contract | Already had production-class bugs; needs clause-level tests + DISC entries |
| **5** | **MCP tool schemas + semantics (no auto-fusion, sandbox, compact ID session)** | Medium–high (mcp.md + tests) | High (agent tools) | Good process tests; not full MCP compliance matrix / schema golden files | Agents depend on tool list stability and fail-closed roots |
| **6** | **Index schema v7 + migration / corrupt reopen / generation bump rules** | High (DDL + index-consistency.md) | High (data loss / stale cache) | Pragma + durability epics; no migration golden corpus of old DBs | Silent schema/cache bugs poison all surfaces |
| **7** | **IVF format v2 wire compatibility** (magic/version/header/fingerprint) | High (binary constants + validation doc) | Medium–high (semantic quality/perf fallback) | Strong round-trip/corrupt tests | Already good; gap is **cross-version fixture corpus** + provenance |
| **8** | **Lang extraction full contract per language** (kinds, spans, calls, imports) | Medium (fixtures + tree-sitter) | High (index wrong → all retrieval wrong) | Presence tuples only | 13 langs; grammar bumps can silently drop symbols |
| **9** | **Signal provenance / margin invariants on all JSON surfaces** | High (signal-provenance.md) | Medium–high (agent trust calibration) | Unit fusion tests + some surface asserts | Spoofed fields re-derived; compact intentionally drops fields — needs DISC + matrix |
| **10** | **LSP method subset vs LSP spec + experimental asgrep provider** | Medium (README method table; no version pin) | Medium (IDE navigation wrong) | Smoke + case-fold refs; no protocol conformance runner | Wrong positions/UTF-16 already regressed once |

Honorable mentions (just outside top 10): hybrid RRF absolute order; chain full expand goldens; Pi Code Mode adapter fidelity to vendor schemas; env-trust SSRF exhaustive host matrix; competitor differential for `pattern:` overlap with ast-grep and lexical with ripgrep.

---

## 6. Culture map (how this repo tests "correctness")

```text
External formal RFCs          almost none (MCP date pin only)
        │
Internal machine contracts    CLI envelope 1.0.0, IVF v2, schema v7, Pi release-contract
        │
Oracle fixtures               ranking cases.json, graph_oracle, extraction tuples
        │
Peer surface parity           CLI ↔ core ↔ LSP HitKeys
        │
Metamorphic / properties      hybrid rank, ANN, reindex (no absolute oracle)
        │
Process harnesses             MCP stdio JSON-RPC, CLI subprocess goldens
        │
Bench ledgers                 vs ripgrep / ast-grep / semgrep (speed/quality, not CI equality)
```

**Proof pack (documented gate list):** forbid-soundness; `ranking_oracle`; `graph_oracle`; `machine_contracts`; MCP `protocol`; embed `math::`. That is the closest thing to a "conformance bar" today -- still oracle/contract, not clause compliance.

---

## 7. Spec pins quick reference

| Pin | Value | Where |
|-----|-------|--------|
| Machine JSON schema | `1.0.0` | `cli/src/machine.rs`, Pi runtime, release-contract |
| SQLite user_version | `7` | `core/src/store/sqlite.rs` |
| IVF sidecar | magic `ASIVF\0`, version `2`, header 80, align 4096 | `semantic_ivf.rs`, validation doc |
| MCP protocolVersion | `2024-11-05` | MCP initialize test + server |
| Pi npm / native CLI | `1.4.0` | `packages/pi/release-contract.json` |
| Pi agent range | `@earendil-works/pi-coding-agent >=0.80.6 <1` | `docs/pi-package.md` |
| Compact accounting keys | `zb`, `zn`, `zt` after content | compact-output.md |

---

## 8. Out of scope for this pass

- Harness quality deep audit (loader, comparator, XFAIL, CI) → **Pass 2**
- Clause extraction, fixtures, or DISCREPANCIES.md → later passes
- Filing `br` beads, implementing tests, or commits

---

## 9. Report card (this pass)

| Item | Value |
|------|--------|
| **Deliverable path** | `/Users/aditya/Developer/ast-sgrep/tests/artifacts/conformance-audit/PASS1_SPEC_SURFACE_INVENTORY.md` |
| **Spec surface table rows** | **28** |
| **Culture** | Oracle / parity / machine-contract (not RFC conformance) |
| **Maturity** | **5 / 10** |
| **Top 3 gaps** | (1) unified machine JSON multi-surface freeze, (2) query-grammar MUST matrix, (3) pattern: subset vs ast-grep DISCREPANCIES + tests |
| **False friends catalogued** | 12 |
| **External differential correctness** | Absent (bench/docs only) |

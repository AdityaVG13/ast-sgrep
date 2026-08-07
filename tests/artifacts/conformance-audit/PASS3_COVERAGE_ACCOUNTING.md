# Pass 3/10 — Coverage Accounting Matrix (EXTRACT)

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Branch:** `perf/software-optimization` (audit only; no beads, no implementation, no commits)  
**Date:** 2026-08-07  
**Skill:** `testing-conformance-harnesses` (loop step 2: EXTRACT + Coverage Accounting Matrix)  
**Prior:** [`PASS1_SPEC_SURFACE_INVENTORY.md`](./PASS1_SPEC_SURFACE_INVENTORY.md), [`PASS2_HARNESS_ARCHITECTURE.md`](./PASS2_HARNESS_ARCHITECTURE.md)  
**Scope:** Enumerate MUST/SHOULD-like critical behaviors and map them to existing tests. **No architecture redesign. No implementation. No beads.**

---

## 0. Methodology and honesty rules

### What "MUST-like" / "SHOULD-like" means here

This product has **almost no RFC-style MUST/SHOULD vocabulary**. Clauses below are **extracted estimates** from:

| Source class | Paths used |
|--------------|------------|
| Normative product docs | `docs/QUERY_GRAMMAR.md`, `docs/structural-patterns.md`, `docs/symbol-normalization.md`, `docs/signal-provenance.md`, `docs/mcp.md`, `docs/index-consistency.md`, `docs/fusion-ranking.md` |
| Machine / validation contracts | `docs/validation/machine-json-schema.md`, `engine-identity.md`, `negative-ledgers.md`, `surface-parity.md`, `compact-output.md`, `semantic-ivf-mmap.md`, `proof-pack.md` |
| Code pins | `MACHINE_SCHEMA_VERSION = "1.0.0"`, `SCHEMA_VERSION = 7`, IVF `ASIVF\0` / `VERSION=2` / `HEADER_SIZE=80`, MCP `2024-11-05` |
| Test names + fixture shape | `machine_contracts.rs`, `ranking_oracle` + `cases.json`, `graph_oracle`, `semantic_ivf_roundtrip`, `extraction_goldens`, MCP `protocol.rs`, `signal_provenance.rs`, `pattern*.rs` |
| Honesty law | Root `Agents.md` + `benchmarks/results/baselines.md` (no bare MRR without provenance) |

**Labels (do not invent false precision):**

| Label | Meaning |
|-------|---------|
| **MUST-like** | Behavior that, if broken, ships wrong agent/CLI contracts, wrong retrieval, or silent fail-open |
| **SHOULD-like** | Documented desirable behavior, multi-lang expansion, or completeness of a freeze |
| **Tested** | At least one automated check whose name/body clearly targets the behavior |
| **Passing** | **Assumed if a dedicated test exists** (this pass did **not** re-run the suite); label is `assumed`, not measured |
| **Divergent** | Documented intentional product delta (e.g. no ast-grep spawn, compact drops fields) |
| **Unknown** | Spec prose exists but no enumerated clause ID / no countable test matrix |
| **Score estimate** | `tested_MUST ≈ Tested ∩ MUST-like` over `MUST-like` count; **ranges**, not 3-decimal claims |

**Skill rule applied:** Score **&lt; 0.95** on MUST-like ⇒ **not conformant** for that surface (even if product quality is high under oracle culture).

**Global absences (from Pass 2, still true):** no `DISCREPANCIES.md`, no `COVERAGE.md`, no requirement-level tags on cases, no formal clause IDs (`QG-001`, …).

---

## 1. Highest-value surfaces (selection)

Selected from Pass 1 top-10 gaps + strongest harnesses in Pass 2. **Eight surfaces** (minimum six).

| # | Surface | Spec pin / doc | Why highest value |
|---|---------|----------------|-------------------|
| S1 | CLI machine JSON envelope | schema `1.0.0` | Agent/Pi parse contracts; one drift breaks tooling |
| S2 | Query prefix grammar | `QUERY_GRAMMAR.md` | Small normative surface; wrong mode → wrong evidence |
| S3 | Native `pattern:` subset | `structural-patterns.md` | Users equate with ast-grep; intentional non-delegation |
| S4 | Symbol / graph case-fold | `symbol-normalization.md` | Issue #12 class (indexed but not retrieved) |
| S5 | MCP tools + sandbox | `mcp.md`, protocol pin | Agent tools; fail-closed roots |
| S6 | Index schema + consistency | user_version `7`, `index-consistency.md` | Data loss / stale cache across all surfaces |
| S7 | IVF sidecar wire v2 | `ASIVF\0` / v2 / header 80 | Semantic quality + mmap safety |
| S8 | Signal provenance / margin | `signal-provenance.md` | Agent trust calibration; compact intentional drop |

Honorable (counted in §4 but not full tables): lang extraction presence tuples; ranking oracle soft bounds; benchmark honesty ledger.

---

## 2. Per-surface coverage tables

> **Reading the Score column:** estimates only. Format `~0.xx (est.)`.  
> Counts are **enumerations of critical behaviors**, not a machine-generated clause corpus.  
> When the true clause set is unenumerable, MUST-like is marked **unknown** and Score is **n/a (needs extraction project)**.

### S1 — CLI machine JSON envelope (`1.0.0`)

**Spec sources:** `docs/validation/machine-json-schema.md`, `engine-identity.md`, `negative-ledgers.md`; `crates/ast-sgrep-cli/src/machine.rs`; fixtures `capabilities.json`, `envelopes.json`, `machine_shapes.json`.  
**Primary harness:** `crates/ast-sgrep-cli/tests/machine_contracts.rs` (**16** `#[test]`).  
**Formal MUST list?** Partial (field table + FailureBundle table). **Not** numbered clause IDs.

| Spec / behavior (MUST-like) | Level | Tested? | Passing | Divergent / unknown | Evidence |
|-----------------------------|:-----:|:-------:|:-------:|---------------------|----------|
| `tool` always `"asgrep"` | MUST | Y | assumed | — | `assert_success` / fixtures |
| `schema_version` == `"1.0.0"` | MUST | Y | assumed | — | every machine assert |
| `ok` boolean; hard faults set `false` | MUST | Y | assumed | — | usage/operational/doctor paths |
| Success envelope `exit_code` 0 | MUST | Y | assumed | — | `assert_success` |
| Usage failure exit **1**, `error.kind=usage` | MUST | Y | assumed | — | format typos / bad flags |
| Operational failure exit **2** | MUST | Y | assumed | — | missing root / empty index style |
| Doctor unhealthy: `healthy:false`, `ok:false`, exit 2 | MUST | Y | assumed | — | `assert_doctor_unhealthy` |
| Capabilities key freeze vs golden | MUST | Y | assumed | — | `capabilities.json` golden |
| Capabilities lists all clap subcommands | MUST | Y | assumed | — | `capabilities_lists_all_clap_…` |
| Shape freeze: index/status/doctor/agent/agent-capsule/compact | MUST | Y | assumed | — | `machine_shapes.json` |
| Envelope goldens: usage/operational/version | MUST | Y | assumed | — | `envelopes.json` |
| `--format` alone implies machine JSON | MUST | Y | assumed | — | dedicated test |
| Format aliases map to same identity | MUST | Y | assumed | — | aliases test |
| Single envelope on bench suite failure | MUST | Y | assumed | — | `bench_suite_json_is_single_envelope…` |
| Pi triple-check rejects schema mismatch | MUST | partial | assumed | Pi path separate | `packages/pi` release-contract / runtime |
| Full search hit-array golden freeze | MUST | **N** | — | **unknown completeness** | Pass 1: full dumps not frozen |
| MCP emits same CLI envelope | — | N | — | **Divergent** (JSON-RPC `isError`) | `surface-parity.md` |

| Spec / behavior (SHOULD-like) | Tested? | Notes |
|-------------------------------|:-------:|-------|
| Doctor suggested commands echo effective root | Y | machine_contracts |
| Edit-distance-2 typos rejected before search | Y | machine_contracts |
| `index --dry-run` does not mutate | Y | machine_contracts |
| Bench skips vacuous ast-grep comparison | Y | honesty-adjacent |
| `robot-docs` markdown/json topics | Y | partial |
| `machine_schema_version` dual identity field always present | partial | schema notes; not every envelope proven |

**Coverage accounting (S1):**

| Spec Section / surface | MUST-like | SHOULD-like | Tested (MUST) | Passing (assumed) | Divergent/unknown | Score estimate |
|------------------------|:---------:|:-----------:|:-------------:|:-----------------:|-------------------|---------------|
| CLI machine envelope 1.0.0 | **~16** | **~6** | **~14** | ~14 | full hit dumps unknown; MCP envelope divergent by design | **~0.88 MUST (est.)** |
| | | | | | | **~0.85 overall MUST+SHOULD (est.)** |

**Verdict:** Strongest product contract surface. Still **&lt; 0.95 MUST** due to incomplete hit-array freeze and multi-consumer (Pi/MCP) not one golden suite.

---

### S2 — Query prefix grammar

**Spec sources:** `docs/QUERY_GRAMMAR.md` (normative for `ParsedQuery::parse`).  
**Primary tests:** unit tests in `crates/ast-sgrep-core/src/query.rs` (**~4**), `properties.rs` (`parse_never_panics`), routing smoke in `pattern_routing.rs` / CLI-MCP smoke.  
**Formal MUST list?** **No clause IDs.** Short table of 8 modes + unsupported list.

| Spec / behavior (MUST-like) | Level | Tested? | Passing | Divergent / unknown |
|-----------------------------|:-----:|:-------:|:-------:|---------------------|
| Unprefixed → Hybrid | MUST | Y | assumed | — |
| `callers:` / `defs:` / `imports:` mode select | MUST | Y | assumed | — |
| `pattern:` mode select | MUST | Y | assumed | — |
| `literal:` / `regex:` / `word:` mode select | MUST | Y | assumed | — |
| `raw` retains mode prefix for all modes | MUST | Y | assumed | unit `raw_keeps_mode_prefix…` |
| Parse never panics on arbitrary input | MUST | Y | assumed | proptest `parse_never_panics` |
| No composable `AND` / multi-prefix conjunction | MUST | **N** | — | **unknown** (prose only; no negative matrix) |
| `sem:` / `path:` / `lang:` **not** query filters | MUST | **N** | — | **unknown** (prose; may hybrid-as-text) |
| Nested/parenthesized boolean unsupported | MUST | **N** | — | **unknown** |
| Empty target after prefix well-defined | SHOULD→MUST-ish | partial | assumed | not fully table-driven |
| Whitespace / casing of prefixes | SHOULD | partial | assumed | — |

**Coverage accounting (S2):**

| Spec Section / surface | MUST-like | SHOULD-like | Tested (MUST) | Passing (assumed) | Divergent/unknown | Score estimate |
|------------------------|:---------:|:-----------:|:-------------:|:-----------------:|-------------------|---------------|
| Query prefix grammar | **~10** | **~4** | **~6** | ~6 | unsupported-forms matrix **unknown** | **~0.60 MUST (est.)** |

**Verdict:** Spec is small enough for full extraction, but **unsupported / fail modes are not clause-tested**. Score clearly **&lt; 0.95**. Highest ROI for a true Pattern-4 matrix.

---

### S3 — Native `pattern:` subset (vs ast-grep)

**Spec sources:** `docs/structural-patterns.md` (explicit non-delegation).  
**Primary tests:** `lang/tests/pattern.rs` (**6**), `core/tests/pattern_routing.rs` (**3**), `search_correctness_epics` exotic fail-closed, ranking oracle pattern modes (sparse).  
**Formal MUST list?** Partial supported/unsupported bullets. **No DISCREPANCIES vs full ast-grep.**

| Spec / behavior (MUST-like) | Level | Tested? | Passing | Divergent / unknown |
|-----------------------------|:-----:|:-------:|:-------:|---------------------|
| Never spawn external `ast-grep` in production path | MUST | partial | assumed | **Divergent** by design from full ast-grep; no process-guard assertion catalogued as global | 
| Exact identifiers / indexed signatures match | MUST | Y | assumed | — |
| Declaration metavars e.g. `fn $NAME($$$)` | MUST | Y | assumed | — |
| Free calls / `$FUNC($$$)` | MUST | partial | assumed | smoke, not exhaustive |
| Member calls `$OBJECT.$METHOD($$$)` | MUST | partial | assumed | — |
| Exact signature equality (`struct App` ≠ `struct AppContext`) | MUST | **N** | — | **unknown** (documented, not seen as dedicated test) |
| One-segment call patterns match final callee only | MUST | **N** | — | **unknown** |
| Unsupported (YAML rules, rewrites, nested templates) → empty, not panic | MUST | Y | assumed | `exotic_pattern_*`, `iva9_7_*` |
| Literal pattern matching case-sensitive | MUST | Y | assumed | pattern.rs |
| Full ast-grep feature parity | — | N | — | **Divergent** (documented: use standalone ast-grep) |

**Coverage accounting (S3):**

| Spec Section / surface | MUST-like | SHOULD-like | Tested (MUST) | Passing (assumed) | Divergent/unknown | Score estimate |
|------------------------|:---------:|:-----------:|:-------------:|:-----------------:|-------------------|---------------|
| Native `pattern:` subset | **~9** | **~5** (multi-lang matrix, kind: signatures, DISC catalog) | **~5** | ~5 | full subset catalog **unknown**; ast-grep parity **divergent** | **~0.55 MUST (est.)** |

**Verdict:** Fail-closed exotic path is tested; **supported subset is smoke, not a completeness matrix**. Users can still confuse product with ast-grep.

---

### S4 — Symbol / graph normalization (ASCII case-fold)

**Spec sources:** `docs/symbol-normalization.md` (Issue #12 / powt class).  
**Primary tests:** `graph_oracle.rs` (1 large multi-query), `parity.rs` store spelling, chain/downstream case tests.  
**Formal MUST list?** Yes-ish narrative + SQL predicate table. Still no clause IDs.

| Spec / behavior (MUST-like) | Level | Tested? | Passing | Divergent / unknown |
|-----------------------------|:-----:|:-------:|:-------:|---------------------|
| Index preserves extractor spelling (no write-path lowercasing) | MUST | Y | assumed | parity store reads |
| `defs:` SQL `lower(s.name) = lower(?)` | MUST | Y | assumed | graph_oracle multi-case |
| `callers:` SQL `lower(c.callee) = lower(?)` | MUST | Y | assumed | graph_oracle + positive score |
| `imports:` escaped LIKE substring, case-insensitive | MUST | partial | assumed | imports non-empty; less case matrix |
| Retrieval + `score_normalized_symbol` stay aligned | MUST | Y | assumed | positive caller scores after mixed-case query |
| Mixed-case query variants resolve stored spelling | MUST | Y | assumed | `SYMBOLS` query table |
| Chain expand resolves case variants | MUST | Y | assumed | graph_oracle chain_ok |
| Non-ASCII case equivalence | — | N | — | **Out of contract / divergent** (Unicode rank vs ASCII SQL) |
| FQN / module-qualified canonicalization | — | N | — | **Out of contract** (stored as extracted) |

**Coverage accounting (S4):**

| Spec Section / surface | MUST-like | SHOULD-like | Tested (MUST) | Passing (assumed) | Divergent/unknown | Score estimate |
|------------------------|:---------:|:-----------:|:-------------:|:-----------------:|-------------------|---------------|
| Symbol/graph ASCII case-fold | **~7** | **~3** (FQN, non-ASCII, multi-lang FQN) | **~6** | ~6 | FQN/non-ASCII **divergent boundaries** | **~0.85 MUST (est.)** on in-contract ASCII |

**Verdict:** Best oracle for a once-broken production class. Still **&lt; 0.95** if imports case matrix and multi-lang FQN edges are counted as MUST; with strict in-contract ASCII scope, closer to **~0.85–0.90**.

---

### S5 — MCP tools, schemas, sandbox

**Spec sources:** `docs/mcp.md`, `docs/validation/surface-parity.md`, `compact-output.md` (MCP sections).  
**Primary harness:** `crates/ast-sgrep-mcp/tests/protocol.rs` (**14** `#[test]`).  
**Formal MUST list?** Tool inventory + process pins. **Not** official MCP conformance suite.

| Spec / behavior (MUST-like) | Level | Tested? | Passing | Divergent / unknown |
|-----------------------------|:-----:|:-------:|:-------:|---------------------|
| `initialize` protocolVersion `2024-11-05` | MUST | Y | assumed | — |
| `tools/list` exposes search + index tools | MUST | Y | assumed | freeze of names |
| No auto-fusion across keyword/ast/semantic | MUST | Y | assumed | hierarchical search test |
| `code_search` is non-fusing alias | MUST | partial | assumed | docs; test coverage thin |
| `code_read` expands session IDs + budgets | MUST | Y | assumed | — |
| Reject invalid budgets / binary / stale ranges | MUST | Y | assumed | — |
| Argument schemas enforced | MUST | Y | assumed | — |
| Unknown method → JSON-RPC method not found | MUST | Y | assumed | — |
| Unknown tool → tool error result | MUST | Y | assumed | — |
| Root sandbox: path outside workspace → `isError` | MUST | Y | assumed | negative ledger alignment |
| `tools/list` byte-identical across calls/processes | MUST | Y | assumed | — |
| Search envelope byte-stable; accounting keys last | MUST | Y | assumed | `zb/zn/zt` ordering |
| Snippet elision + `resend_seen` + clear on reindex | MUST | Y | assumed | — |
| Official MCP multi-client conformance suite | SHOULD | **N** | — | **unknown** / not claimed |
| Full tools/list JSON golden file | SHOULD | partial | assumed | assert names, not full schema golden dump |

**Coverage accounting (S5):**

| Spec Section / surface | MUST-like | SHOULD-like | Tested (MUST) | Passing (assumed) | Divergent/unknown | Score estimate |
|------------------------|:---------:|:-----------:|:-------------:|:-----------------:|-------------------|---------------|
| MCP product contracts | **~13** | **~4** | **~12** | ~12 | official MCP suite absent | **~0.90 MUST (est.)** product; **~0.00** vs external MCP suite |

**Verdict:** Strong process harness for **owned** protocol pin. Still **&lt; 0.95** if full tool JSON Schema freeze is required; **not** MCP-org conformant.

---

### S6 — Index schema v7 + consistency / durability

**Spec sources:** `docs/index-consistency.md`; `SCHEMA_VERSION = 7` in `store/sqlite.rs`.  
**Primary tests:** `store_pragmas.rs`, durability/freshness/cache version tests, migration tests (scattered).  
**Formal MUST list?** Operational table (WAL, NORMAL, busy 5000) + generation rules. Migrations not fully clause-listed.

| Spec / behavior (MUST-like) | Level | Tested? | Passing | Divergent / unknown |
|-----------------------------|:-----:|:-------:|:-------:|---------------------|
| `PRAGMA user_version` / schema == **7** | MUST | partial | assumed | migrations exist; not one golden “v7 only” matrix |
| `journal_mode` = WAL | MUST | Y | assumed | store_pragmas |
| `synchronous` = NORMAL (1) at rest | MUST | Y | assumed | — |
| `busy_timeout` = 5000 ms | MUST | Y | assumed | — |
| `file_tx` / bulk restore NORMAL after commit/rollback | MUST | Y | assumed | store_pragmas |
| Integrity check on open; quarantine corrupt | MUST | partial | assumed | integrity_check path; quarantine less matrixed |
| `index_data_version` bump on searchable mutation | MUST | partial | assumed | freshness/cache tests |
| ResponseCache fail-closed if generation unreadable | MUST | partial | assumed | response_cache_version tests |
| Sidecars never independent truth (fingerprint miss → fallback) | MUST | partial | assumed | search_correctness empty sidecar |
| IVF fingerprint includes `semantic_data_version` | MUST | partial | assumed | index-consistency + IVF tests |
| Non-UTF8 indexed paths rejected | MUST | partial | assumed | kqhp epic (assumed present; not re-verified here) |
| Golden corpus of pre-v7 DBs migrate cleanly | SHOULD | **N** | — | **unknown** / gap |
| Multi-writer concurrent reindex stress | SHOULD | **N** | — | **unknown** |

**Coverage accounting (S6):**

| Spec Section / surface | MUST-like | SHOULD-like | Tested (MUST) | Passing (assumed) | Divergent/unknown | Score estimate |
|------------------------|:---------:|:-----------:|:-------------:|:-----------------:|-------------------|---------------|
| Index schema + consistency | **~11** | **~4** | **~7–8** | ~7–8 | migration golden corpus **unknown** | **~0.70 MUST (est.)** |

**Verdict:** Pragma/durability island is strong; **generation + migration completeness** keeps score well below 0.95.

---

### S7 — IVF format v2 wire + mmap contract

**Spec sources:** `docs/validation/semantic-ivf-mmap.md`; constants in `semantic_ivf.rs`.  
**Primary tests:** `semantic_ivf_roundtrip.rs` (**9** tests, **1** `#[ignore]` tradeoff).  
**Formal MUST list?** Binary constants + publish/mmap rules. Latency numbers in validation doc are **host-measured** (honesty: do not restate as universal SLOs without provenance).

| Spec / behavior (MUST-like) | Level | Tested? | Passing | Divergent / unknown |
|-----------------------------|:-----:|:-------:|:-------:|---------------------|
| Magic `ASIVF\0` | MUST | Y | assumed | reject path |
| Version **2** | MUST | Y | assumed | — |
| Header size **80** | MUST | Y | assumed | — |
| Vector payload 4096-aligned | MUST | partial | assumed | write path; not all readers re-assert |
| Round-trip vectors + fingerprint gate | MUST | Y | assumed | — |
| Corrupt/truncated frames reject without panic | MUST | Y | assumed | table of cases |
| Fingerprint mismatch / wrong population reject | MUST | Y | assumed | — |
| Atomic sidecar replace: old mapping valid | MUST | Y | assumed | — |
| CE-003 IVF top-k equals brute force (thresholded) | MUST | Y | assumed | non-vacuous n ≥ ANN threshold |
| Mapped open does not own vector payload | MUST | partial | assumed | mmap validation / perf asserts |
| Cross-version (v1→v2) fixture corpus | SHOULD | **N** | — | **unknown** |
| Open p99 latency SLO | SHOULD | partial | env / ignore | **not** default CI MUST; host-specific figures in doc |

**Coverage accounting (S7):**

| Spec Section / surface | MUST-like | SHOULD-like | Tested (MUST) | Passing (assumed) | Divergent / unknown | Score estimate |
|------------------------|:---------:|:-----------:|:-------------:|:-----------------:|---------------------|---------------|
| IVF v2 wire + mmap | **~10** | **~3** | **~8–9** | ~8–9 (1 ignore tradeoff) | cross-version fixtures **unknown** | **~0.85–0.90 MUST (est.)** |

**Verdict:** Strongest **Pattern-3** surface. Below 0.95 mainly for missing versioned on-disk historical frames and SLO/ignore separation.

---

### S8 — Signal provenance / margins (multi-surface)

**Spec sources:** `docs/signal-provenance.md`, `docs/fusion-ranking.md`, compact DISC notes.  
**Primary tests:** `signal_provenance.rs` (**2** unit), cascade_planner contributors, cli_smoke field presence, compact omits fields.  
**Formal MUST list?** Field + signal enum table. Surface matrix incomplete.

| Spec / behavior (MUST-like) | Level | Tested? | Passing | Divergent / unknown |
|-----------------------------|:-----:|:-------:|:-------:|---------------------|
| Hits carry `signal`, `contributors`, `score`, `margin` | MUST | partial | assumed | core fusion unit; CLI smoke; not all surfaces goldens |
| Signal tier preserved under fusion (not re-labeled by rank) | MUST | Y | assumed | signal_provenance spoofed path |
| Margin ≥ 0; last = 0; ties = 0; within-channel only | MUST | Y | assumed | unit margins |
| Legacy/spoofed JSON re-derives signal from `kind` | MUST | Y | assumed | — |
| Compact omits contributors/score/margin; keeps signal code | MUST | Y | assumed | **Divergent** intentional |
| Native / GitHub / GitLab / agent / MCP / LSP all preserve full fields | MUST | partial | assumed | multi-surface matrix **incomplete** |
| RRF k=60 fusion formula exact | SHOULD | partial | assumed | fusion docs + epic ceiling; not clause-level numeric suite |

**Coverage accounting (S8):**

| Spec Section / surface | MUST-like | SHOULD-like | Tested (MUST) | Passing (assumed) | Divergent/unknown | Score estimate |
|------------------------|:---------:|:-----------:|:-------------:|:-----------------:|-------------------|---------------|
| Signal provenance / margins | **~6** | **~3** | **~4** | ~4 | multi-surface preserve matrix **unknown** | **~0.65–0.75 MUST (est.)** |

**Verdict:** Core math unit is solid; **surface-wide provenance matrix** is the gap (and compact DISC belongs in a future DISCREPANCIES.md).

---

## 3. Adjacent surfaces (summary only)

| Surface | MUST-like status | Tested snapshot | Score est. | Note |
|---------|------------------|-----------------|------------|------|
| **Lang extraction (13 langs)** | Full kind/span/order contract = **unknown** (no formal MUST list) | 13 presence cases via `assert_language_conformance` | Presence net **~0.9 (est.)**; full extract **n/a** | Name overclaims “conformance” (Pass 1/2) |
| **Ranking oracle** | Soft `must_include` + `max_rank` only | 12 `cases.json` rows | **~0.5** as absolute rank; **~0.9** as soft oracle | Not absolute RRF order conformance |
| **Hybrid RRF absolute order** | No formal MUST | metamorphic / learn weights only | **n/a** | Oracle problem; do not score as MUST |
| **Negative ledgers** | 7 cases listed in `negative-ledgers.md` | CLI/MCP/doctor/embed URL partial | **~0.70 (est.)** | Short ledger — extract fully |
| **Benchmark published numbers** | Agents.md honesty: trace to `baselines.md` or UNREPRODUCIBLE | baselines.md labels rows UNREPRODUCIBLE | Process **ok** if followed | **Not** a product runtime surface |
| **Peer HitKey parity CLI/core/LSP** | Peer equality | `no_embed_hit_key_parity` | High for drift | **Not** external oracle |

---

## 4. Overall gaps — MUST-like Score &lt; 0.95

**All eight primary surfaces score below the skill’s 0.95 MUST bar** (estimated). Ranked by (user risk × gap size):

| Rank | Surface | MUST Score est. | Dominant gap |
|:----:|---------|:---------------:|--------------|
| 1 | **S3 pattern subset** | **~0.55** | No supported-feature clause matrix; no DISC vs ast-grep |
| 2 | **S2 query grammar** | **~0.60** | Unsupported forms / fail modes untested as clauses |
| 3 | **S8 signal provenance surfaces** | **~0.65–0.75** | Multi-surface field preserve incomplete |
| 4 | **S6 index consistency / migrations** | **~0.70** | No old-DB migration golden corpus |
| 5 | **S1 machine envelope** | **~0.88** | Full hit dumps + multi-consumer freeze incomplete |
| 6 | **S4 symbol case-fold** | **~0.85** | Imports/FQN edges thinner than defs/callers |
| 7 | **S7 IVF v2** | **~0.85–0.90** | Cross-version fixtures; SLO ignore vs MUST |
| 8 | **S5 MCP product** | **~0.90** | Not full schema golden / official MCP suite |

**None** of these surfaces may be described as “conformant” under the skill rule without either raising coverage or documenting remaining MUST gaps in COVERAGE/DISCREPANCIES.

---

## 5. Surfaces where no formal MUST list exists

These need an **extraction project first** (numbered clauses, then tests) — scoring today is estimate-only:

| Surface | What exists | What’s missing for Pattern-4 |
|---------|-------------|------------------------------|
| Query grammar | Mode table + “not supported” bullets | `QG-NNN` IDs; negative cases per unsupported form |
| Pattern subset | Supported/unsupported prose | Exhaustive feature matrix + DISC-NNN vs ast-grep |
| Lang extraction | Hand presence tuples | Per-lang kind/span/import/call MUST set; grammar pin metadata |
| Hybrid ranking absolute order | RRF formula prose | Declared out of absolute-oracle scope; need soft-oracle + MR policy, not fake MUST |
| Full machine hit payloads | Envelope fields + shapes | Frozen hit-object schema per format |
| LSP method subset | README method table | Method × param MUST; UTF-16 positions |
| Official MCP protocol | Date pin + own process tests | External runner / full tools schema dump |
| Index migrations | SCHEMA_VERSION constant + code paths | Version-by-version migration MUST + golden DBs |

**Surfaces with the best partial formalization (still not clause IDs):** machine envelope field table, FailureBundle exit map, symbol SQL predicate table, IVF binary constants, negative ledger short list, index pragma table.

---

## 6. Aggregated findings for later beads (max 5 deep items)

> Not filed this pass. Themes only — max five, deep (not micro-nits).

### B1 — Number the small normative docs first (QG + envelope + negative ledger)
`QUERY_GRAMMAR.md`, machine schema notes, and `negative-ledgers.md` are short enough to become **QG-001… / MJ-001… / NL-001…** with one test tag per clause. Highest ROI: turns estimate scores into real Score ≥ 0.95 tracking.

### B2 — `pattern:` supported subset matrix + DISCREPANCIES vs ast-grep
Document intentional non-delegation as **DISC** entries; table-drive supported metavariable forms and exact-signature equality. Prevents “we’re ast-grep” product confusion and lifts S3 from ~0.55.

### B3 — Multi-surface provenance + compact DISC registry
Single matrix: which formats emit `signal` / `contributors` / `score` / `margin` (full vs compact codes). Ties S1 residual, S8, and compact validation into one COVERAGE row set.

### B4 — Index migration golden corpus (vN→7) + generation fail-closed cases
Checked-in minimal DBs for prior `user_version` values; explicit tests for cache fail-closed and sidecar fingerprint miss. Closes S6’s largest unknown.

### B5 — Promote ranking/MCP/machine oracles into a thin requirement-level report
Do not rebuild architecture (Pass 2). Add **case ID + level** fields and a markdown matrix emitter over existing fixtures (ranking `cases.json`, machine goldens, MCP tool list). Converts assumed-passing lists into a living COVERAGE artifact.

---

## 7. Score roll-up (estimates only)

| Surface | MUST-like (est. count) | SHOULD-like (est.) | Tested MUST (est.) | Score MUST (est.) | &lt; 0.95? |
|---------|:----------------------:|:------------------:|:------------------:|:-----------------:|:---------:|
| S1 Machine envelope | 16 | 6 | 14 | **~0.88** | **yes** |
| S2 Query grammar | 10 | 4 | 6 | **~0.60** | **yes** |
| S3 Pattern subset | 9 | 5 | 5 | **~0.55** | **yes** |
| S4 Symbol case-fold | 7 | 3 | 6 | **~0.85** | **yes** |
| S5 MCP product | 13 | 4 | 12 | **~0.90** | **yes** |
| S6 Index consistency | 11 | 4 | 7–8 | **~0.70** | **yes** |
| S7 IVF v2 | 10 | 3 | 8–9 | **~0.85–0.90** | **yes** |
| S8 Signal provenance | 6 | 3 | 4 | **~0.65–0.75** | **yes** |

**Portfolio view:** Best estimated MUST coverage ≈ **MCP product contracts** and **IVF wire** and **machine envelope**. Worst ≈ **pattern subset** and **query unsupported matrix**. Overall product remains **oracle/contract culture (Pass 1 maturity 5/10)** — not a ≥95% MUST conformance program.

**Passing column caveat:** All “passing” cells are **assumed from test existence**. This pass did not execute `cargo test` / proof-pack.

---

## 8. Worst 3 surfaces (summary for handoff)

| # | Surface | MUST Score est. | Why worst |
|---|---------|:---------------:|-----------|
| **1** | **Native `pattern:` subset** | **~0.55** | Supported behaviors are smoke-level; no feature completeness matrix; intentional ast-grep divergence not formalized as DISC clauses |
| **2** | **Query prefix grammar** | **~0.60** | Happy prefixes covered; **unsupported forms and fail modes** lack tests despite being half the normative doc |
| **3** | **Signal provenance across surfaces** | **~0.65–0.75** | Core margin/spoof unit tests exist; multi-surface preserve + compact DISC accounting incomplete for agent trust |

Close runners-up: **S6 index migrations (~0.70)** if migration goldens are prioritized over provenance.

---

## 9. Out of scope (confirmed)

- No implementation of clauses, harness shell, goldens, or DISC/COVERAGE files  
- No `br` beads filed  
- No commits  
- No architecture redesign (Pass 2 domain)  
- Did not re-run proof-pack or full test suite (Passing = assumed)

---

## 10. Report card

| Item | Value |
|------|--------|
| **Deliverable** | `/Users/aditya/Developer/ast-sgrep/tests/artifacts/conformance-audit/PASS3_COVERAGE_ACCOUNTING.md` |
| **Surfaces fully tabled** | **8** (+ adjacent summary) |
| **Formal numbered MUST lists** | **0** (partial field/predicate tables only) |
| **Surfaces ≥ 0.95 MUST (est.)** | **0** |
| **Worst 3** | pattern subset ~0.55 · query grammar ~0.60 · signal provenance ~0.65–0.75 |
| **Best 3 (est.)** | MCP product ~0.90 · IVF ~0.85–0.90 · machine envelope ~0.88 |
| **Beads filed** | none (per mission) |
| **Precision** | All scores **estimates**; counts are behavior enumerations, not exhaustive clause corpora |


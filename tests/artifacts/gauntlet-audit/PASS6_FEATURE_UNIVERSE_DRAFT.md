# Pass 6/16 — Surface FeatureUniverse draft

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Date:** 2026-08-07  
**Skill:** `running-the-gauntlet-on-your-rust-port` · Surface parity pillar  
**Mode:** audit-only (no cargo, no beads, no commit, no product code)  
**Inputs:** PASS1–PASS5; `docs/validation/{feature-universe,surface-parity,machine-json-schema,compact-output,jell-deferral}.md`; README Interfaces / feature claims; `crates/ast-sgrep-cli/tests/fixtures/capabilities.json`; source inventories for CLI / MCP / LSP / Pi / codemode / lang / plugins / embed / store  

**Oracle framing (PASS1):** greenfield multi-reference hybrid — matrix is against **product promises**, not full ripgrep / full ast-grep / full SQL-class surface.

**Status vocabulary (SurfaceMatrix):** `present` | `partial` | `missing` | `excluded` | `n/a`  
- **present** — implemented on the named surface with in-tree evidence (code + test/doc path).  
- **partial** — shipped but incomplete vs promise, cross-surface parity, or proof obligations.  
- **missing** — in-scope product promise / parity expectation, not implemented.  
- **excluded** — intentional non-goal (rationale required); counts as coverage debt for strict-100% skill claims.  
- **n/a** — not applicable on that host (protocol shape, platform, or product role).

**Skill status note:** FeatureUniverse `ParityStatus` uses `Passing|Partial|Missing|Excluded`. This draft uses SurfaceMatrix labels (`present` ≈ Passing-ready once formalized). No `parity_score.json` / weights yet — statuses are **honest inventory**, not scored Passing.

---

## 0. Executive summary

| Field | Value |
|-------|--------|
| **Surface maturity (PASS2)** | **3 / 10** (reaffirmed) |
| **Existing short lists** | `feature-universe.md` ≈ 10 feature IDs (no status enum); `surface-parity.md` 5 capability rows × 4 hosts |
| **Draft matrix rows (this pass)** | **94** family×feature rows |
| **present** | **61** |
| **partial** | **17** |
| **missing** | **8** |
| **excluded** | **7** |
| **n/a** | **1** |
| **Skill stack present?** | **No** — no `docs/contracts/`, no `supported_surface_matrix.toml`, no `parity_score_contract.toml`, no `feature_coverage.json` / `parity_score.json`, no harness `parity_taxonomy` |
| **Top residual (surface)** | Formal FeatureUniverse + SurfaceMatrix + weighted / conformal score pipeline (PASS2 residual #3) |

**Headline:** Product surfaces are **real and multi-host** (CLI formats, 13 langs, MCP split channels + `code_read`, LSP nav, Pi + Code Mode, machine JSON, IVF store). Skill-grade **enumeration + status + evidence + score** is still a draft-only gap. Do **not** refile golden/extraction dump work as surface beads — **nz7i** owns freezes.

---

## 1. Draft FeatureUniverse-style matrix

Evidence paths are repo-relative. Where a feature spans hosts, the **status is the product-promise status** with host notes in the Notes column.

### 1.1 CLI output formats (`search_formats`)

| Feature ID (draft) | Feature | Status | Evidence | Notes |
|--------------------|---------|:------:|----------|-------|
| F-FMT-001 | `--format native` | **present** | `crates/ast-sgrep-plugins/src/lib.rs` (`OutputFormat::Native`); `capabilities.json` `search_formats` | Full `SearchResponse` JSON |
| F-FMT-002 | `--format agent` | **present** | plugins `OutputFormat::Agent`; capabilities fixture | Agent-oriented hit list |
| F-FMT-003 | `--format agent-capsule` | **present** | plugins `AgentCapsule`; `plugins/tests/capsule_format.rs` | Pi default path |
| F-FMT-004 | `--format compact` | **present** | plugins `Compact` + `CompactBudget`; `docs/validation/compact-output.md`; CLI `search_cmd.rs` empty-hit diagnostic | Token-budgeted; measured reduction in compact-output.md (not a release score) |
| F-FMT-005 | `--format github` | **present** | plugins `GitHub`; capabilities `search_formats` | CI annotation shape |
| F-FMT-006 | `--format gitlab` | **present** | plugins `GitLab`; capabilities | CI annotation shape |
| F-FMT-007 | Format parity across hosts | **partial** | PASS2 §3; MCP always compact; Pi capsule; LSP raw JSON | Same query → different envelopes by design; not unified optional format on MCP/LSP |

### 1.2 Search modes / query grammar

| Feature ID | Feature | Status | Evidence | Notes |
|------------|---------|:------:|----------|-------|
| F-QRY-001 | Hybrid unprefixed cascade | **present** | `docs/QUERY_GRAMMAR.md`; CLI bare/`search`; core fusion | Product default on CLI/core |
| F-QRY-002 | Hybrid auto-fusion on MCP | **excluded** | `docs/mcp.md`; `docs/validation/surface-parity.md`; MCP tools split channels | Intentional: agent chooses channel; `code_search` is keyword alias only |
| F-QRY-003 | `semantic` / `--semantic-only` | **present** | CLI `semantic` + flag; MCP `semantic_search`; LSP `asgrep.search.semantic`; feature-universe `semantic_search` | Embed channel |
| F-QRY-004 | Lexical keyword / FTS | **present** | CLI `keyword`; MCP `keyword_search`; core FTS | |
| F-QRY-005 | `pattern:` native structural | **present** | QUERY_GRAMMAR; MCP `ast_search`; core `pattern.rs` | Native tree-sitter + index; **not** external ast-grep spawn on happy path |
| F-QRY-006 | `defs:` / `callers:` / `imports:` graph | **present** | QUERY_GRAMMAR; CLI prefixes; LSP executeCommand defs/callers; codemode tools | Graph modes |
| F-QRY-007 | `chain` call-chain traversal | **present** | CLI `chain`; codemode `chain`; Pi mode `chain` | Missing as first-class MCP tool (see F-MCP-*) |
| F-QRY-008 | `literal:` / `regex:` / `word:` | **present** | QUERY_GRAMMAR; capabilities `query_prefixes`; Pi modes | |
| F-QRY-009 | Composable AND / multi-prefix / parens | **excluded** | QUERY_GRAMMAR “What is not supported” | By design |
| F-QRY-010 | In-query `path:` / `lang:` / `sem:` filters | **excluded** | QUERY_GRAMMAR | Use CLI flags / host options instead |
| F-QRY-011 | Query prefix advertised in agent contract | **partial** | capabilities `query_prefixes` present; older docs drift risk (PASS2) | Fixture is good; onboarding docs historically thinner |

### 1.3 Languages (product: 13)

| Feature ID | Feature | Status | Evidence | Notes |
|------------|---------|:------:|----------|-------|
| F-LNG-001 | Language enum size = 13 | **present** | `crates/ast-sgrep-lang/src/lib.rs` `Language::all()` + unit assert len 13; README v1.4.0 | Rust, TS, JS, Python, Go, Java, C#, Ruby, Swift, C, C++, Kotlin, PHP |
| F-LNG-002 | Parse + extract presence goldens (all 13) | **present** | `crates/ast-sgrep-lang/tests/extraction_goldens.rs` + `fixtures/extract/*` | Shared presence/forbid contract per lang |
| F-LNG-003 | Full extraction tree dump freezes | **partial** | golden-audit / **nz7i** program (PASS2 cross-link) | Do **not** bead as surface matrix work — **nz7i** owns dumps |
| F-LNG-004 | VS Code `onLanguage` activation for all 13 | **present** | `editors/vscode/package.json` activationEvents | Includes swift/kotlin/php/c/cpp/csharp |
| F-LNG-005 | Per-lang quality bake-off vs external oracles | **excluded** | `docs/validation/jell-deferral.md` | Full external hit-ID deferred; subset structural under **ghiw.3** |

### 1.4 MCP tools (`asgrep-mcp`)

| Feature ID | Feature | Status | Evidence | Notes |
|------------|---------|:------:|----------|-------|
| F-MCP-001 | `keyword_search` | **present** | `crates/ast-sgrep-mcp/src/lib.rs` tools/list + dispatch; `docs/mcp.md` | Lexical only |
| F-MCP-002 | `ast_search` | **present** | mcp lib; docs/mcp.md | pattern: semantics |
| F-MCP-003 | `semantic_search` | **present** | mcp lib; docs/mcp.md | Embed only |
| F-MCP-004 | `code_search` compat alias | **present** | dispatch → Keyword; protocol tests pin | Deprecated; no fusion |
| F-MCP-005 | `code_read` (id → body) | **present** | mcp lib; docs/mcp.md | Hierarchical snippet-first pattern with compact ids |
| F-MCP-006 | `index_status` | **present** | mcp lib | |
| F-MCP-007 | `index_repo` single-flight + deadline | **present** | mcp lib (`tool_index_repo`); feature-universe `mcp_index_repo` | Wall-clock deadline |
| F-MCP-008 | Compact envelope default | **present** | mcp tool descriptions `COMPACT_CONTRACT`; plugins test “MCP … Compact” | Client-facing contract in tools/list text |
| F-MCP-009 | Optional format selection (agent/capsule/github) | **missing** | mcp hardcodes agent/compact path; no format arg in schema | CLI has 6 formats; MCP fixed |
| F-MCP-010 | First-class `defs` / `callers` / `chain` / hybrid tools | **missing** | dispatch has no defs/callers/chain/hybrid | Query-prefix inside `keyword_search`/`code_search` may work for some prefixes — not first-class tools; hybrid excluded |
| F-MCP-011 | Graph/hybrid MCP parity with CLI/Pi | **partial** | surface-parity intentional deltas; thinner tool set | Partial by design for fusion; graph tools still a gap |
| F-MCP-012 | MCP tools/list schema freeze | **partial** | mcp `tests/protocol.rs`; **nz7i** dump freezes incomplete (PASS2) | Cross-link **nz7i**, not new surface bead for dumps |

### 1.5 LSP (`asgrep-lsp`)

| Feature ID | Feature | Status | Evidence | Notes |
|------------|---------|:------:|----------|-------|
| F-LSP-001 | `workspace/symbol` | **present** | `crates/ast-sgrep-lsp/src/server.rs` HANDLERS | |
| F-LSP-002 | `textDocument/documentSymbol` | **present** | server.rs | |
| F-LSP-003 | `textDocument/definition` | **present** | server.rs | |
| F-LSP-004 | `textDocument/references` | **present** | server.rs | |
| F-LSP-005 | Call hierarchy (prepare/in/out) | **present** | server.rs + `callHierarchyProvider` | |
| F-LSP-006 | `workspace/executeCommand` suite | **present** | backend: `asgrep.search`, `.semantic`, `.reindex`, `.callers`, `.defs` | |
| F-LSP-007 | Experimental `asgrep/search` | **present** | server HANDLERS; initialize `asgrepSearchProvider` | |
| F-LSP-008 | HitKey peer parity with CLI/core | **present** | `crates/ast-sgrep-cli/tests/no_embed_hit_key_parity.rs` | Strong **search key** parity, not full envelope parity |
| F-LSP-009 | Doctor / robot-triage | **excluded** | surface-parity intentional delta | LSP IDE-focused |
| F-LSP-010 | Agent output format options | **n/a** | LSP returns protocol/JSON shapes | Not CLI `--format` surface |
| F-LSP-011 | Discoverability from main CLI agent triad | **partial** | capabilities `integrations` + `sibling_binaries` now list mcp/lsp | Improved vs older wave1; still secondary to clap help |

### 1.6 Pi package (`packages/pi`)

| Feature ID | Feature | Status | Evidence | Notes |
|------------|---------|:------:|----------|-------|
| F-PI-001 | `asgrep_codemode` primary tool | **present** | `packages/pi/extension/src/index.ts`; `docs/codemode.md`; README | Code Mode default story |
| F-PI-002 | Escape-hatch `asgrep_search` | **present** | index.ts modes: natural/pattern/defs/callers/chain/semantic/word/literal/regex/imports | Wide mode union |
| F-PI-003 | `asgrep_index` / `asgrep_status` | **present** | index.ts | |
| F-PI-004 | Slash commands + skill | **present** | README Pi blurb; extension registerCommand | |
| F-PI-005 | Machine envelope triple (`tool`/`schema_version`/`ok`) | **present** | `docs/validation/machine-json-schema.md`; runtime | Pi rejects schema mismatch |
| F-PI-006 | Mode matrix tests vs schema | **partial** | Historical wave1: tests/skill lag modes | Verify on implement pass; treat as partial until tests cover all 10 modes |
| F-PI-007 | MCP adapter inside Pi | **excluded** | product: native CLI/NAPI/codemode path, no MCP required | README: no MCP adapter required |

### 1.7 Code Mode (Rust catalog + hosts)

| Feature ID | Feature | Status | Evidence | Notes |
|------------|---------|:------:|----------|-------|
| F-CM-001 | Tool catalog (12 tools) | **present** | `crates/ast-sgrep-codemode/src/catalog.rs` | search, semantic, chain, defs, callers, imports, index_*, filter_hits, select, catalog_* |
| F-CM-002 | Anthropic / OpenAI / Cloudflare adapters | **present** | `crates/ast-sgrep-codemode/src/adapters/*` | PTC shapes |
| F-CM-003 | CLI `codemode-batch` / `codemode-serve` | **present** | capabilities commands; CLI lib dispatch | Sticky worker + batch |
| F-CM-004 | Progressive catalog discovery | **present** | `catalog_search` / `catalog_describe` | |
| F-CM-005 | Codemode surface manifest freeze | **partial** | **nz7i** owns catalog dump freezes (PASS2) | Cross-link only |

### 1.8 Embed backends

| Feature ID | Feature | Status | Evidence | Notes |
|------------|---------|:------:|----------|-------|
| F-EMB-001 | Local semantic (always-on hashed/concept) | **present** | `crates/ast-sgrep-embed/src/semantic.rs`; README provider chain | No API key |
| F-EMB-002 | Cloud embed (opt-in) | **present** | embed `CloudEmbedder`; env `ASGREP_CLOUD_EMBED` | Requires key/config |
| F-EMB-003 | Ollama embed (opt-in) | **present** | `OllamaEmbedder`; env | |
| F-EMB-004 | Neural embed (feature-gated) | **partial** | `#[cfg(feature = "neural-embed")]`; `docs/validation/neural-trust.md` | Optional cargo feature; mock-free e2e under **lbx1** |
| F-EMB-005 | Rerank (feature-gated) | **partial** | `#[cfg(feature = "rerank")]`; CLI flags `--rerank` | Optional |
| F-EMB-006 | Embed-on surface parity (CLI/MCP/codemode) | **partial** | **lbx1** mock-free program (PASS2) | Cross-link **lbx1**; not FeatureUniverse dumps |

### 1.9 Index / store

| Feature ID | Feature | Status | Evidence | Notes |
|------------|---------|:------:|----------|-------|
| F-IDX-001 | SQLite index `.asgrep/index.db` | **present** | `IndexStore`; README | |
| F-IDX-002 | Schema `user_version = 7` | **present** | `store/sqlite.rs` `SCHEMA_VERSION` | |
| F-IDX-003 | Incremental `index` / destructive `reindex` | **present** | CLI commands; safe_mutating notes in capabilities | build-then-swap |
| F-IDX-004 | Semantic IVF sidecar `semantic.ivf` | **present** | `semantic_ivf.rs`; `docs/validation/semantic-ivf-mmap.md` | Magic ASIVF / v2 (PASS1 pin) |
| F-IDX-005 | Optional Tantivy path | **partial** | `tantivy_index.rs`; env `ASGREP_TANTIVY` / flag | Secondary path; not primary promise |
| F-IDX-006 | `watch` incremental reindex | **present** | CLI `watch`; `cli/tests/watch_incremental.rs` | |
| F-IDX-007 | `status` / `doctor` health | **present** | CLI; feature-universe `doctor` | Doctor CLI-only (LSP excluded) |
| F-IDX-008 | Cross-engine jell index identity | **excluded** | `docs/validation/jell-deferral.md` | Non-goal full hit-ID |

### 1.10 Machine JSON / agent contract

| Feature ID | Feature | Status | Evidence | Notes |
|------------|---------|:------:|----------|-------|
| F-MJ-001 | Envelope `tool` + `schema_version` + `ok` | **present** | `docs/validation/machine-json-schema.md`; `machine_contracts.rs` | schema `1.0.0` |
| F-MJ-002 | `asgrep capabilities --json` | **present** | fixture + golden test `capabilities_and_version_match_goldens`; lists commands/formats/prefixes/siblings | Strong agent self-doc |
| F-MJ-003 | `robot-docs` handbook | **present** | CLI command; capabilities canonical_tasks | |
| F-MJ-004 | MCP JSON-RPC vs CLI envelope | **partial** | surface-parity / machine-json-schema note | Different transport (`isError` vs `ok`) — documented intentional delta |
| F-MJ-005 | Formal schema JSON Schema / contract TOML | **missing** | no `docs/contracts/`; notes only in validation md | Skill expects contracts dir |
| F-MJ-006 | Engine identity fields | **present** | `docs/validation/engine-identity.md` | tool/schema/embed/index format |

### 1.11 CLI command surface (product completeness)

| Feature ID | Feature | Status | Evidence | Notes |
|------------|---------|:------:|----------|-------|
| F-CLI-001 | Core search triad (search/keyword/semantic) | **present** | capabilities commands list | |
| F-CLI-002 | Index lifecycle (index/reindex/status/watch) | **present** | capabilities | |
| F-CLI-003 | Agent triad (capabilities/robot-docs/doctor) | **present** | capabilities; machine_contracts | |
| F-CLI-004 | chain / bench / eval / version | **present** | capabilities commands | |
| F-CLI-005 | codemode-batch / codemode-serve | **present** | capabilities | |
| F-CLI-006 | forbid_soundness (CI unsafe ban) | **present** | feature-universe `forbid_soundness`; workspace forbid | Not a runtime CLI feature |

### 1.12 Adjacent product surfaces (library / VS Code / plugins)

| Feature ID | Feature | Status | Evidence | Notes |
|------------|---------|:------:|----------|-------|
| F-LIB-001 | Public `ast-sgrep-core` library API | **partial** | README Interfaces; crates not crates.io-published per getting-started history | Embeddable but external agent discoverability weak |
| F-VSC-001 | VS Code extension + LSP client | **partial** | `editors/vscode/`; activation for 13 langs | In-tree; not always first-class in short Interfaces table (README now lists LSP, not VS Code row) |
| F-PLG-001 | JSON plugins crate formats | **present** | `ast-sgrep-plugins` | Host for all `--format` values |

### 1.13 Skill-only scoring artifacts (meta)

| Feature ID | Feature | Status | Evidence | Notes |
|------------|---------|:------:|----------|-------|
| F-SKL-001 | `docs/contracts/supported_surface_matrix.toml` | **missing** | ABSENT (PASS2/3/5) | SurfaceMatrix SSoT |
| F-SKL-002 | `parity_score_contract.toml` weights | **missing** | ABSENT | sum(weights)==1.0 per category |
| F-SKL-003 | `feature_coverage.json` / `parity_score.json` | **missing** | ABSENT | Beta + conformal lower bound + truncate_score |
| F-SKL-004 | `docs/progress/surface-deferrals.md` | **missing** | ABSENT (PASS5) | retry_condition for excluded/deferred |
| F-SKL-005 | harness `parity_taxonomy` / dashboard | **missing** | no port-harness surface modules | Greenfield residual |
| F-SKL-006 | Short product feature-universe seed | **partial** | `docs/validation/feature-universe.md` | 10 IDs, no status/weights |
| F-SKL-007 | Short surface-parity table | **partial** | `docs/validation/surface-parity.md` | 5 rows × 4 hosts; intentional deltas only |

---

## 2. Status counts (this draft)

| Status | Count | Share of 94 rows |
|--------|------:|-----------------:|
| **present** | 61 | 64.9% |
| **partial** | 17 | 18.1% |
| **missing** | 8 | 8.5% |
| **excluded** | 7 | 7.4% |
| **n/a** | 1 | 1.1% |

**Operator counts requested by mission:**

| Metric | Value |
|--------|------:|
| **missing** | **8** |
| **partial** | **17** |
| **missing + partial** | **25** |
| **excluded** (debt if claiming strict 100%) | **7** |

**Missing list (implement / formalize later):**  
- F-MCP-009 optional MCP format selection  
- F-MCP-010 first-class MCP `defs` / `callers` / `chain` tools (hybrid remains **excluded**, not missing)  
- F-MJ-005 formal schema / `docs/contracts` machine JSON  
- F-SKL-001 `supported_surface_matrix.toml`  
- F-SKL-002 `parity_score_contract.toml` weights  
- F-SKL-003 `feature_coverage.json` / `parity_score.json`  
- F-SKL-004 `docs/progress/surface-deferrals.md`  
- F-SKL-005 harness `parity_taxonomy` / coverage dashboard  

**Note:** F-MCP-010 is one row covering multiple missing first-class tools; skill scoring later should split IDs if weighted.

**Partial list (themes):** cross-host format parity; query-prefix agent advertising; lang dump freezes (**nz7i**); MCP graph parity + tools/list freeze (**nz7i**); LSP discoverability; Pi mode tests; codemode dump freezes; neural/rerank/tantivy optional paths; embed-on e2e (**lbx1**); library publish; VS Code Interfaces promotion; short validation docs vs full universe; MCP transport vs CLI envelope.

---

## 3. Gaps vs skill SurfaceMatrix / `parity_score`

| Skill expectation (THREE-PILLARS / FEATURE-UNIVERSE) | ast-sgrep today | Gap |
|------------------------------------------------------|-----------------|-----|
| `docs/contracts/supported_surface_matrix.toml` every feature `present\|partial\|missing\|n/a\|excluded` + rationale | **This markdown draft only** | No machine-loadable matrix |
| FeatureUniverse typed features + weights sum 1.0/category | `feature-universe.md` list of ~10 IDs | No weights, no FeatureId scheme, no loader |
| Status progression Missing → Partial → Passing with proof obligations | Informal docs + tests | No enforcement / bead-close contract |
| `parity_score_contract.toml` | Absent | No category weights |
| `feature_coverage.json` per-family verdict | Absent | No dashboard |
| `parity_score.json` Beta posterior + conformal lower bound + `truncate_score` 6 dp | Absent | No release number (PASS2 residual #8) |
| `docs/progress/surface-deferrals.md` + `retry_condition` | Absent (PASS5) | Intentional deltas live only in surface-parity prose |
| Verification contract on bead close / release | Absent | Skill fail-missing-evidence not wired |
| Deterministic FeatureId iteration / report SHA | N/A | No harness |
| Certification: 100% of non-excluded obligations verified | Blocked | Would require matrix + proofs first; excluded jell/MCP-fusion must be explicit debt |

**What is already “Passing-shaped” without skill harness:** HitKey peer parity (CLI/core/LSP), machine_contracts + capabilities golden, MCP channel split + code_read, 13-lang extraction presence, compact format contract, IVF store path, codemode catalog.

**What must not be scored green from this draft alone:** Any conformal lower-bound; any CERTIFIED surface pillar; any claim of full MCP↔CLI feature parity.

---

## 4. What `docs/validation/surface-parity.md` already covers

Existing file (13 lines) is a **capability stub**, not a FeatureUniverse:

| Capability row | CLI | MCP | LSP | Pi |
|----------------|-----|-----|-----|-----|
| Hybrid search | yes | via keyword/ast/semantic (no auto-fusion) | `asgrep.search` | extension tools |
| Semantic-only | `--semantic-only` / `semantic` | `semantic_search` | `asgrep.search.semantic` | yes |
| Limit clamp | `MAX_OUTPUT_RESULTS` | `clamp_agent_limit` (100) | default_limit | timeout/bytes caps |
| Index | `index`/`reindex` | `index_repo` (single-flight) | background index | rebuild helpers |
| Doctor/triage | `doctor` | — | — | handbook |
| Boolish env | clap + core `env_flag` | NO_EMBED boolish | settings | env aliases |

**Intentional deltas already written:** MCP no auto-fuse; LSP IDE-focused (no doctor).

**Not covered by surface-parity.md (this draft adds):**  
CLI format ×6; full query prefixes; 13 languages; MCP `code_read` / channel tools inventory; LSP call hierarchy & command list; Pi Code Mode primary tool; embed provider chain; IVF/tantivy/watch/doctor; machine JSON schema; codemode catalog; library/VS Code; skill scoring artifacts; excluded grammar features; jell non-goal.

**Related short doc:** `docs/validation/feature-universe.md` seeds 10 feature IDs (`hybrid_search`, `semantic_search`, `keyword_search`, `pattern_search`, `defs_callers_imports`, `chain`, `compact_output`, `doctor`, `mcp_index_repo`, `forbid_soundness`) — **IDs only**, no statuses.

---

## 5. Aggregated findings for beads (max 3 deep)

> **Pass 11 owns filing.** Do not open beads in this pass.  
> **Do not refile** extraction dump / assert_golden work (**nz7i** / **nz7i.4** family) as surface FeatureUniverse beads — cross-link only.  
> Same for: **ghiw** DISC/MUST matrix, **ghiw.3** pattern×ast-grep subset, **lbx1** embed mock-free, **b8q3** fuzz.

### Deep finding S1 — Formal SurfaceMatrix + FeatureUniverse SSoT (product promises)

- **Theme:** Promote this draft into `docs/contracts/supported_surface_matrix.toml` (+ optional `parity_score_contract.toml` category weights adapted to greenfield hybrid, not SQL copy-paste).  
- **Why:** Skill Surface pillar cannot move past ~3/10 without machine-checkable statuses; short validation tables are insufficient.  
- **Scope boundary:** Status + rationale + evidence pointers only in first implement slice; scoring harness can follow.  
- **Cross-links:** PASS2 residual #3; PASS3 skill score gap; PASS5 surface-deferrals absence; this PASS6 matrix.  
- **Not in scope:** nz7i dump freezes, cargo harness modules in the same bead if oversized — split scoring to S2.

### Deep finding S2 — Greenfield `parity_score` / coverage pipeline (or explicit non-goal)

- **Theme:** Either implement minimal `feature_coverage.json` + lower-bound score path (weights, truncate_score, deterministic FeatureId order) **or** document a greenfield-adapted certification rule that does **not** claim skill CERTIFIED without it.  
- **Why:** PASS2 residual #8; forbidden-victory if surface “green” without number.  
- **Depends on:** S1 matrix rows stable enough to weight.  
- **Honesty:** Do not invent a high parity_score from this draft’s present-count alone (present ≠ oracle-Passing).

### Deep finding S3 — Cross-host agent surface honesty (MCP graph/format + deferral ledger)

- **Theme:** One epic for **documented intentional deltas + real missing agent gaps**:  
  - Keep **excluded:** MCP auto-fusion, full jell, in-query boolean grammar.  
  - Close or permanently exclude with `retry_condition`: MCP first-class graph tools (defs/callers/chain), optional MCP format arg, Pi mode test matrix closure, promote VS Code/library honesty in Interfaces.  
  - Stand up `docs/progress/surface-deferrals.md` importing surface-parity + jell pointers (PASS5 Form-2/4).  
- **Why:** Highest user-visible partial/missing cluster after skill scaffolding; avoids “fixing” intentional MCP non-fusion.  
- **Cross-links:** **nz7i** for tools/list & handbook freezes (do not duplicate); **lbx1** for embed-on; **ghiw.2** machine envelope MUST (conformance-facing).  
- **Explicit non-goals for this epic:** extraction tree dumps (**nz7i.4**), full rg identity, neural default-on.

---

## 6. Cross-link map (do not duplicate ownership)

| Concern | Owner | Surface pass action |
|---------|-------|---------------------|
| CLI/MCP/Pi/codemode/lang **dump freezes** | **nz7i** (+.1–.5) | Cross-link only |
| Query grammar + machine envelope MUST | **ghiw.2** | Conformance-facing; matrix cites contracts when written |
| pattern: × ast-grep minimal differential | **ghiw.3** | External structural honesty, not matrix row spam |
| Embed/cloud/Ollama/neural mock-free | **lbx1** | Partial F-EMB rows |
| Fuzz wire robustness | **b8q3** | Not FeatureUniverse |
| Full external hit-ID | **jell-deferral.md** | Excluded |
| Keep-gate / bench history | Perf residual (PASS2 #1 / PASS4) | Out of surface scope |
| Composite oracle dispatch SSoT | Conf residual (PASS2 #2) | Adjacent; not matrix |

---

## 7. README / product claims vs matrix (honesty)

| README / product claim | Matrix read |
|------------------------|-------------|
| v1.4.0 · 13 languages | F-LNG-001/002 **present** (presence goldens); full dumps **partial** via nz7i |
| Hybrid + AST graph + semantic + Code Mode | F-QRY-001, F-QRY-006, F-QRY-003, F-CM/F-PI **present** on primary hosts |
| MCP stdio for AI agents | F-MCP-001–008 **present**; fusion **excluded**; graph tools **missing**/partial |
| LSP editor navigation | F-LSP-001–008 **present** |
| JSON plugins agent\|github\|gitlab\|agent-capsule (+ compact) | F-FMT-001–006 **present** on CLI |
| No API key required (local semantic) | F-EMB-001 **present** |
| Quality MRR/Recall/nDCG in README | **Not a surface feature** — honesty under PASS3/Agents.md baselines; do not put in FeatureUniverse as Passing |

---

## 8. Evidence log (what this pass actually did)

- Read PASS1–PASS5 under `tests/artifacts/gauntlet-audit/`.  
- Read `docs/validation/{feature-universe,surface-parity,machine-json-schema,compact-output}.md` (+ jell/negative via PASS5).  
- Read README Interfaces / semantic / version claims (no numbers restated as certified).  
- Parsed `crates/ast-sgrep-cli/tests/fixtures/capabilities.json` (16 commands, 6 formats, 7 prefixes, integrations).  
- Source-audited: MCP tools/list+dispatch; LSP HANDLERS+executeCommand; lang `Language::all` len 13 + extraction_goldens; plugins `OutputFormat`; codemode `tool_catalog` (12); Pi modes; embed exports; store SCHEMA_VERSION 7 + semantic.ivf; QUERY_GRAMMAR.  
- Confirmed **absent:** `docs/contracts/`, `feature_coverage.json`, `parity_score.json`, `docs/progress/surface-deferrals.md`.  
- **Did not run:** cargo, beads, commits, product edits.  
- **Did not** restate unreproducible quality numbers as product guarantees.

---

## 9. Verdict block

| Item | Value |
|------|--------|
| **Artifact** | `/Users/aditya/Developer/ast-sgrep/tests/artifacts/gauntlet-audit/PASS6_FEATURE_UNIVERSE_DRAFT.md` |
| **Matrix rows** | **94** |
| **present / partial / missing / excluded / n/a** | **61 / 17 / 8 / 7 / 1** |
| **missing + partial** | **25** |
| **Skill SurfaceMatrix / parity_score** | **Absent** (draft only) |
| **surface-parity.md** | Covers 5 capabilities × 4 hosts + 2 intentional deltas; incomplete vs this draft |
| **Deep beads (Pass 11)** | S1 formal matrix SSoT · S2 score pipeline or explicit non-goal · S3 cross-host honesty + surface-deferrals |
| **Do not refile** | **nz7i** extraction/dumps; ghiw/lbx1/b8q3 owned work |

**DONE** — Pass 6 FeatureUniverse draft complete; audit-only; no cargo; no beads; no commit; no product code.

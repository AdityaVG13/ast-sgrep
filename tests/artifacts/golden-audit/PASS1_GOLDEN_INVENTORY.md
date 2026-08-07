# Pass 1 — Golden Inventory & Confidence Matrix

**Repo:** `/Users/aditya/Developer/ast-sgrep`  
**Branch:** `perf/software-optimization` (inventory only; no implementation)  
**Date:** 2026-08-07  
**Skill:** `testing-golden-artifacts`  
**Scope:** freeze-style expected-output tests, named golden/snapshot artifacts, and false friends. No `assert_golden` work, no beads, no CI changes.

**Search coverage:** `crates/**/tests/**`, crate `src/**` unit tests, `tests/**`, `packages/pi/**/test/**`, `packages/pi/scripts/**`, `benchmarks/results/**`, `docs/validation/**`. Patterns: `golden`, `snapshot`, `insta`, `UPDATE_GOLDENS`, `*.snap`, `*.golden`, `include_str!` fixtures, oracle/must_include, machine envelopes. ZeroStack was busy (`machine_permit_busy`); inventory used shell/`rg`.

---

## 1. Executive summary

Golden-artifact testing in this repo is **partially mature on machine contracts, immature as a suite**.

- **Strongest frozen artifacts:** CLI machine JSON goldens under `crates/ast-sgrep-cli/tests/fixtures/` (full equality after version scrub) plus key-shape fixtures; ranking oracle JSON; extraction "goldens" that are really **hand presence tuples**, not dump files; plugins formatters with dense `assert_eq!` on synthetic responses; Pi release-contract structural freeze.
- **Missing core infrastructure:** no `insta`, no `*.snap` / `*.golden` files, no `UPDATE_GOLDENS`, no shared `assert_golden` helper, no scrubber registry, no `PROVENANCE.md` for test goldens, no `*.actual` gitignore rule, no CI golden-diff artifact upload.
- **Dominant style:** field-by-field / structural / must_include oracles and metamorphic self-compares -- good for correctness, weak for large structured dumps (full extract trees, full search JSON pages, handbook markdown, MCP tool schemas).
- **Benchmarks:** `benchmarks/results/baselines.md` is a **published historical freeze** tagged UNREPRODUCIBLE; it is not a CI golden test (and must not be re-quoted without that provenance per `Agents.md`).
- **Maturity score (subjective):** ~3/10 infrastructure, ~6/10 machine-contract coverage, ~4/10 overall golden-artifact practice.

---

## 2. Inventory table

Columns: Path | Kind | Pattern | Deterministic? | Platform-dependent? | Volatility 1-5 | Strategy recommended | Notes

| Path | Kind | Pattern | Det? | Plat? | Vol | Strategy | Notes |
|------|------|---------|------|-------|-----|----------|-------|
| `crates/ast-sgrep-cli/tests/machine_contracts.rs` + `tests/fixtures/capabilities.json` | JSON fixture compare (true golden equality) | scrubbed exact | Y | N | 3 | scrubbed exact | `capabilities_and_version_match_goldens`; `version` field replaced with `"<version>"` before `assert_eq!` |
| `crates/ast-sgrep-cli/tests/fixtures/envelopes.json` + machine_contracts operational/usage/version tests | JSON fixture compare | scrubbed exact | Y | N | 2 | scrubbed exact | Placeholders `<command>`, `<message>`, `<version>`; failure envelopes byte-stable after scrub |
| `crates/ast-sgrep-cli/tests/fixtures/machine_shapes.json` + shape tests in machine_contracts | JSON fixture compare (key sets only) | structural | Y | N | 3 | structural | Sorted object keys vs frozen key arrays for index/status/doctor/agent/agent-capsule/compact -- not value-level |
| `crates/ast-sgrep-cli/tests/machine_contracts.rs` (agent formats, chain/eval/bench envelopes, format aliases) | hand assert_eq expected fields / shapes | structural + sparse values | Y | N | 3 | structural + selective exact | Many tests assert shapes + bounds (limit, preview length); not full hit dumps |
| `crates/ast-sgrep-cli/tests/cli_smoke.rs` | hand assert structural | structural | Y | N | 3 | structural | Smoke on agent-capsule/compact/github shapes; compact single-line stdout |
| `crates/ast-sgrep-cli/tests/no_embed_hit_key_parity.rs` | other (cross-surface parity, not frozen file) | semantic / structural | Y | N | 3 | structural | CLI vs core vs LSP HitKey sets sorted equal -- oracle is peer surface, not golden file |
| `crates/ast-sgrep-cli/tests/watch_incremental.rs` | hand assert_eq counts | exact counts | Y | N | 2 | exact | Index update stats only |
| `crates/ast-sgrep-cli/tests/agent_surface/R-001__broken_pipe_json.sh` | other (absence oracle) | N/A | Y | Partial | 2 | structural | Must not panic on SIGPIPE; not content golden |
| `crates/ast-sgrep-cli/tests/agent_surface/R-002__format_typo_teaches.sh` | hand substring expected | fuzzy (grep) | Y | N | 3 | scrubbed exact (candidate) | Human did-you-mean strings for format typo |
| `crates/ast-sgrep-cli/tests/agent_surface/R-003__missing_query_teaches.sh` | hand substring expected | fuzzy (grep) | Y | N | 3 | scrubbed exact (candidate) | Human teaching lines + usage envelope fragments |
| `crates/ast-sgrep-lang/tests/extraction_goldens.rs` + `fixtures/extract/*` | hand assert expected tuples (named golden) | structural / presence | Y | N | 3 | structural → true golden dump optional | 13 langs; fixtures are **inputs**; expectations are symbol/import/call/pattern/forbid tuples via `assert_language_conformance` -- not full ExtractionResult dumps |
| `crates/ast-sgrep-lang/tests/pattern.rs` | hand assert | structural | Y | N | 2 | structural | Pattern routing / case sensitivity / metavariable native vs fallback |
| `crates/ast-sgrep-testkit/src/lang.rs` | other (assert harness) | structural | Y | N | 1 | N/A | Shared conformance engine; not a frozen artifact |
| `tests/fixtures/ranking/cases.json` + `crates/ast-sgrep-core/tests/ranking_oracle.rs` | JSON fixture compare (oracle constraints) | structural / rank bounds | Partial | N | 4 | structural (keep); scrubbed full ranking optional | `must_include` with `max_rank`; semantic case needs embed -- order beyond max_rank not frozen |
| `tests/fixtures/sample/**` | other (shared corpus input) | N/A | Y | N | 2 | N/A | Multi-lang sample used by CLI/core/LSP/ranking; not expected-output |
| `crates/ast-sgrep-core/tests/graph_oracle.rs` | hand assert expected sets | structural | Y | N | 2 | structural | Known symbols + case-fold queries; non-empty parity |
| `crates/ast-sgrep-core/tests/determinism_loop.rs` | other (self-golden over 50 runs) | exact self-compare | Y | N | 1 | exact (keep) | First JSON is baseline; not committed golden file |
| `crates/ast-sgrep-core/tests/e2e_smoke.rs` | hand assert / identity oracles | structural | Y | N | 3 | structural | Graph store + search smoke; comments note old weak embed oracle tightened |
| `crates/ast-sgrep-core/tests/parity.rs` | hand assert | structural | Y | N | 3 | structural | Search option wiring + IVF preserve |
| `crates/ast-sgrep-core/tests/downstream_correctness.rs` | hand assert | structural | Y | N | 3 | structural | Chain truncation, search correctness beads |
| `crates/ast-sgrep-core/tests/search_correctness_epics.rs` | hand assert_eq scores/fields | exact + structural | Y | N | 3 | exact/structural | RRF scores, filters, fail-closed |
| `crates/ast-sgrep-core/tests/signal_provenance.rs` | hand assert_eq | exact | Y | N | 2 | exact | Fusion signal tiers + margins on synthetic hits |
| `crates/ast-sgrep-core/tests/pattern_prefilter.rs` | hand assert_eq profile counts | exact | Y | N | 3 | exact | files_considered/prefiltered/parsed/hits |
| `crates/ast-sgrep-core/tests/metamorphic.rs` | other (metamorphic) | N/A | Y | N | 2 | N/A | Relations between queries, not frozen dumps |
| `crates/ast-sgrep-core/tests/semantic_ivf_roundtrip.rs` | hand assert / fuzzy recall | exact + fuzzy | Partial | N | 4 | fuzzy for recall; exact for vectors | Vector roundtrip + ANN recall thresholds |
| `crates/ast-sgrep-core/src/bench_suite.rs` (unit: identity oracle required) | other (meta-test) | structural | Y | N | 2 | structural | Every bench case must declare identity oracle -- not output golden files |
| `crates/ast-sgrep-plugins/tests/capsule_format.rs` | hand assert_eq expected JSON | exact (synthetic) | Y | N | 3 | exact → true golden file optional | Agent capsule, compact budgets, github/gitlab page fields on fixed `SearchResponse` |
| `crates/ast-sgrep-mcp/tests/protocol.rs` | hand assert_eq lists/fields | structural + exact names | Y | N | 3 | structural / true golden for tools list | Full tool name vector frozen inline; tool call results checked for shape/kind not full dump |
| `crates/ast-sgrep-lsp/tests/lsp.rs` | hand assert structural | structural | Y | N | 3 | structural | Search hits fields, edit/UTF-16, readiness; no snapshot files |
| `crates/ast-sgrep-codemode/tests/catalog.rs` | hand assert names/fields | structural | Y | N | 3 | structural / true golden for schemas | Tool catalog presence + adapter host shapes |
| `crates/ast-sgrep-codemode/tests/session_plan.rs` | hand assert | structural | Y | N | 3 | structural | Capsule defaults, plan steps |
| `crates/ast-sgrep-codemode/tests/batch.rs` | hand assert | structural | Y | N | 3 | structural | Serial/parallel batch + serve NDJSON |
| `crates/ast-sgrep-cli/src/eval.rs` + ephemeral gold in machine_contracts | other (eval harness + inline gold) | structural / metric | Partial | N | 4 | fuzzy metrics + gold fixture file | **No checked-in multi-query gold** for published baselines; test writes tiny temp gold |
| `benchmarks/results/baselines.md` | other (published historical freeze) | fuzzy / N/A for CI | N (historical) | Y | 5 | DO NOT treat as CI golden; honesty ledger only | UNREPRODUCIBLE MRR/Recall/nDCG; Agents.md provenance rules |
| `benchmarks/results/{bakeoff,head-to-head,losses,speed}.md` | other (published records) | N/A | N | Y | 5 | ledger only | Same honesty class as baselines |
| `docs/validation/cargo-geiger-baseline.txt` | other (policy baseline doc) | N/A | Y | N | 2 | N/A | Not an automated golden test |
| `packages/pi/release-contract.json` + `packages/pi/scripts/check-contract.mjs` | JSON fixture / contract freeze | exact structural + formatting | Y | N | 3 | exact / scrubbed | Deterministic 2-space JSON + trailing newline; versions coupled to Cargo/npm |
| `packages/pi/scripts/release-acceptance.mjs` | hand assert inventories | exact | Y | Partial | 3 | exact | Artifact file inventories, publish order |
| `packages/pi/scripts/ci-install-smoke.mjs` | structural predicates | structural | Y | N | 3 | structural | Runtime fixture index/search invariants |
| `packages/pi/extension/test/*.ts` (tools, code-mode, commands, runtime, security, …) | hand deepEqual expected args/shapes | exact (mocked) | Y | N | 2 | exact | CLI argv matrices frozen as arrays; not file goldens |
| `packages/pi/launcher/test/*.mjs` | hand assert | structural/exact | Y | N | 2 | structural | Package security, mode matrix |
| `crates/ast-sgrep-testkit/src/test_log.rs` (`IndexSnapshot`) | false friend name | N/A | Y | N | 1 | N/A | Logging snapshot helper, not approval testing |
| `packages/pi/extension/test/runtime.test.ts` (`structuredClone` snapshot) | false friend | N/A | Y | N | 1 | N/A | Mutability guard, not insta snapshot |

**Inventory row count:** 42 classified rows (including false-friend and ledger entries). **True file-backed goldens:** 3 JSON files under CLI fixtures + ranking cases + Pi release contract. **Named "golden" test modules:** 1 (`extraction_goldens.rs`). **insta / `*.snap` / `UPDATE_GOLDENS`:** 0.

---

## 3. Coverage map by surface

| Surface | Has golden-like coverage? | Quality |
|---------|---------------------------|---------|
| **CLI machine contracts** | **Yes -- strongest** | Full equality goldens for capabilities + failure/version envelopes (scrubbed); structural key-shape freeze for index/status/doctor/agent formats; rich exit-code/usage tests. Gap: successful search **hit payloads** not frozen end-to-end. |
| **CLI human output** | Partial / weak | Agent-surface shell greps for did-you-mean and teaching footers; robot-docs only asserts body contains `"agent handbook"`. No full `--help` / handbook / human error golden files. |
| **Lang extraction** | Partial (named goldens, presence-only) | 13-language fixtures + shared conformance tuples. Excellent regression net for "must emit X / forbid Y"; weak for ordering, full symbol lists, span exactness beyond bounds, AST node dumps. |
| **Core search / index** | Partial (oracles, not dumps) | Ranking oracle JSON, graph oracle, determinism loop, metamorphic suite, correctness epics. Strong behavioral gates; almost no frozen full `SearchResponse` JSON for sample corpus. |
| **Embed** | Weak for goldens | Unit dim/backend asserts; IVF roundtrip + fuzzy recall; no embedding vector goldens (correct -- vectors non-deterministic across models). |
| **LSP** | Structural only | Smoke + edit/UTF-16 + readiness; no frozen LSP JSON-RPC transcripts. |
| **MCP** | Partial | Tool name list exact; initialize protocol version exact; tool results shape/kind. Full tool schema / large result payloads not frozen. |
| **Codemode** | Partial | Catalog names, adapter shapes, capsule defaults, batch modes. Tool input schemas not full golden files. |
| **Benchmarks / baselines** | Ledger only (not tests) | `baselines.md` freezes published numbers as UNREPRODUCIBLE historical record. No CI golden that regenerates or asserts them. Correct honesty posture; does not protect runtime regressions. |
| **Pi / npm release** | Strong structural contract | `release-contract.json` + check scripts freeze packaging surface; install smoke is predicate-based. |
| **Plugins (output formats)** | Strong hand asserts | Capsule/compact/github/gitlab on synthetic response; best candidate to convert to file goldens for review diffs. |

---

## 4. False friends

Things named golden/snapshot that are **not** golden-artifact tests:

| Name / path | What it actually is |
|-------------|---------------------|
| `GoldenWidget`, `GoldenState`, `GoldenRender`, `GoldenPoint`, `GoldenRecord`, `GoldenRenderable`, `GoldenWorker`, `GoldenAlias` in `crates/ast-sgrep-lang/tests/fixtures/extract/*` | Fixture **symbol identifiers** for extraction tests |
| Same names in `extraction_goldens.rs` expectation tables | Expected extracted names, not golden files |
| `IndexSnapshot` in `crates/ast-sgrep-testkit/src/test_log.rs` | Test logger event payload |
| `structuredClone(...); snapshot` in `packages/pi/extension/test/runtime.test.ts` | Immutability check |
| `.zerostack/**/logical-snapshot.sqlite3` | ZeroStack forensic DB copies |
| `docs/validation/cargo-geiger-baseline.txt` | Manual unsafe audit notes |
| `capabilities` field text `"deterministic": "stable JSON key ordering..."` | Contract documentation string, not a golden suite |
| `update_paths` / `update_bench_history` | Mutating APIs, not golden update modes |
| `registry-snapshot` option in release-acceptance | Offline registry JSON path for release checks |
| Benchmark "gold" queries (18 gold / 14 gold in baselines.md) | Historical eval labels **absent from tree** |

---

## 5. Missing infrastructure checklist (skill) -- status only

| Checklist item | Status |
|----------------|--------|
| `assert_golden` infrastructure with `UPDATE_GOLDENS` support | **N** |
| Scrubber handles dynamic values (UUIDs, timestamps, durations, paths) | **Partial** -- ad-hoc field scrub only (`version` / `message` / `command` in machine_contracts) |
| Every golden file reviewed by human before first commit | **Partial** -- CLI fixtures and ranking cases are deliberate; no suite-wide process |
| PROVENANCE.md records how goldens were generated | **N** (baselines.md has provenance for metrics only) |
| `.gitignore` includes `*.actual` files | **N** |
| CI fails on golden mismatch (no auto-update in CI) | **Partial** -- normal `cargo test` / contract scripts fail on mismatch; no dedicated golden job |
| Diff output in failure messages (not just "mismatch") | **Partial** -- standard `assert_eq!` Debug diffs; no unified golden diff helper |
| Golden files organized by feature/module | **Partial** -- CLI fixtures only; no `tests/golden/` tree |
| Cross-platform canonicalization if needed | **Partial** -- `NO_COLOR=1`, path ends_with checks; no systematic canonicalize helper |
| insta / cargo-insta workflow | **N** |
| Binary / semantic golden helpers | **N** |

---

## 6. Top 10 highest-value golden candidates

Ranked by **(output complexity × regression risk × currently weak assertions)**.

| Rank | Candidate | Why high value | Suggested pattern |
|------|-----------|----------------|-------------------|
| 1 | **Sample-corpus search responses** (native/agent/agent-capsule/compact) for a fixed query set | Complex multi-field JSON; ranking/format bugs slip past key-shape + non-empty checks | Scrubbed exact golden files per format (scrub paths, scores if needed, keep order) |
| 2 | **Full language extraction dumps** (or sorted symbol/import/call lists) per `fixtures/extract/*` | Presence tuples miss extras, kind drift, ordering; extraction is core correctness | Scrubbed/canonicalized JSON golden per language (sort keys; normalize spans if volatile) |
| 3 | **`robot-docs` / agent handbook body** | Long agent-facing markdown; only substring `"agent handbook"` today | Exact or scrubbed markdown golden |
| 4 | **Plugins formatters** (`capsule_format.rs` synthetic cases → files) | Already dense asserts; hard to review as Rust literals | Exact golden files + `UPDATE_GOLDENS` |
| 5 | **MCP `tools/list` full tool descriptors** (schemas, not just names) | Schema drift breaks hosts; names alone miss property renames | Scrubbed JSON golden |
| 6 | **Codemode tool catalog + host adapters** (Anthropic/OpenAI/CF) | Progressive discovery contracts; currently name/shape samples | Structural or scrubbed full catalogs |
| 7 | **CLI human teaching diagnostics** (R-002/R-003, usage tips) | Agent UX regressions; greps are brittle and incomplete | Scrubbed exact stderr/stdout goldens |
| 8 | **Checked-in eval gold for `tests/fixtures/sample`** | Enables reproducible ranking metrics; closes baselines honesty gap without inventing numbers | Gold fixture (eval schema) + optional fuzzy MRR gates separate from historical ledger |
| 9 | **LSP search executeCommand / hover-like payloads** on sample_backend | Multi-field hit JSON across editor surface | Scrubbed JSON golden transcripts |
| 10 | **Graph chain expand JSON** for fixed oracle fixture | Chain truncation/edge bugs; mostly non-empty asserts | Exact/structural golden of nodes+edges (sorted) |

**Not recommended as exact goldens:** embedding vectors, ANN recall numbers, wall-clock latency, historical UNREPRODUCIBLE baseline rows as CI pass/fail.

---

## Confidence matrix (summary)

| Artifact class | Det? | Plat? | Vol | Current strategy | Fit |
|----------------|------|-------|-----|------------------|-----|
| CLI capabilities JSON | Y | N | 3 | scrubbed exact | Excellent -- keep |
| CLI error envelopes | Y | N | 2 | scrubbed exact | Excellent -- keep |
| CLI shape keys | Y | N | 3 | structural | Good -- consider value goldens for hits |
| Extraction conformance | Y | N | 3 | structural presence | Good -- dumps would raise bar |
| Ranking cases | Partial | N | 4 | structural max_rank | Good for identity; weak for full order |
| Determinism loop | Y | N | 1 | self exact | Excellent for flakiness |
| Plugins formatters | Y | N | 3 | hand exact | Good -- migrate to files for review |
| Benchmarks baselines | N | Y | 5 | ledger / UNREPRODUCIBLE | Correct non-CI treatment |
| Handbook / human CLI | Y | N | 3 | fuzzy grep | Weak -- promote to golden |

---

## Method notes

- No `insta` dependency in workspace Cargo manifests.
- No files matching `*.snap` or `*.golden` in product/test trees (excluding agent skill caches under `.pi-subagents/`).
- Primary true golden equality path: `capabilities_and_version_match_goldens` and envelope scrub compares in `machine_contracts.rs`.
- `extraction_goldens` naming is intentional but means **conformance cases**, not golden files.
- Published quality numbers live only in `benchmarks/results/*` and are explicitly non-reproducible from this tree.

---

*End of Pass 1. Passes 2+ may add infrastructure, beads, and goldens -- out of scope here.*

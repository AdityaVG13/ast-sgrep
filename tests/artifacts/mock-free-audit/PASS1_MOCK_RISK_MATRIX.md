# Pass 1 -- Mock Risk Matrix (inventory only)

**Scope:** Score I/O-backed test paths. No fixes, no beads, no refactors.
**Doctrine:** If a mock hides a bug that would break production, the mock is worse than no test.
**Score** = Production Impact (1-5) x Mock Divergence Risk (1-5). Score >= 8 MUST be mock-free; >= 4 SHOULD be.
**Date:** 2026-08-07
**Corpus scanned:** `crates/**/tests/**`, `crates/ast-sgrep-testkit/**`, I/O unit tests under `crates/*/src`, embed stubs, soft-skips, `#[ignore]`.

---

## 1. Inventory summary

| Area | Path count (approx) | Dominant style |
|------|---------------------|----------------|
| `ast-sgrep-core/tests/*.rs` | 31 files | Real SQLite `IndexStore` via `tempfile` + real `Indexer`/`Searcher` |
| `ast-sgrep-cli/tests/*.rs` | 4 files | Real CLI process (`CARGO_BIN_EXE_asgrep`) except `watch_incremental` (library API) |
| `ast-sgrep-mcp/tests/protocol.rs` | 1 file / ~9 tests | Real `asgrep-mcp` subprocess, stdin JSON-RPC |
| `ast-sgrep-codemode/tests/*.rs` | 3 files | Real index + in-process session (no embed) |
| `ast-sgrep-lsp/tests/lsp.rs` | 1 file | Real `IndexStore` via testkit `sample_backend` |
| `ast-sgrep-testkit` | helpers only | Real index/CLI/LSP surfaces; not a mock layer |
| `ast-sgrep-embed` unit | stubs in `embedder.rs` | `stub_ollama` / `stub_cloud` for dim-probe only |
| `ast-sgrep-lang/plugins` | parse/capsule | Pure / fixture -- out of critical I/O surface |

**No** `mockall` / `mockito` / `wiremock` / fake in-memory IndexStore found.
**Critical production surfaces (focus):** SQLite IndexStore, index/search pipeline, embed backends, CLI process, MCP JSON-RPC, durability/WAL, semantic ANN.

### Stub / soft-skip / ignore map

| Kind | Location | Notes |
|------|----------|--------|
| `stub_ollama` / `stub_cloud` | `crates/ast-sgrep-embed/src/embedder.rs` (`dim_probe_tests`) | Injected via `with_embed_fn`; fixed constant vectors; never hits HTTP |
| Soft-skip embed oracle | `downstream_correctness.rs` `bead_vwga_ranking_cases_json_self_oracle` | Forces `use_embed: false`; skips `kind=="embed"` must_include with eprintln |
| Soft-return budget | `sub1ms.rs` | Debug build returns early (no gate); release asserts |
| `#[ignore]` | `e2e_smoke.rs` `archived_pi_fixture_graph_modes_match_indexed_keys` | Needs `ASGREP_REAL_PI_FIXTURE` large archive |
| `#[ignore]` | `semantic_ivf_roundtrip.rs` `adaptive_ivf_tradeoff_at_2048_and_10000_vectors` | Release-mode ANN recall/latency |
| `#[ignore]` | `store_delete.rs` `re_upsert_many_files_is_linear` | Timing quarantine only |
| Fixture-only / pure | `ranking_oracle` synthetic hit ranking in `search/mod.rs` unit tests | In-memory `SearchHit` lists -- not DB I/O |
| Production semantic path | `HashedEmbedder` | **Not a mock** -- default offline embed backend in prod |

---

## 2. Mock Risk Matrix (top paths)

Scoring: **I** = Production Impact, **R** = Mock Divergence Risk, **S** = I x R.
Style: how the path is tested today. Gap: what still fails the mock-free doctrine.

| # | Path / surface | I | R | S | Current style | Gap |
|---|----------------|---|---|---|---------------|-----|
| 1 | **Ollama embed HTTP** (`embed_via_ollama` / `OllamaEmbedder`) | 5 | 5 | **25** | unit stub only (`stub_ollama` constant 384-d vec) | No live or loopback Ollama; JSON/error/dim/probe never exercised against real API shape |
| 2 | **Cloud embed HTTP** (`embed_via_api` / `CloudEmbedder`) | 5 | 5 | **25** | unit stub only (`stub_cloud` constant 1536-d); URL allowlist pure unit | No recorded/live cloud e2e; SSRF allowlist unit-only; no key/env failure contract via process |
| 3 | **Neural embed (fastembed)** opt-in path | 4 | 5 | **20** | flag wiring + fail-closed when feature off (`e2e_smoke` / `parity`); capabilities list flag names | Zero real ONNX/model load or index+search with `EmbedBackend::Neural` in CI |
| 4 | **CLI default / agent search with embed ON** | 5 | 4 | **20** | real CLI process but almost always `--no-embed` (`machine_contracts`, `cli_smoke`, `no_embed_hit_key_parity`) | Production default is embed-on; agent JSON shapes never assert embed hits or embed meta through CLI |
| 5 | **Large real-corpus graph e2e** (`archived_pi_fixture_...`) | 5 | 4 | **20** | real Indexer/IndexStore when run; **`#[ignore]`** by default | Never in default CI; scale/case-fold/import graph bugs only on manual archive |
| 6 | **Ranking embed oracle (vwga soft-skip)** | 4 | 4 | **16** | real sample index; **`use_embed: false`** + soft-skip embed must_include | Diverges from `ranking_oracle.rs` (hard assert, `use_embed: true`); synonym case can pass CI while embed channel is dead |
| 7 | **MCP JSON-RPC process + semantic tool** | 5 | 2 | **10** | real subprocess + real index (`embed_semantic: true`); asserts `semantic_search` -> `kind=embed` | Good mock-free for hashed semantic; no cloud/ollama/neural through MCP; unit cache test uses `use_embed: false` only |
| 8 | **ANN IVF quality at scale** (`adaptive_ivf_tradeoff_...`) | 4 | 3 | **12** | synthetic flat vectors (real ANN code); **`#[ignore]`** | Scale recall/latency not gated in CI; small-N IVF tests do run |
| 9 | **SQLite durability / WAL / integrity** (`durability_epics`, `store_pragmas`) | 5 | 1 | **5** | real on-disk SQLite, corrupt open, pragmas, tx rollback | Already mock-free; keep as gold standard |
| 10 | **IndexStore delete / graph clear** (`store_delete`, durability remove_file) | 5 | 1 | **5** | real DB rows + IVF sidecar | Mock-free; ignored test is timing-only (score 6 for timing gate alone) |
| 11 | **Index + hybrid search pipeline** (`e2e_smoke`, `parity`, `search_correctness_epics`, `graph_oracle`) | 5 | 1 | **5** | real TempDir corpus + Indexer + Searcher | Mock-free for hashed semantic / lexical / graph |
| 12 | **Semantic ANN locality + IVF roundtrip (non-ignored)** | 4 | 1 | **4** | real `SemanticAnnIndex` + disk IVF save/load | Mock-free for algorithm; scale ignored separately (#8) |
| 13 | **CLI machine contracts / smoke** (no-embed) | 4 | 2 | **8** | real process + real index | Mock-free for JSON envelopes; gap is embed-on (#4) not process fakeness |
| 14 | **Surface hit-key parity CLI/core/LSP** | 4 | 2 | **8** | real all three surfaces; forced `--no-embed` / `no_embed` | Correct for identity parity; does not cover embed-kind key parity across surfaces |
| 15 | **Codemode batch/session** | 4 | 2 | **8** | real SQLite index; `embed_semantic: false`, `use_embed: false` | Real process-less API; embed path and true multi-process serve under load untested |
| 16 | **Watch / incremental update** (`watch_incremental`) | 4 | 3 | **12** | real `Indexer::update_paths` + SQLite; **not** CLI `watch` or fs event loop | Library path mock-free; missing real CLI watch daemon / notify e2e |
| 17 | **LSP search + reindex** (`lsp.rs` + testkit) | 4 | 2 | **8** | real IndexStore backend | In-process backend (not language-server stdio JSON-RPC end-to-end) |
| 18 | **Response / semantic cache identity** | 4 | 1 | **4** | real store meta + Searcher | Mock-free |
| 19 | **External ast-grep fallback** | 3 | 4 | **12** | production fail-closed without allow; unit asserts disabled | No opt-in e2e with real `ast-grep` binary when allowed (bench-only path) |
| 20 | **sub1ms pipeline budget** | 3 | 2 | **6** | real warm sample; soft-return in debug | Release-only hard gate; not a mock issue |

---

## 3. Score >= 8 gaps (MUST mock-free later)

These are the highest-priority bead candidates for pass 2 (inventory only here).

1. **S=25 -- Ollama live/contract path**  
   Today: `stub_ollama` only. Need: loopback fake HTTP server with real `ureq` client *or* opt-in live Ollama gated by env (hard fail when requested, not soft-skip). Cover: request body model field, non-200, empty embedding, dim probe after first call, `ASGREP_NO_OLLAMA`.

2. **S=25 -- Cloud embed live/contract path**  
   Today: `stub_cloud` + pure URL allowlist. Need: same pattern for OpenAI-shaped JSON; auth failure; blocked SSRF URL at client call site (not only allowlist unit); dim probe.

3. **S=20 -- Neural embed feature e2e**  
   Today: parse/flag + fail-closed without feature. Need: `cargo test --features neural-embed` path that loads model once, indexes sample, asserts embed hits / backend meta `neural` (or documented skip only if model download forbidden -- prefer offline fixture model pin).

4. **S=20 -- CLI agent/search with embed default ON**  
   Today: contracts force `--no-embed`. Need: at least one machine-contract (or smoke) that indexes with semantic embed, searches without `--no-embed`, asserts embed hit presence or backend field in JSON envelope.

5. **S=20 -- Archived Pi / large-corpus graph e2e**  
   Today: `#[ignore]` + env archive. Need: either CI artifact + scheduled non-ignore job, or smaller committed multi-k-file fixture that still exercises graph scale classes; keep ignore only for full archive.

6. **S=16 -- Unify ranking oracle embed policy**  
   `ranking_oracle.rs` hard-asserts with `use_embed: true`; `bead_vwga_*` disables embed and soft-skips. Need: single CI-hard path for `synonym_credential_renewal` / embed must_include (hashed semantic is enough); remove soft-skip or hard-fail when embed hits empty after semantic index.

7. **S=12 -- ANN scale quality gate**  
   `adaptive_ivf_tradeoff_at_2048_and_10000_vectors` ignored. Need: release CI job or smaller SLO that still fails on quality regressions.

8. **S=12 -- CLI watch daemon e2e**  
   `watch_incremental` is library-only. Need: real `asgrep watch` (or documented supervisor path) process with file edit -> reindex observation.

9. **S=12 -- External ast-grep opt-in e2e**  
   Fail-closed is good; when `external_ast_grep_allowed`, no test proves spawn/parse path.

10. **S=10/8 residual**  
    MCP is strong for hashed semantic; add cloud/neural tool args only after #1-3. CLI surface parity with embed kinds (file/line/kind) when embed on.

---

## 4. Score >= 4 gaps (SHOULD)

| Gap | S | Note |
|-----|---|------|
| Codemode embed-on session | 8 | Index/search with `use_embed: true` + semantic meta |
| LSP stdio protocol e2e | 8 | Today in-process `LspBackend`, not full LSP transport |
| Embed-kind multi-surface hit keys | 8 | Extend parity beyond `--no-embed` |
| `sub1ms` debug soft-return | 6 | Document-only OK; optional release-only CI job |
| `store_delete` timing ignore | 6 | Bench territory; low correctness risk |
| Cloud feature gate is compile-only | 4-6 | `cloud_feature_gate.rs` checks cfg, not HTTP |

---

## 5. Already correct (mock-free gold -- keep)

Named evidence paths that already use real DB / real process and should not be "mocked down":

1. **`crates/ast-sgrep-core/tests/durability_epics.rs`** -- real SQLite: WAL/sync restore, corrupt open quarantine, transactional `clear_all_data`, IVF atomic rename, response cache generation invalidation.
2. **`crates/ast-sgrep-core/tests/store_pragmas.rs`** -- real `IndexStore::open`, asserts journal/pragma behavior on disk.
3. **`crates/ast-sgrep-cli/tests/cli_smoke.rs`** + **`machine_contracts.rs`** -- real `Command` to `asgrep` binary; index/status/doctor/agent JSON envelopes (with `--no-embed`).
4. **`crates/ast-sgrep-mcp/tests/protocol.rs`** -- real `asgrep-mcp` subprocess JSON-RPC; hierarchical tools including `semantic_search` with real embed hits (hashed).
5. **`crates/ast-sgrep-core/tests/e2e_smoke.rs`** `parity_index_defs_hybrid_chain` -- real sample index, defs/callers/semantic synonym, chain expand, reopen identity.
6. **`crates/ast-sgrep-core/tests/graph_oracle.rs`** -- real index fixture; defs/callers/imports/chain retrieval parity.
7. **`crates/ast-sgrep-core/tests/ranking_oracle.rs`** -- real index + hard must_include (including embed synonym case with `use_embed: true`).
8. **`crates/ast-sgrep-testkit`** (`index_sample`, `CliSession`) -- builds real SQLite and real CLI processes; not a fake store.

Hashed/semantic embed used throughout CI is **production offline backend**, not a mock of Ollama/cloud/neural. Divergence risk applies when tests claim coverage of those backends while only exercising hashed or stubs.

---

## 6. Method notes

- Inventory via filesystem listing of `crates/*/tests/**` and ripgrep for `stub_`, `#[ignore]`, soft-skip, `TempDir`, `IndexStore::open`, `Command::new`, embed backends.
- Did not run `cargo test` (inventory-only pass).
- `HashedEmbedder` scored as production path; stub injectors scored as mock divergence for remote backends only.
- Soft-skip scored high when it converts a correctness oracle into a no-op under green CI.

---

## 7. Pass-2 handoff (do not execute in this pass)

- Create beads only for Score >= 8 (and selected >= 4) gaps above.
- Prefer env-gated real services or loopback HTTP with the real client over expanding `with_embed_fn` stubs.
- Do not replace real-DB tests with in-memory fakes.
- When adding live Ollama/cloud: fail hard when the gate env is set; never soft-skip as success.

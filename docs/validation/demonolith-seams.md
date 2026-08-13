# Demonolith Phase 2 seams — ast-sgrep-wt-demonolith

Run: `2026-08-13-ast-sgrep-wt-demonolith-1` · Phase 2 static sweep only (no extractions, no Phase 3 baselines).

Workspace artifacts: `../ast-sgrep-wt-demonolith__demonolith_workspace/phase2_findings_*.md` + `graphs/`.
Project HEAD at analysis: `2142be7`.

| ID | File | Clusters | Severity | Evidence (one line) |
|---|---|---|---|---|
| F-001 | `crates/ast-sgrep-core/src/store/sqlite.rs` | concern cut inside C1 (query/read ~1209–1731); C2 DTOs shared leaf | must-split | 92-method `IndexStore` hub; graph god-symbol `impl@:126`; B3 query surface separable from upsert/tx |
| F-002 | `crates/ast-sgrep-core/src/store/sqlite.rs` | concern cut inside C1 (upsert/insert_* ~801–1199); C2 with writers | must-split | Contiguous write pipeline; co-change with `index.rs` (21) on upsert path |
| F-003 | `crates/ast-sgrep-core/src/store/sqlite.rs` | concern cut inside C1 (semantic chunk/embed IO) | should-split | Semantic IO mixed into IndexStore; co-change with `semantic_ann.rs` (17) |
| F-004 | `crates/ast-sgrep-core/src/store/sqlite.rs` | C1-internal `FORCE_*:26–32` | leave-alone-with-rationale | Test injects; graph C1↔C2 shared-state = 0; travel with tx helpers |
| F-001 | `crates/ast-sgrep-core/src/index.rs` | C5+C6 → extract; C2 stays | must-split | Recovery/quarantine/sidecar FS cluster; C6 cohesion 0.81 |
| F-002 | `crates/ast-sgrep-core/src/index.rs` | C3+C4 → extract; C2 stays | must-split | prepare/hash/extract helpers leaf-ward of Indexer hub |
| F-003 | `crates/ast-sgrep-core/src/index.rs` | C1↔C2 via `FORCE_SIDECAR_REBUILD_ERR:22` | borderline + ⚠ ESCALATE | Graph SEAM-KILLER; set in C1, read at rebuild `:751` (C2) |
| F-004 | `crates/ast-sgrep-core/src/index.rs` | C7 → extract | should-split | `normalize_watch_path` / `canonicalize_affected_path` leaf |
| F-001 | `crates/ast-sgrep-core/src/search/mod.rs` | C3 → extract; C2 Searcher stays | should-split | finish_response/ranking/gates free-fn cluster (intra 22) |
| F-002 | `crates/ast-sgrep-core/src/search/mod.rs` | C6 → extract | should-split | Ledger append trio (`:998–1043`) low inter-edges |
| F-003 | `crates/ast-sgrep-core/src/search/mod.rs` | C5 → extract | borderline | `maybe_rerank` / `apply_rerank_order` small optional path |
| F-004 | `crates/ast-sgrep-core/src/search/mod.rs` | C4 owns nested `META_CACHE:952` | borderline + ⚠ ESCALATE | ast-grep OnceLock\<Mutex\<HashMap\>\> inside `estimate_prevented_reads` |
| F-005 | `crates/ast-sgrep-core/src/search/mod.rs` | C2 Searcher Mutex caches `:76–78` | leave-alone-with-rationale | Intentional B5 warm-path hub state; not a mechanical extract |
| F-001 | `tests/cli/machine_contracts.rs` | family partition across C1/C2/C3 | borderline | Multi-family CLI contracts; high inter-edges (helpers weld) |
| F-002 | `tests/cli/machine_contracts.rs` | helpers in C1/C3 → support mod | should-split | Shared harness prerequisite before family file split |
| F-001 | `tests/core/metamorphic.rs` | C5 → extract | should-split | `mr_pred_*` leaf; intra/inter 44/6 (cohesion 0.88) |
| F-002 | `tests/core/metamorphic.rs` | C3+C4+C7 → extract | borderline | Edge-orphan ANN MR tests weakly coupled to search MRs |
| F-003 | `tests/core/metamorphic.rs` | whole-file catalog | leave-alone-with-rationale | Strong `//!` inventory + 1-commit churn; tripwire if >1500 LOC |

## Notes
- Coverage Appendix B deferred to Phase 3 / pass 3 (no instrumented suite this pass).
- `sqlite.rs` communities under-resolved (impl hub); F-001–F-003 use method-span evidence inside graph C1.
- No invented benchmark numbers; churn from `churn_coupling.json` only.
- Watchlist only (not analyzed as monolith this pass): `crates/ast-sgrep-mcp/src/lib.rs` (972 LOC).

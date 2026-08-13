# Demonolith leave-alone + compile-resource — B11 / borderline / residual

**Run:** `2026-08-13-ast-sgrep-wt-demonolith-1`  
**Branch:** `refactor/de-monolithize-isomorphic`  
**Pass:** Leave-alone documentation only (no extractions, no crate source edits)  
**Confirmed extracts kept:** EXP-001..006  
**Sizes:** `wc -l` at documentation time (product soft threshold = 1000 code LOC / repo rule)

## Compile-resource

| Item | Status |
|---|---|
| Compile time + peak RSS (`compile-mem-profile.sh`) | **SKIPPED** — Phase 3 incomplete; no invented numbers |
| Criterion benches | **SKIPPED** — Phase 3 incomplete; no invented numbers |
| Isomorphism GATE 4/5 (perf / compile-resource) | **SKIPPED** — same Phase 3 gap; EXP-001..006 used `--quick` class proof (suite + public-api only) |

Re-measure on a quiet SAME-MACHINE window before claiming compile-resource neutrality for any future `index.rs` extract.

## No new dyn dispatch

This pass adds none. Prior EXP-001..006 moved only pre-existing `&dyn ToSql` sites with their methods (sqlite); no new `Box<dyn` / `Arc<dyn>` / trait-object hubs.

---

## Per-file decisions

### `crates/ast-sgrep-core/src/store/sqlite/mod.rs` — 800 `wc -l` (under)

| Field | Value |
|---|---|
| Bucket | Post-split façade / schema+tx hub (was B1+B3; F-004 injects B5(test)) |
| Cohesion | Single IndexStore open/schema/meta/tx concern after EXP-001 (`queries.rs`) + EXP-006 (`writes.rs`). `FORCE_*` thread-locals stay with tx helpers (F-004 leave-alone). |
| Decision | **No-split / leave-alone** — under soft threshold; remaining body is the intentional persistence hub behind `store/mod.rs`. |
| Re-examine when | Hub regrows past ~1000 LOC, or a new concern lands beside open/tx without a confirmed seam. |

### `crates/ast-sgrep-core/src/store/sqlite/queries.rs` — 505 · `writes.rs` — 461

| Field | Value |
|---|---|
| Bucket | Extracted satellites (EXP-001 / EXP-006); not census monoliths |
| Cohesion | Read/query surface vs upsert/write pipeline already proven isomorphic. |
| Decision | **No-split** — under threshold; do not re-slice without a new SEAM_CONFIRMED probe. |

### `crates/ast-sgrep-core/src/search/mod.rs` — 846 (under)

| Field | Value |
|---|---|
| Bucket | B4 hub remnant + B5 warm-path Mutex caches (F-005); finish/ranking already EXP-003 |
| Cohesion | `Searcher` + `semantic_cache` / `lexicon_cache` / `response_cache` are one warm-path hub. Extracting caches alone is SEAM_REFUTED territory (F-005). Ledger / META_CACHE remain borderline satellites, not this pass. |
| Decision | **Leave-alone (B11-shaped hub state)** for F-005 caches; file under soft threshold after EXP-003. |
| Re-examine when | File crosses 1000 again, or a deliberate cache-owner redesign lands with its own isomorphism experiment. |

### LEAVE-ALONE: Searcher Mutex caches (F-005)

- Concern: Warm-path Searcher hub state (response/lexicon/semantic caches + poison-clear lock helpers).
- Cluster evidence: C2 owns Searcher + caches; instance Mutexes are not module-level cross-cluster statics.
- Docs: Module + Searcher docs describe reuse across calls.
- Navigation: Intentional B5; mechanical extract without Searcher redesign is out of scope.
- Re-examine when: Cache ownership redesign is explicitly scoped.

### `crates/ast-sgrep-core/src/index.rs` — 1347 (still over)

| Field | Value |
|---|---|
| Bucket | B1+B3 (Indexer pipeline + helpers); B5(test) `FORCE_SIDECAR_REBUILD_ERR` (F-003) |
| Cohesion | Recovery leaf already EXP-002 (`index_recovery.rs`). Remaining file still mixes Indexer hub with prepare/hash/extract (F-002) and watch-path helpers (F-004). |
| Decision | **Must-split residual — do not extract this pass** (fresh-eyes / next pass). Primary residual: **F-002 prepare/hash/extract helpers**. Secondary: F-004 watch-path leaf; F-003 inject ownership before any C1/C2 product cut. |
| This pass | Document only; no `⊕ EXTRACT`. |

### Residual still-split (next pass)

| Finding | Target | Severity | Note |
|---|---|---|---|
| **F-002** | prepare / hash / extract-row helpers (`PreparedFile`, `prepare_file`, `hash_content`, `rows_from_extraction`, …) | **must-split** | Leaf-ward of Indexer; language/`ParserRegistry` thread_local must move with `prepare_file` or stay reachable. |
| F-004 | `normalize_watch_path` / `canonicalize_affected_path` (+ adjacency) | should-split | Small leaf; preserve `pub` façade for `canonicalize_affected_path`. |
| F-003 | `FORCE_SIDECAR_REBUILD_ERR` ownership | borderline + escalate | Not a product split; clear SEAM-KILLER before splitting inject owner from rebuild path. |

### `crates/ast-sgrep-mcp/src/lib.rs` — 1005 `wc -l` / ~972 tokei code (borderline)

| Field | Value |
|---|---|
| Bucket | Borderline B4 hub (+ B5 `SearcherCache` Mutexes); sandbox leaf already EXP-004 |
| Cohesion | One MCP stdio concern: JSON-RPC dispatch, tool handlers, warm SearcherCache, byte-stability contract (`//!` header). Sandbox IO helpers extracted; remaining file is protocol+session hub. |
| Decision | **Borderline leave-alone** — soft code-LOC under 1000 (tokei); physical `wc` just over. Do not invent further cuts without a SEAM_CONFIRMED probe. No new dyn dispatch. |
| Re-examine when | Tokei code LOC ≥ 1000, or a second concern (non-protocol) lands in `lib.rs`. |

### `tests/core/metamorphic.rs` — 1382 (B9 catalog)

| Field | Value |
|---|---|
| Bucket | B9 test monolith; F-003 file-level leave-alone (justified catalog) |
| Cohesion | Single metamorphic-relations suite with strong `//!` inventory. EXP-005 moved `mr_pred_*` + `HitKey` to `metamorphic_preds.rs`; catalog `fn mr_*` stays together. Low historical churn. |
| Decision | **Leave-alone (F-003)** — keep MR catalog unified; further ANN-family splits (old F-002) stay deferred. |
| Re-examine when | File > ~1500 LOC, or ANN vs search reviewers conflict on the same suite. |

### LEAVE-ALONE: `tests/core/metamorphic.rs` (1382 LOC, bucket B9 / justified catalog)

- Concern: Metamorphic relations catalog (`fn mr_*`) for oracle-hard search/index/ANN surfaces.
- Cluster evidence: Graph shows extractable satellites; purpose remains one suite after predicate leaf extract.
- Docs: Extensive `//!` inventory of implemented MRs (current).
- Churn: Historically low (census 4 / 1); tripwire is size/review conflict, not churn.
- Navigation: Catalog + `#[path]` preds module is navigable; Score ≥ 2.0 gate documented in header.
- Re-examine when: exceeds ~1500 LOC, or ANN vs search families conflict in review.

### `tests/cli/machine_contracts.rs` — 1075 (B9 borderline)

| Field | Value |
|---|---|
| Bucket | B9 multi-family CLI machine-JSON contracts |
| Cohesion | Organizational “one contract suite”; graph shows soft communities with high helper inter-edges (F-001/F-002). Not Q≫0.3 product modularity. |
| Decision | **Borderline leave-alone** this pass — acceptable single suite if navigation stays fine. Helper extract (F-002) is prerequisite before any family file split; not executed here. |
| Re-examine when | File > ~1500 LOC, or families conflict in review / CI ownership. |

### `crates/ast-sgrep-core/src/semantic_ann.rs` — 682 (under)

| Field | Value |
|---|---|
| Bucket | Watchlist only (census prior high density×churn; under soft LOC) |
| Decision | **No-split / leave-alone** — under threshold (EXP-006 under-1k note). |

### `packages/pi/extension/src/runtime.ts` — 886 (under)

| Field | Value |
|---|---|
| Bucket | Watchlist only (partially decomposed into `src/codemode/*`) |
| Decision | **No-split / leave-alone** — under soft threshold. |

### Generated (unchanged)

`packages/pi/extension/dist/**` — B10; exclude from hand-splitting (census).

---

## Summary tables

### Leave-alone list

| File | Bucket / note | Decision |
|---|---|---|
| `store/sqlite/mod.rs` | Post-EXP façade/tx hub (+ F-004 injects) | leave-alone (under) |
| `store/sqlite/queries.rs` | EXP-001 satellite | leave-alone (under) |
| `store/sqlite/writes.rs` | EXP-006 satellite | leave-alone (under) |
| `search/mod.rs` | Hub under soft; **F-005** Mutex caches | leave-alone |
| `mcp/lib.rs` | Borderline MCP stdio hub post EXP-004 | borderline leave-alone |
| `tests/core/metamorphic.rs` | B9 catalog; **F-003** | leave-alone |
| `tests/cli/machine_contracts.rs` | B9 multi-family | borderline leave-alone |
| `semantic_ann.rs` | Under soft | leave-alone |
| `packages/pi/extension/src/runtime.ts` | Under soft | leave-alone |

### Residual still-split list

| File | Finding | Severity | This pass |
|---|---|---|---|
| `crates/ast-sgrep-core/src/index.rs` | **F-002** prepare/hash/extract | must-split | **deferred** (next / fresh-eyes) |
| `index.rs` | F-004 watch-path helpers | should-split | deferred |
| `index.rs` | F-003 FORCE_SIDECAR inject ownership | borderline + escalate | deferred (blocker hygiene, not product extract) |

### Compile-resource status

**SKIPPED** (Phase 3 incomplete; no invented RSS or bench numbers).

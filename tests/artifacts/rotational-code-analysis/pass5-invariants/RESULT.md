# Pass 5 RESULT — Contracts & invariants

| Field | Value |
|-------|-------|
| Loop | 5 / contracts-and-invariants |
| Status | **COMPLETE** |
| Mode | audit (no product edits under crates/ or packages/) |
| Freeze | `fb932aac852f5496c0a7035cc5a0b508e05111cb` (retained; HEAD may be ahead with books) |
| Axes | representation:requirements→properties · observer:user+caller · scale:system→function · evidence:docs+source+tests |
| Axes vs pass 4 | **4** (all changed) |
| Braid | **Continue** → pass 6 happy-path control flow |
| Prior state leveraged | true |

## Deliverables

| Artifact | Path |
|----------|------|
| Invariant ledger | `iterations/05-invariants/invariant-ledger.md` |
| Contract anchors | `iterations/05-invariants/contract-anchors.md` |
| Contradictions & gaps | `iterations/05-invariants/contradictions-and-gaps.md` |
| Machine ledger | `iterations/05-invariants/invariants.json` |
| Slim mirror | `tests/artifacts/rotational-code-analysis/pass5-invariants/` |

## Headline findings

1. **CONTRADICTION C1:** Cascade docs claim empty structural → no hybrid hits; code+tests implement lexical fallback + optional semantic (ht1h.3).
2. **CONTRADICTION C2:** MCP workspace jail vs Code Mode free `root` — no cross-surface isolation parity invariant holds.
3. **CONSISTENT safety cores:** embed allowlist + no-redirect, durability fail-closed, AST_GREP dual opt-in, MCP sandbox+searcher invalidation, batch no-mut-parallel, Pi edit root, limit clamps, cascade no file-widen.
4. **GAPs:** codemode searcher invalidation untested; `read_only` advisory only; XOR docs-only; `ASGREP_INDEX_PATH` absolute privilege unlabeled.

## Gate check

> Each critical scenario has at least one falsifiable invariant and a named evidence source or explicit inferred-necessity rationale.

**Met** — 18 invariants spanning authz, consistency, SSRF, durability, retrieval, resources. Conflicts recorded, not silently chosen.

## Evidence commands

```
rg -n 'sandbox_root|root_arg|try_index_db_path|invalidate_searcher|read_only|embed_url_is_allowed|Durability|working_files|validate_search_feature_flags' crates packages docs
# cascade: crates/ast-sgrep-core/src/search/mod.rs search_hybrid + tests/cascade_planner.rs
# docs: docs/cascade-query-planner.md vs code ht1h.3
# MCP tests: crates/ast-sgrep-mcp/tests/protocol.rs tool_roots_are_sandboxed_*
# index path: crates/ast-sgrep-core/src/store/mod.rs try_index_db_path
zs --json -C … fs '…'  # failed: fszero-codemode missing (B-ZS-ENGINES)
```

## Counts

- Invariant records: **18**
- CONSISTENT: **11**
- CONTRADICTION: **2**
- GAP: **5**
- Critical scenarios covered: **14**

## Residuals → pass 6 (happy path)

- Trace MCP `search` / `index_repo` happy path against INV-MCP-SANDBOX + INV-MCP-SEARCHER-INV
- Trace Code Mode `search`/`index_repo` with foreign `root` (INV-CM-ROOT-FREE) as intentional vs accidental
- Hybrid query with lexical-only survivors (C1 / INV-CASCADE-STRUCT-EMPTY) — observe live hits shape
- Pi sticky path for `index_repo` without host approval (INV-RO-CATALOG)
- Confirm ranking/fusion on sample corpus path (INV-RANK-FUSION) without inventing published metrics

## Braid residue

```
SPIN_THE_BLOCK_RESULT:
status: in_progress
mode: audit
target_root: /Users/aditya/Developer/ast-sgrep
prior_state_leveraged: true
iteration: 5
coverage_pending: foundation loops 6+
high_critical_without_loop27: n/a (audit observations; no R-* product findings filed)
braid_resolve: Continue
axes_changed: 4
void_fixture_outcome: n/a mid-wave
north_star_probe_outcome: n/a mid-wave
independent_loop27: pending
queue_action: none
books: .rotational-code-analysis/
```

# Demonolith EXP-003 — search finish_response / ranking / gates seam

**Verdict:** SEAM_CONFIRMED  
**Run:** 2026-08-13-ast-sgrep-wt-demonolith-1  
**Branch:** `refactor/de-monolithize-isomorphic`  
**Finding:** F-001 (search/mod.rs C3 finish/ranking/gates; finish-path C5 helpers included)

## What moved

| Step | Commit | Change |
|---|---|---|
| 1 | `070537d` | Finish/ranking/gate helpers → `crates/ast-sgrep-core/src/search/finish.rs`; `mod finish;` + façade re-exports in `search/mod.rs` |

**Moved into `finish.rs`:** `finish_response` (`pub`), `finish_response_checked` (`pub(crate)`), `identifier_tokens`, `definition_query_affinity`, `cmp_ranked_hits`, `same_definition_locus`, `rerank_candidate_limit`, `maybe_rerank`, `apply_rerank_order`, `enforce_result_gates`, `cap_per_file`, `contains_term_token`, `excerpt_term_coverage`, `MAX_HITS_PER_FILE`.

**Left in `mod.rs`:** `Searcher` hub + cache/lock helpers, `literal_prefilter_pass` / `structural_index_pass`, ledger trio (`record_ledger_from_env` / `try_append_ledger` / `append_ledger_entry`), `estimate_prevented_reads` + nested `META_CACHE`, `compile_glob` (still used by `passes/regex.rs`), git-head helpers, `#[cfg(test)] #[path = "../../../../tests/unit/core/search.rs"] mod tests`.

Façade: `pub use finish::finish_response` so `crate::search::finish_response` and `ast_sgrep_core::search::finish_response` keep working (`tests/core/signal_provenance.rs`, `pipeline_parts.rs`). Unit-test helpers re-imported under `#[cfg(test)]` for the existing `#[path]` suite.

## Evidence

### Behavior
- Command: `rch exec -- cargo test --workspace --no-fail-fast` (spark-1672)
- Result: **488 passed / 0 failed / 4 ignored** (exit 0)
- Matches Phase 3 baseline `baseline_tests.json` (488/0/4)

### Public API
- Command: `cargo +nightly public-api --simplified -p ast-sgrep-core` and `-p ast-sgrep-mcp`
- Diff vs workspace `api_snapshot_before.txt`: **0 removals, 0 additions** (set compare of package bodies)
- `search::finish` is crate-private (`mod finish;`) — no public leak; `finish_response` remains on `ast_sgrep_core::search`

### Structural
- No new `Box<dyn` / `Arc<dyn` / trait-object indirection
- No public symbol renames; no sqlite/index edits
- Shared `compile_glob` deliberately left in `mod.rs` (finish + regex pass)

### Gate script / SKIPPED
- GATE 4/5: **SKIPPED** (Phase 3 benches/compile-RSS incomplete; `--quick`)
- Binding proof is the manual suite + public-api runs above (same class as EXP-001/002)
- Workspace log: `ast-sgrep-wt-demonolith__demonolith_workspace/phase5_experiment_results/EXP-003.log`

## Non-goals (this pass)
- F-002 ledger trio extraction  
- F-004 META_CACHE / `estimate_prevented_reads`  
- F-005 Searcher Mutex caches (leave-alone)  
- sqlite / index  

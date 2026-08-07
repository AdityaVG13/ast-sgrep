# Pass 5 — Deep: search / ranking / fusion

**Date:** 2026-08-07  
**Scope:** `crates/ast-sgrep-core/src/search/**`, `fusion.rs`, `rank.rs` -- empty hits, limit 0, fuse/merge same-location, confidence calc, generation fence interaction.

## Bugs found

### P1 -- confidence assignment (fixed) -- `ast-sgrep-d2a1.7` (closed)

1. **`dedup=false` left confidence at 0.0.**  
   `Searcher::search_semantic` calls `finish_response_checked(..., dedup=false)`. Confidence was only assigned inside `dedup_hits`, so pure semantic results always reported `confidence: 0.0` despite the vh65 contract.

2. **Post-margin signal rewrite could inflate trust metadata.**  
   `estimate_confidence` read `hit.signal`. Multi-channel same-location merge can keep Embed as `kind` while contributors include Asgrep; confidence used Exact base, then `assign_signal_margins` rewrote display `signal` to Semantic without recomputing confidence.

**Fix:**
- `estimate_confidence` bases strength on strongest contributor (and `kind`), not display `signal` alone.
- New `assign_hit_confidence`; always run after `assign_signal_margins` in `finish_response_checked`.

### P3 -- JSON deserialize zeros confidence (fixed) -- `ast-sgrep-d2a1.8`

`SearchHit` serializes `confidence` but custom `Deserialize` via `SearchHitWire`
previously omitted the field, so JSON round-trip always yielded `0.0`.

**Fix:** `SearchHitWire` carries `#[serde(default)] confidence: f64`; deserialize
maps it and sanitizes non-finite values to `0.0` (same policy as `margin`).
Missing field still defaults to `0.0`.

## Regression evidence

```text
cargo test -p ast-sgrep-core --lib search::types::tests
# 5 passed:
#   confidence_uses_strongest_contributor_not_display_signal
#   semantic_only_confidence_is_nonzero_without_dedup
#   empty_hits_confidence_assign_is_noop
#   search_hit_json_round_trip_preserves_confidence
#   search_hit_json_missing_confidence_defaults_zero

cargo test -p ast-sgrep-core --lib confidence
# also: finish_response_assigns_confidence_when_dedup_false

# Also green (prebuilt lib test bin):
fusion::tests::* (5)
hybrid_window_retains_definition_evidence
pretruncate_keeps_high_coverage_lower_score
rank score_def_and_caller_zero_when_no_coverage / coverage monotone
```

No commit (per mission).

## Verified correct paths (≥3)

1. **`apply_weighted_rrf` same-location merge**  
   - Key `(file, line_start)`; multi-channel contributors sorted/deduped; zero/non-finite scores excluded (not fused).  
   - Same-channel duplicates do not consume extra RRF ranks (`same_channel_duplicates_do_not_consume_rrf_positions`).  
   - Empty input early-returns; all-zero input yields empty fused list (intentional).

2. **`Searcher` limit 0 / oversize**  
   - `clamp_output_limit` + `Searcher::new` remap `limit: 0` → default 16 and cap at `MAX_OUTPUT_RESULTS` (`searcher_remaps_zero_and_oversize_limit`).  
   - `enforce_result_gates` with `limit == 0` truncates to empty without panic (`head > 0` guards def pin).

3. **Generation fence + response cache**  
   - `fenced`: `BEGIN DEFERRED` fail-closed when autocommit; gen re-read after compute; mismatch errors when this call owns the snapshot.  
   - `cached`: only stores when pre/post `index_gen` match; poison clears cache. No new gen-fence bug found under pass5 review.

4. **Ranking gates**  
   - Hybrid def retention via `enforce_result_gates` + pre-truncate coverage key (`8mb8`) behave as tested.  
   - `route_hits` normalizes only; intent weights applied once in fusion (no double multiply).

5. **Symbol / RRF score math**  
   - `rrf_score(rank, k) = 1/(k+rank+1)` with 0-based ranks is 1-based RRF-equivalent.  
   - Single-char substring blocked; case-insensitive term side; def/caller zero when no coverage.

## Beads

| ID | Status | Labels |
|---|---|---|
| `ast-sgrep-d2a1.7` | closed (fixed) | `bug-hunt`, `pass5-search` |
| `ast-sgrep-d2a1.8` | closed (fixed) | `bug-hunt`, `pass5-search` |

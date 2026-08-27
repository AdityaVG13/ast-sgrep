# Audit 1: response-finishing correctness (finish.rs / fusion.rs dedup_hits / types.rs signal+confidence)

Repo: ast-sgrep @ fix/bun-sqlite-and-auto-index (read-only inline audit by session agent;
two subagent attempts died to provider 524 timeouts at delivery).
Date: 2026-08-23. Scope: finish.rs (371 lines, full), fusion.rs (72 lines, full),
types.rs targeted reads (assign_signal_margins L566-606, assign_hit_confidence L608-613,
estimate_confidence L658-673, merge_channel_evidence L616-655).

## Finding 1 (LOW): tie-break gap can violate the documented cross-process byte-stability contract

- file: crates/ast-sgrep-core/src/search/finish.rs:62-82 (cmp_ranked_hits) +
  crates/ast-sgrep-core/src/search/passes/lexical.rs:199-206 (hits_from_matches)
- Trigger class: two DISTINCT hits sharing (file, line_start, line_start-equal spans) whose
  scores AND coverages compare Equal (e.g. two callers of different callees on the same
  source line with equal normalized scores). cmp_ranked_hits ends at
  `a.line_start.cmp(&b.line_start)` and returns Equal for such pairs;
  `keyed.sort_unstable_by` is not stable, and the input order feeding it comes from
  `hits_from_matches`, which iterates a `HashMap` whose SipHash seed is randomized per
  process. Same query, same index, two different MCP/server processes -> the tied pair can
  serialize in either order.
- Why LOW: requires exact score+coverage ties on identical spans; the 35-contract battery
  never produces them. But crates/ast-sgrep-mcp/src/lib.rs documents "Search envelopes are
  deterministic for the same query and index generation", and MCP servers restart between
  calls often.
- Repro sketch: fixture with one line containing two calls (`foo(); bar();`) where both
  callee names tokenize to equal-score terms; run codemode-serve twice as separate
  processes, diff serialized hits. RED = order flips across runs.
- Fix sketch (production, failure-first): extend cmp_ranked_hits with
  `.then_with(|| a.line_end.cmp(&b.line_end)).then_with(|| a.symbol.cmp(&b.symbol))`
  (or fall back to `sort_by` + explicit total key) so the comparator is a total order.

## Ruled out (checked, refuted)

- Double confidence assignment (dedup_hits -> assign_hit_confidence, then again in
  finish_response_checked): estimate_confidence is a pure function of (kind, contributors);
  it never reads prior confidence or display signal. Idempotent; second call exists to
  serve the dedup=false path. Safe.
- assign_signal_margins rewriting display `signal` from `kind` before confidence:
  confidence ignores `signal` entirely (uses kind/contributors ranks). No order dependency.
- cap_per_file overflow movement + definition promotion (enforce_result_gates):
  remove+insert preserves vector length; no capped-file resurrection; final
  truncate(limit) always bounds; promotion is deterministic (first Def in current order).
- best_definition push-after-truncate exceeding limits: bounded by enforce_result_gates'
  truncate(limit) immediately after.
- excerpt_term_coverage / contains_term_token UTF-8 safety: match_indices yields
  char-boundary-aligned ranges; all slicing happens at those boundaries. Byte-safe.
- dedup_hits ordering: output preserves first-occurrence order; HashMap is lookup-only,
  never iterated for output.
- count_only early return: emits only per-file counts; no un-finished hit fields leak.
- finish_response compatibility wrapper dropping invalid globs: deliberate, documented
  legacy behavior (comment at finish.rs:91-93).

## Verdict

No high/medium correctness defects found in the finishing path. One LOW determinism gap
(Finding 1) worth a failure-first fix when the campaign next touches finish.rs.

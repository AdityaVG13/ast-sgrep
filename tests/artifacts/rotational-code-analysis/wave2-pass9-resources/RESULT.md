# RESULT — Wave 2 / Pass 9 (HARDEN Loop 14 resources/backpressure)

```text
SPIN_THE_BLOCK_RESULT:
status: complete
mode: harden
target_root: /Users/aditya/Developer/ast-sgrep
prior_state_leveraged: true
wave: 2
campaign_pass: 9
iteration: 21
product_safe: true
product_source_edits: no
PRODUCTIVE: false
ZERO_CHANGE: true
residual_closed: none
residual_opened:
  - R-EMBED-HTTP-TIMEOUT-BODY   # GAP availability/saturation; not dual-evidence wrong-answer
  - R-PATTERN-UNBOUNDED-READ    # GAP vs MAX_INDEX_FILE_BYTES asymmetry
  - R-CM-SOFT-TIMEOUT-ORPHAN    # GAP capacity bleed; NAPI Mutex serializes correctness
residual_retained: R-PI-EDIT-SYMLINK-LEXICAL  # Refuse this loop (out of scope)
technique: cost-model / capacity-planner audit; no product edit (no high correctness-under-load small fix)
axes_changed: 4
axes: representation:cost-model | observer:capacity-planner | scale:request→fleet | time:load
vs_pass8: policy-lattice/attacker+tenant/identity→resource → cost-model/capacity-planner/request→fleet/load
void_avoided: V-SAME-GAZE (schema/watch/generation/root not re-done)
frozen_revision_pass1: 62ee4b4595ad2433bd16b0ac14747dada612b4d6
head_at_verify: 872cc82a73d387f97f391497b3b642238fbdae23 (dirty; books + beads/Pi leftover)
dirty: true
dirty_note: ZERO-CHANGE product; books only; no Pi runtime.ts; no R-PI-EDIT-SYMLINK-LEXICAL
zerostack: unavailable-fszero-codemode
independent: dual-evidence for GAPs (source ↔ ureq defaults / io_bounds asymmetry); no loop27 promote
braid_resolve: Continue
NEXT_PASS: Loop 15 (risk ring) or Seal wave-2; optional packets for embed timeout/body + pattern read cap; optional R-PI-EDIT-SYMLINK-LEXICAL
void_fixture_outcome: n/a mid-wave harden
north_star_probe_outcome: n/a product harden
independent_loop27: n/a
```

## Gate

- [x] New axes ≥2 vs pass 8 (cost-model + capacity-planner + request→fleet + load)
- [x] ≥3 concrete Loop 14 sites checked (table below)
- [x] No new high/critical dual-evidence correctness-under-load bug with small fix → ZERO-CHANGE
- [x] V-SAME-GAZE avoided on schema / watch / generation / root
- [x] No Pi `runtime.ts` / `index.ts` leftover edits
- [x] Cost + saturation model + unbounded paths + backpressure inventory written

## Sites (≥3)

| # | Site | Verdict | Why |
|---|------|---------|-----|
| 1 | Embed HTTP `embed_http_agent` + `into_json` | **GAP** | `redirects(0)` only; ureq defaults `timeout_read=None` / `timeout=None` (connect 30s). Pass9 concurrency books assumed timeouts. Hang/huge body → index stall/OOM; not silent wrong hits. Small timeout+body cap is hardening, not this-pass correctness fix. |
| 2 | `pattern.rs` native `fs::read` vs `MAX_INDEX_FILE_BYTES` | **GAP** | Index refuses >64 MiB; pattern walk loads full files in rayon. OOM under huge json/md → crash (fail-closed), not wrong ranked hits. |
| 3 | MCP `index_lock` + `INDEX_REPO_DEADLINE` + stdio serial | **CONSISTENT** | Single-flight; soft 600s pre/post; stdin line-serial so search∥index in-process cannot amplify. ESC-3 post-mutate deadline retained. |
| 4 | `MAX_SCAN_BYTES` (MCP `scan_line_window` + Pi `code-mode.ts`) | **CONSISTENT** | 64 MiB scan + `max_chars` / `MAX_READ_CHARS`; TOCTOU reopen checks stay outside scan. |
| 5 | Query / result limits (`MAX_QUERY_CHARS`, `clamp_agent_limit`, `MAX_OUTPUT_RESULTS`, MCP schema maxLength) | **CONSISTENT** | Shared `limits.rs`; MCP schema + parse enforce. |
| 6 | IVF build memory (`build_from_flat` clone+kmeans) + mmap open | **CONSISTENT** | Peak O(n·dim) expected at threshold; open path mmap + `read_clusters_bounded`. Cost residual, not correctness hole. |
| 7 | Lockfile dumps (`Cargo.lock` / `package-lock.json`) | **CONSISTENT** | Extension policy: `.lock` skipped (`should_skip_file`); `.json` indexable but capped by `MAX_INDEX_FILE_BYTES`. Noise/cost, not unbounded. |
| 8 | Code Mode `runCodemode` Promise.race timeout | **GAP** | Soft wall returns Err; AsyncFunction may keep calling pooled NAPI `Session` (Mutex). Capacity bleed under load; correctness serialized. |
| 9 | `R-PI-EDIT-SYMLINK-LEXICAL` / sandbox_root / schema | **Refuse** | Different loop / V-SAME-GAZE if re-described. |

## Diff summary (product)

None (ZERO-CHANGE).

## Verify

```text
ZERO-CHANGE — no RCH cargo product verify this pass.
Evidence commands: tests/artifacts/rotational-code-analysis/wave2-pass9-resources/commands.md
```

## Braid

**Freeze(retained) → Axis(cost-model + capacity-planner + request→fleet + load) → Enact(source+ureq-default dual evidence; site table) → Independent(n/a promote; GAP dual-evidence recorded) → Residual(3 GAPs opened; R-PI-EDIT retained Refuse) → Resolve Continue**

## Failure modes (named)

1. Allowlisted embed endpoint slow-loris or multi-GB JSON → indexer/MCP flight stuck or OOM; warm Searcher may still answer until process death.
2. `pattern:` on a tree of huge indexable files → parallel heap blowup.
3. Timed-out Code Mode continues tool calls under shared session lock → delayed capacity for the next program; agent already saw timeout Err.

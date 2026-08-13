# RESULT — Wave 2 / Pass 6 (HARDEN Loop 9 persistence)

```text
SPIN_THE_BLOCK_RESULT:
status: complete
mode: harden
target_root: /Users/aditya/Developer/ast-sgrep
prior_state_leveraged: true
wave: 2
campaign_pass: 6
iteration: 18
product_safe: true
product_source_edits: yes
residual_closed: R-MISSING-GEN-FALLTHROUGH
bead: ast-sgrep-aeft
technique: fail-closed try_index_db_path when active generation dir missing; refuse stale flat legacy / empty create
axes_changed: 3
axes: representation:state-store-model | observer:data-integrity | time:commit+recovery
vs_pass5: lifecycle-runbook/operator/docs → state-store/data-integrity/commit+recovery
frozen_revision_pass1: 62ee4b4595ad2433bd16b0ac14747dada612b4d6
head_at_verify: a8458dd609adc0a367707e5b4647da70c20a171b (dirty; product edits uncommitted)
dirty: true
dirty_note: store path fail-closed + generation_swap pin + index-consistency doc; no Pi leftover; no Searcher/root/xproc/FastUnsafe reopen
zerostack: unavailable-fszero-codemode
independent: dual-evidence source+generation_swap test (originator harden; loop27 n/a this mid-wave)
braid_resolve: Continue
NEXT_PASS: Seal wave-2 (residuals closed) or authorized Pi leftover only
PRODUCTIVE: true
void_fixture_outcome: n/a mid-wave harden
north_star_probe_outcome: n/a product harden
independent_loop27: n/a
```

## Gate

- [x] New axes ≥2 vs passes 2–5 (not V-SAME-GAZE on invalidate/root/stamp/FastUnsafe docs)
- [x] ≥3 concrete Loop 9 sites checked (table below)
- [x] New high with dual evidence + small fix shipped
- [x] RCH `cargo test -p ast-sgrep-core --test generation_swap` → 5 passed
- [x] No Pi `runtime.ts` edits

## Crash / coherence sites (≥3)

| # | Site | Verdict | Why |
|---|------|---------|-----|
| 1 | `try_index_db_path` when `active.json` names missing `generations/<id>/` | **FIXED** (was silent stale legacy) | Leftover flat `index.db` from pre-gen index could answer wrong corpus; now Err refuse |
| 2 | `index_all`: bulk SQLite commit → `advertise_writer_generation` → sidecar rebuild | **CONSISTENT** | Mid-sidecar Err: durable rows + stamp; lexical `is_fresh` / IVF fingerprint fall back to SQLite; MCP/CM invalidate-on-Err already closed |
| 3 | `semantic.ivf` publish (`*.tmp` + fsync + rename) | **CONSISTENT** | Torn write never under final name; miss → flat cosine |
| 4 | Tantivy `lexical.db` rebuild (`BEGIN`…`DELETE`…`INSERT`…`source_generation`…`COMMIT`) | **CONSISTENT** | Single tx; crash rolls back; stale gen → FTS fallback (`freshness_identity`) |
| 5 | `reindex_into_new_generation` build-then-`write_active_manifest` | **CONSISTENT** | Destroyed candidate leaves active serving (`generation_swap`) |
| 6 | Pinned `ASGREP_INDEX_PATH` in-place `clear_all_data`+rebuild | **Refuse / by-design** | Documented CL-PINNED crash window; architectural campaign, not this pass |

## Diff summary (product)

| File | Change |
|------|--------|
| `crates/ast-sgrep-core/src/store/mod.rs` | Missing active gen → Err; `index_db_path` diagnostics prefer broken gen path |
| `crates/ast-sgrep-core/tests/generation_swap.rs` | Pin test `missing_active_generation_refuses_stale_legacy_fallthrough` |
| `docs/index-consistency.md` | Corrupt active generation fail-closed note |

## Verify

```text
RCH_CANONICAL_PROJECT_ROOT=/Users/aditya \
rch exec -- … cargo test -p ast-sgrep-core --test generation_swap
  ok. 5 passed (incl. missing_active_generation_refuses_stale_legacy_fallthrough)
```

## Braid

**Freeze(retained) → Axis(state-store+data-integrity+commit/recovery) → Enact(fail-closed missing gen) → Independent(source+test) → Residual(R-MISSING-GEN closed; wave2 seal next) → Resolve Continue**

## Failure modes (named)

1. Operator deletes `generations/<active>/` while keeping `active.json` → open/search/index now hard-fail until restore/reindex (intentional).
2. Auto-rollback to `manifest.previous` not implemented — fail-closed only; optional future recovery campaign.
3. Crash-before-`writer_generation` bump remains Option C lite best-effort (not reopened this pass).

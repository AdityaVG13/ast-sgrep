# RESULT — Wave 2 / Pass 7 (HARDEN Loop 12 serialization)

```text
SPIN_THE_BLOCK_RESULT:
status: complete
mode: harden
target_root: /Users/aditya/Developer/ast-sgrep
prior_state_leveraged: true
wave: 2
campaign_pass: 7
iteration: 19
product_safe: true
product_source_edits: yes
residual_closed: R-NEWER-SCHEMA-SILENT-OPEN
technique: fail-closed init_schema when user_version > SCHEMA_VERSION; migration tests pin to schema_version()
axes_changed: 3
axes: representation:wire/storage-format | observer:old+new-peer | time:upgrade
vs_pass6: state-store/data-integrity/commit+recovery → wire-format/old+new-peer/upgrade
frozen_revision_pass1: 62ee4b4595ad2433bd16b0ac14747dada612b4d6
head_at_verify: b9ff8a0f015830a0503eea0df63487f42f9adac2 (dirty; product edits uncommitted)
dirty: true
dirty_note: sqlite init_schema refuse + semantic_chunk_migration pins/test; no Pi leftover; no generation fallthrough reopen
zerostack: unavailable-fszero-codemode
independent: dual-evidence source+semantic_chunk_migration (originator harden; loop27 n/a this mid-wave)
braid_resolve: Continue
NEXT_PASS: Loop 13 auth/isolation (wave2 pass 8) or Seal if stop rule hits
PRODUCTIVE: true
void_fixture_outcome: n/a mid-wave harden
north_star_probe_outcome: n/a product harden
independent_loop27: n/a
```

## Gate

- [x] New axes ≥2 vs pass 6 (not V-SAME-GAZE on missing-gen fallthrough)
- [x] ≥3 concrete Loop 12 sites checked (table below)
- [x] New high with dual evidence + small fix shipped
- [x] RCH `cargo test -p ast-sgrep-core --test semantic_chunk_migration` → 4 passed
- [x] No Pi `runtime.ts` edits

## Compatibility sites (≥3)

| # | Site | Verdict | Why |
|---|------|---------|-----|
| 1 | `IndexStore::init_schema` when `PRAGMA user_version > SCHEMA_VERSION` | **FIXED** (was silent open) | Old binary opened future schema; wrong answers / mid-query SQL risk. Now Err with upgrade message |
| 2 | `semantic_chunk_migration` post-migrate `user_version` pins | **FIXED** | Asserted `== 7` after SCHEMA_VERSION moved to 9 (tests red). Now pin to `migrated.schema_version()` |
| 3 | MCP `handle_initialize` protocolVersion negotiate | **CONSISTENT** | Exact match for `2024-11-05` / `2026-07-28`; unknown → current (`protocol.rs`) |
| 4 | Machine JSON envelope `schema_version: "1.0.0"` + status `writer_generation` | **CONSISTENT** | Distinct from store schema; shapes golden includes `writer_generation`; Pi mismatch gate separate namespace |
| 5 | Compact envelope `v: 1` (plugins + MCP outputSchema) | **CONSISTENT** | Stable schema id; docs + formatters agree |
| 6 | `ActiveManifest.schema_version` vs on-disk user_version | **Refuse / by-design** | Advisory activation metadata; DB `user_version` is the hard gate after this pass |

## Diff summary (product)

| File | Change |
|------|--------|
| `crates/ast-sgrep-core/src/store/sqlite.rs` | Refuse `user_version > SCHEMA_VERSION`; equality path for current |
| `crates/ast-sgrep-core/tests/semantic_chunk_migration.rs` | Pin migrate landing to `schema_version()`; add `newer_than_binary_schema_refuses_open` |

## Verify

```text
RCH_CANONICAL_PROJECT_ROOT=/Users/aditya \
rch exec -- cargo test -p ast-sgrep-core --test semantic_chunk_migration -- --nocapture
  ok. 4 passed (incl. newer_than_binary_schema_refuses_open)
```

## Braid

**Freeze(retained) → Axis(wire/storage + old+new-peer + upgrade) → Enact(fail-closed future schema + migration pin fix) → Independent(source+test) → Residual(R-NEWER-SCHEMA closed) → Resolve Continue**

## Failure modes (named)

1. Operator runs older asgrep against an index written by a newer binary → hard open failure until upgrade or reindex (intentional).
2. `ActiveManifest.schema_version` may lag on-disk after in-place migrate of an existing generation; peers must trust `user_version`, not the manifest field alone.
3. Machine / compact / store version namespaces remain separate; conflating them in agent code is still a consumer footgun (documented, not changed).

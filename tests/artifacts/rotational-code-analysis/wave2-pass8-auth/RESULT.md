# RESULT — Wave 2 / Pass 8 (HARDEN Loop 13 auth/isolation)

```text
SPIN_THE_BLOCK_RESULT:
status: complete
mode: harden
target_root: /Users/aditya/Developer/ast-sgrep
prior_state_leveraged: true
wave: 2
campaign_pass: 8
iteration: 20
product_safe: true
product_source_edits: yes
residual_closed: R-WATCH-SYMLINK-ESCAPE
residual_opened: R-PI-EDIT-SYMLINK-LEXICAL
technique: refuse existing symlinks in normalize_watch_path (WalkDir follow_links(false) parity)
axes_changed: 3
axes: representation:policy-lattice | observer:attacker+tenant | scale:identity→resource
vs_pass7: wire/old+new-peer/upgrade → policy-lattice/attacker+tenant/identity→resource
vs_pass3: not re-describing sandbox_root (V-SAME-GAZE)
frozen_revision_pass1: 62ee4b4595ad2433bd16b0ac14747dada612b4d6
head_at_verify: c6f0cce5d9ab29dad871575769a2f0b969a2e759 (dirty; watch symlink fix uncommitted)
dirty: true
dirty_note: index normalize_watch_path + watch_incremental symlink test; no Pi runtime/index leftover; no sandbox_root reopen
zerostack: unavailable-fszero-codemode
independent: dual-evidence source (WalkDir follow_links(false) vs metadata-follow) + watch_incremental test
braid_resolve: Continue
NEXT_PASS: Loop 14 or Seal wave-2 if stop rule; optional R-PI-EDIT-SYMLINK-LEXICAL
PRODUCTIVE: true
void_fixture_outcome: n/a mid-wave harden
north_star_probe_outcome: n/a product harden
independent_loop27: n/a
```

## Gate

- [x] New axes ≥2 vs pass 7 (not V-SAME-GAZE on sandbox_root / schema)
- [x] ≥3 concrete Loop 13 sites checked (table below)
- [x] New high with dual evidence + small fix shipped
- [x] RCH `cargo test -p ast-sgrep-cli --test watch_incremental` → 2 passed
- [x] No Pi `runtime.ts` / `index.ts` leftover edits

## Auth / isolation sites (≥3)

| # | Site | Verdict | Why |
|---|------|---------|-----|
| 1 | `normalize_watch_path` + `update_paths` file symlink → outside content | **FIXED** | Lexical under-root + `is_file`/`metadata` follow; poisoned index/search excerpts. Now refuse existing symlinks (WalkDir parity). |
| 2 | `ASGREP_INDEX_PATH` / `--index-path` absolute writable DB | **CONSISTENT / by-design** | Privileged sink labeled (`docs/env-trust.md`, agent ops, MCP docs). Not a silent unlabeled escape. |
| 3 | MCP / CM / NAPI tool `root` jail vs CLI operator root | **CONSISTENT** | MCP+CM+NAPI jailed (pass 3); CLI operator-trusted by design; Pi `resolveRuntimeRoot` realpath + explicit-only `allowOutsideProject`. |
| 4 | Device / proc fd paths | **CONSISTENT (Pi)** / **N/A (MCP edit)** | Pi `assertSafeEditTarget` / read path refuse `/dev/*` + `/proc/*/fd/*`. MCP/CM have no edit surface. Index path can still be any writable file (privileged sink #2). |
| 5 | Pi `planEdit` lexical `resolve` without realpath | **OPEN residual** | Symlink under project → write follows outside. Dual vs `resolveRuntimeRoot`/`code-mode` realpath. Left for follow-up (not dirty leftover files). |
| 6 | MCP `code_read` / `sandbox_root` | **Not reopened** | Pass 3 Option A + TOCTOU `same_opened_file`; V-SAME-GAZE if re-described. |

## Diff summary (product)

| File | Change |
|------|--------|
| `crates/ast-sgrep-core/src/index.rs` | `normalize_watch_path` refuses existing symlinks |
| `crates/ast-sgrep-cli/tests/watch_incremental.rs` | `update_paths_refuses_symlink_escape_into_index` |

## Verify

```text
RCH_CANONICAL_PROJECT_ROOT=/Users/aditya \
rch exec -- cargo test -p ast-sgrep-cli --test watch_incremental -- --nocapture
  ok. 2 passed (incl. update_paths_refuses_symlink_escape_into_index)
```

## Braid

**Freeze(retained) → Axis(policy-lattice + attacker+tenant + identity→resource) → Enact(watch symlink refuse) → Independent(source asymmetry + test) → Residual(R-WATCH closed; R-PI-EDIT-SYMLINK open) → Resolve Continue**

## Failure modes (named)

1. Planted in-tree symlink + `asgrep watch` / `update_paths` previously indexed outside bytes into the project DB; search could return them even when `code_read` canonicalize would fail.
2. `ASGREP_INDEX_PATH` still lets a confused-deputy host rewrite any writable DB the process can open -- intentional privilege, not a tool-root jail.
3. Pi edit can still follow a project-local symlink on write until `planEdit` gains realpath containment (`R-PI-EDIT-SYMLINK-LEXICAL`).

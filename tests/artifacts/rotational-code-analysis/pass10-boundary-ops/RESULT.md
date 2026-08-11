# Pass 10 RESULT — Boundary / adversary + ops

| Field | Value |
|-------|-------|
| Loop | 10 / boundary-adversary-ops (campaign pass 10; protocol boundary+ops) |
| Status | **COMPLETE** |
| Mode | audit (no product edits under crates/ or packages/) |
| Freeze | `fb932aac852f5496c0a7035cc5a0b508e05111cb` (retained; HEAD may hold books) |
| Axes | representation:**attack-surface+ops** · observer:**attacker+operator** · scale:**boundary** · evidence:**source+config** |
| Axes vs pass 9 | **≥4** (from interleaving/scheduler → attack-surface/attacker/boundary/config) |
| Braid | **Continue** → pass 11 independent verification |
| Prior state leveraged | true (pass 9 residuals + pass 8 BY-* + pass 4 trust map + pass 5 INV) |

## Deliverables

| Artifact | Path |
|----------|------|
| Adversary findings table | `iterations/10-boundary-ops/adversary-findings-table.md` |
| Ops failure signals map | `iterations/10-boundary-ops/ops-failure-signals-map.md` |
| Machine result | `iterations/10-boundary-ops/loop-10-result.json` |
| Slim mirror | `tests/artifacts/rotational-code-analysis/pass10-boundary-ops/` |

## 1. Pass 9 residual disposition

| Residual (from pass 9 §4) | Disposition this pass |
|---------------------------|----------------------|
| BY-CM-ROOT / C2 × concurrent index | **CONTRADICTION** C2 reaffirmed; **GAP** amplify: free root + shared `index_path` → walk/prune foreign into pinned DB; plan `$ref` composition |
| path_registry stale after failed index | **GAP** reaffirmed (BY-REGISTRY-STALE); read path still re-jails under MCP root |
| GAP-WATCH-XPROC multi-writer ops | **GAP** high — watch stderr-only; no MCP/CM invalidate; ops map documents false-negative |
| Pinned index_path privilege + crash | **BY-DESIGN** privilege + **GAP** gen-layout disable / CL-PINNED-REINDEX |
| Embed allowlist / no-redirect | **CONSISTENT** reaffirm (unit pins; redirects(0)); GAP-EMBED-REDIR-IT demoted to optional live IT |
| FastUnsafe ops footgun | **BY-DESIGN** named; **GAP** doctor/ops non-warning; MCP/CM inherit env |
| Dual-evidence / beads | deferred to pass 11 (no product R-* this pass) |
| Retain C1, GAP-CM-ROOT, GAP-XOR-RUNTIME, GAP-RO-HOST, B-ZS-ENGINES, B-DIRTY-FREEZE, B-SECURITY-NAPI-DOC | retained |

## 2. Top boundary/ops findings

| Rank | ID | Summary | Status | Severity |
|------|-----|---------|--------|----------|
| 1 | **C2** / **BY-CM-ROOT** × index | CM free root vs MCP jail; shared index_path + prune = durable corpus rewrite | CONTRADICTION + GAP | high |
| 2 | **GAP-WATCH-XPROC** | watch+MCP multi-writer; silent stale Searcher | GAP | high (ops) |
| 3 | **CL-MID-SIDECAR-CACHE** | post-commit sidecar Err → no invalidate; agent error lies | GAP | high |
| 4 | **INV-INDEX-PATH-PRIV** | absolute index path anywhere; pin kills atomic gen reindex | BY-DESIGN + GAP | medium–high |
| 5 | **FastUnsafe** | env-inherited power-loss corruption risk; doctor silent | BY-DESIGN + GAP ops | medium |
| 6 | **BY-REGISTRY-STALE** | path_registry uncleared on index Err | GAP | medium |
| 7 | **Embed SSRF** | allowlist + redirects(0) | **CONSISTENT** | — |
| 8 | **ESC-3** / deadline | mutate then soft fail ack | known | low–med |

No new product **R-*** filings (audit books only). No invented CVEs or benchmark numbers.

## 3. Ops observability headline

Production reveals **tool string errors** and **on-demand status/doctor**. It does **not** reveal multi-writer staleness, committed-but-errored index, or FastUnsafe as a health issue. See `ops-failure-signals-map.md`.

## 4. Residual for pass 11 (independent verification + beads)

Pass 11 card: independent verification, dual-evidence, loop-27 style, optional bead promotion.

1. **Independent re-prove** of C2 foreign-root + shared `ASGREP_INDEX_PATH` prune (minimal integration or documented manual proof) without relying only on pass 10 narrative.
2. **Independent re-prove** mid-sidecar non-invalidate (MCP unit or scripted) — promote dual-evidence for CL-MID-SIDECAR-CACHE.
3. **Watch×MCP** staleness: decide product vs ops-doc only; if product, R-* for xproc invalidate/lease.
4. **Doctor FastUnsafe issue** + structured MCP error codes — product judgment ASK.
5. **Bead promotion policy:** only high/critical with dual evidence (source + test or repro). Candidates if dual-evidence lands:
   - mid-sidecar invalidate-on-Err
   - registries clear on index Err
   - optional CM root jail or host-contract docs (C2 may stay intentional INV-CM-ROOT-FREE)
6. **Do not** file beads for CONSISTENT embed SSRF or pure docs C1 unless product asks.
7. Retain books freeze discipline; B-ZS-ENGINES; no commit/push unless authorized.
8. Pass 12 prep: release/residual seal, braid residue, queue PENDING high without loop27.

## Gate check

> Boundary failures that violate trust/privilege have a named control or are reported as GAP/CONTRADICTION with evidence.

**Met** — adversary table + ops map; embed reaffirmed CONSISTENT; multi-writer and free-root gaps named.

## Evidence commands

```
# zerostack engines unavailable (B-ZS-ENGINES)
rg -n 'fn root_arg|fn index_repo|fn sandbox_root|fn tool_index_repo|path_registry|fn read_node' \
  crates/ast-sgrep-codemode/src/session.rs crates/ast-sgrep-mcp/src/lib.rs
rg -n 'fn try_index_db_path|enum Durability|FastUnsafe|generation_layout_root|fn reindex_all' \
  crates/ast-sgrep-core/src/store/mod.rs crates/ast-sgrep-core/src/index.rs
rg -n 'fn run_watch|rebuild_dirty_sidecars|apply_bulk_write_result' \
  crates/ast-sgrep-cli/src/watch.rs crates/ast-sgrep-core/src/index.rs
rg -n 'embed_url_is_allowed|redirects\(0\)|embed_http_agent_disables' \
  crates/ast-sgrep-embed/src/embedder.rs
rg -n 'fn run_doctor|doctor_triage|isError|ASGREP_DURABILITY' \
  crates/ast-sgrep-cli/src/agent.rs crates/ast-sgrep-mcp/src/lib.rs crates/ast-sgrep-cli/src/cli_args.rs
# docs: docs/env-trust.md docs/index-consistency.md docs/INSTRUMENTATION.md
```

## Counts

- Adversary/ops finding rows (ranked): **12** (+ positive controls table)
- CONSISTENT boundary controls: **9+**
- High GAP/CONTRADICTION: **3** (C2 amplify, WATCH-XPROC, MID-SIDECAR)
- New product R-*: **0**
- Beads filed: **0** (pass 11)

## Braid residue

```
SPIN_THE_BLOCK_RESULT:
status: in_progress
mode: audit
target_root: /Users/aditya/Developer/ast-sgrep
prior_state_leveraged: true
iteration: 10
coverage_pending: foundation loops 11-12
high_critical_without_loop27: n/a (audit observations; no new R-* product findings)
braid_resolve: Continue
axes_changed: 4+
void_fixture_outcome: n/a mid-wave
north_star_probe_outcome: n/a mid-wave
independent_loop27: pending (pass 11)
queue_action: none
books: .rotational-code-analysis/
```

# Pass 11 RESULT — Independent verification + residual queue

| Field | Value |
|-------|-------|
| Loop | 11 / independent-verification (campaign pass 11; protocol loop-27 style) |
| Status | **COMPLETE** |
| Mode | audit (no product edits under crates/ or packages/) |
| Freeze retained | `fb932aac852f5496c0a7035cc5a0b508e05111cb` |
| HEAD observed | `7cb1a28d53d5a5752ea62010b970e0b491d2dc75` (dirty) |
| Axes | observer:**skeptic non-originator** · evidence:**tests+source re-read** · scale:**residual only** · representation:**dual-evidence ledger** |
| Axes vs pass 10 | **≥4** (from attack-surface/ops → skeptic/tests/residual/dual-evidence) |
| Braid | **Continue** → pass 12 residual re-rotate / audit seal candidate |
| Prior state leveraged | true (pass 10 residual-pass11.md + adversary table) |
| Beads filed | **0** (open_issues=51; flood gate → markdown only) |
| Product R-* source fixes | **0** |

## Deliverables

| Artifact | Path |
|----------|------|
| Dual-evidence high findings | `dual-evidence-high-findings.md` |
| Residual scorecard | `residual-scorecard.md` |
| Work packets (4) | `work-queue/01`…`04-*.md` |
| Pass 12 handoff | `residual-pass12.md` |
| Machine result | `loop-11-result.json` |
| Books mirror | `.rotational-code-analysis/iterations/11-independent-verification/` |

## 1. Dual-evidence outcomes (TOP high)

| ID | Verdict | Dual status | Notes |
|----|---------|-------------|-------|
| **C2** CM free root vs MCP sandbox | CONTRADICTION **CONFIRMED** | DUAL-OK (source both + MCP sandbox test PASS) | Live foreign-root prune fixture not run; composition solid |
| **CL-MID-SIDECAR-CACHE** | GAP **CONFIRMED** | DUAL-OK (commit→sidecar order + Ok-only invalidate; Ok unit PASS; Err unit ABSENT) | Comment/code mismatch on MCP "always drop" |
| **GAP-WATCH-XPROC** | GAP **CONFIRMED** | DUAL-OK (watch mutates stderr-only + MCP process-local lock/cache; no xproc test) | Design ASK |

None REFUTED. No new high IDs invented.

### Cheap tests executed

```text
cargo test -p ast-sgrep-mcp --lib
# ok; 3 passed

cargo test -p ast-sgrep-mcp --test protocol tool_roots_are_sandboxed
# ok; 1 passed
```

`ast-sgrep-core --lib` subset did not compile (unrelated SearchHit/SearchResponse test helpers) — noted, not treated as H2 disconfirm.

## 2. Residual packets created

| ID | File | Disposition |
|----|------|-------------|
| R-CM-ROOT-POLICY | `work-queue/01-R-CM-ROOT-POLICY.md` | design ASK |
| R-INDEX-ERR-CACHE-SYNC | `work-queue/02-R-INDEX-ERR-CACHE-SYNC.md` | fix candidate |
| R-XPROC-MULTIWRITER | `work-queue/03-R-XPROC-MULTIWRITER.md` | design ASK |
| R-OPS-DOCS-FOOTGUNS | `work-queue/04-R-OPS-DOCS-FOOTGUNS.md` | optional ops/docs |

Slot 5 unused (anti-bloat).

## 3. Pass 12 ZERO-CHANGE seal?

| Question | Answer |
|----------|--------|
| Can pass 12 seal the **audit campaign** as ZERO-CHANGE? | **Yes, if** residual-only re-rotate finds no new material R-* and cites this scorecard |
| Does ZERO-CHANGE mean product gaps closed? | **No** — 02 remains FIX PENDING; 01/03 DESIGN PENDING |
| Should pass 12 implement fixes? | Only if user re-scopes off audit-only |

## Gate check

> High findings have non-originator dual evidence (source + test or second surface); residuals aggregated ≤5.

**Met.**

## Braid residue

```
SPIN_THE_BLOCK_RESULT:
status: in_progress
mode: audit
target_root: /Users/aditya/Developer/ast-sgrep
prior_state_leveraged: true
iteration: 11
coverage_pending: foundation loop 12 seal
high_critical_without_loop27: 0 (TOP 3 dual-evidenced this pass)
independent_loop27: complete
queue_action: residual_packets_4
packets:
  - R-CM-ROOT-POLICY
  - R-INDEX-ERR-CACHE-SYNC
  - R-XPROC-MULTIWRITER
  - R-OPS-DOCS-FOOTGUNS
braid_resolve: Continue
axes_changed: 4+
pass12_zero_change_audit_seal: eligible_if_no_new_R
books: .rotational-code-analysis/
```

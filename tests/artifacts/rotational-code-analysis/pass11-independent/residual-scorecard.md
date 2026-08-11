# Pass 11 — Residual R-* scorecard

Campaign residual work is **≤5 aggregated packets** (anti-bloat). Open beads at pass 11 start: **51** → **markdown work-queue only** (no `br create`).

## Dual-evidence status (TOP high)

| Ledger ID | Residual packet | Independent verdict | Dual-evidence | Promote product fix? |
|-----------|-----------------|---------------------|---------------|----------------------|
| C2 / BY-CM-ROOT | **R-CM-ROOT-POLICY** | CONTRADICTION retained | DUAL-OK asymmetry; PARTIAL live prune | No — design ASK |
| CL-MID-SIDECAR-CACHE | **R-INDEX-ERR-CACHE-SYNC** | GAP retained | DUAL-OK | **Yes** (surface invalidate on Err) |
| GAP-WATCH-XPROC | **R-XPROC-MULTIWRITER** | GAP retained | DUAL-OK | No — design ASK |

Tests actually run (pass 11):

```text
cargo test -p ast-sgrep-mcp --lib
# 3 passed (write_resp + 2 cache)

cargo test -p ast-sgrep-mcp --test protocol tool_roots_are_sandboxed
# 1 passed

cargo test -p ast-sgrep-core --lib apply_bulk_write_result
# COMPILE FAIL (unrelated SearchHit.resolution / SearchResponse fields in lib tests)
# — not used as disconfirming evidence for H2
```

## Aggregated residual packets (4 / max 5)

| # | Residual ID | Path | Sev | Disposition |
|---|-------------|------|-----|-------------|
| 1 | R-CM-ROOT-POLICY | `work-queue/01-R-CM-ROOT-POLICY.md` | high | DESIGN ASK |
| 2 | R-INDEX-ERR-CACHE-SYNC | `work-queue/02-R-INDEX-ERR-CACHE-SYNC.md` | high | FIX CANDIDATE |
| 3 | R-XPROC-MULTIWRITER | `work-queue/03-R-XPROC-MULTIWRITER.md` | high | DESIGN ASK |
| 4 | R-OPS-DOCS-FOOTGUNS | `work-queue/04-R-OPS-DOCS-FOOTGUNS.md` | med/low | OPTIONAL hygiene |
| — | *(slot 5 unused)* | — | — | anti-bloat reserve |

## Ledger ID → packet map (carry collapse)

| Prior ledger IDs | Packet |
|------------------|--------|
| C2, BY-CM-ROOT, GAP-CM-ROOT, GAP-CM-INV-TEST, B-SECURITY-NAPI-DOC, INV-CM-ROOT-FREE | 01 |
| CL-MID-SIDECAR-CACHE, RW-MCP-MID-SIDECAR, BY-REGISTRY-STALE, CL-INDEX-FAIL-REGISTRIES, CL-CM-POISON-INV, ESC-3 (slice) | 02 |
| GAP-WATCH-XPROC, GAP-XOR-RUNTIME, GAP-RO-HOST | 03 |
| FastUnsafe-ops, GAP-INDEX-PATH-DOC, CL-PINNED-REINDEX, INV-INDEX-PATH-PRIV, C1, B-DIRTY-FREEZE, B-ZS-ENGINES, ESC-3 (docs) | 04 |
| Embed SSRF / GAP-EMBED-REDIR-IT | **closed as CONSISTENT** — no packet |
| MCP sandbox / Ok-path invalidate / durability fail-closed parse | **CONSISTENT** — no packet |

## Pass 12 ZERO-CHANGE seal readiness

| Criterion | Status |
|-----------|--------|
| High findings independently re-verified | **Yes** (this pass) |
| Residual queue bounded ≤5 | **Yes** (4 packets) |
| No new material high beyond ledger | **Yes** |
| Product source unchanged (audit) | **Yes** |
| All high dual-evidence REFUTED | **No** — 3 CONFIRMED |
| Product fixes landed | **No** (audit-only) |

**Pass 12 can be a ZERO-CHANGE *audit seal*** if it only re-rotates residuals, finds no new R-*, and accepts the residual ledger as honest PENDING/DESIGN — **not** a claim that product gaps are fixed.

Pass 12 **cannot** seal "no residual risk" or "multi-writer/cache consistent" without product work on packets 01–03 (or explicit BY-DESIGN Accept with host duty for 01/03 and fix for 02).

Recommended pass 12 mode: **residual-only re-rotate → ZERO-CHANGE seal of audit campaign** with residual scorecard cited; do not file more packets unless new dual-evidence high appears.

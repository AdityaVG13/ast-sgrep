# R-OPS-DOCS-FOOTGUNS

| Field | Value |
|-------|-------|
| Residual ID | **R-OPS-DOCS-FOOTGUNS** |
| Aggregates | FastUnsafe-ops, GAP-INDEX-PATH-DOC, CL-PINNED-REINDEX (docs), INV-INDEX-PATH-PRIV, C1 cascade docs, ESC-3 error honesty, B-SECURITY-NAPI-DOC (docs slice), B-DIRTY-FREEZE (process), B-ZS-ENGINES (tooling) |
| Severity | medium / low (ops + docs; not TOP dual-evidence fix) |
| Status | **OPTIONAL** hygiene bundle — no severity inflation |
| Pass | 11 independent verification |
| Tracker | markdown only |

## Problem (bundled small residuals)

Campaign retained several **non-high-fix** items that still create operator/docs debt. Aggregated here so pass 12 does not re-spawn unbounded ledger IDs.

| Sub-ID | Issue | Suggested action |
|--------|-------|------------------|
| FastUnsafe-ops | `ASGREP_DURABILITY=fast-unsafe` opt-in power-loss risk; doctor silent; MCP/CM inherit env | Doctor issue or status warn when FastUnsafe active |
| GAP-INDEX-PATH-DOC / INV-INDEX-PATH-PRIV | Absolute `index_path` / env accepted anywhere writable | Document privileged sink |
| CL-PINNED-REINDEX | Pin disables generation atomic reindex → in-place crash window | Doc + optional warn when pin disables gen layout |
| C1 cascade docs | Docs vs hybrid empty-structural continue | Doc fix only |
| ESC-3 | Soft deadline after durable index → agent sees fail after work | Error string notes index may have committed |
| B-SECURITY-NAPI-DOC | NAPI = full-user Session / free root | Document next to CM host duty |
| B-ZS-ENGINES | tokenzero-codemode missing on this host | Install note; not product |
| B-DIRTY-FREEZE | Tree dirty during campaign; freeze sha retained | Process; books cite freeze + HEAD |

## Evidence

- Pass 10 adversary table ranks 4–12; pass 11 did **not** re-run embed SSRF (CONSISTENT retained).
- FastUnsafe / durability: `store/mod.rs` Durability enum; CLI clap/env; not re-expanded this pass.
- Embed CONSISTENT: allowlist + `redirects(0)` unit pins (prior passes).

## Acceptance (optional, partial OK)

- [ ] Doctor or status surfaces FastUnsafe when set
- [ ] Docs: privileged `ASGREP_INDEX_PATH`, pin disables gen reindex, NAPI host duty
- [ ] C1 doc aligned with cascade behavior **or** explicitly "historical mismatch known"
- [ ] ESC-3: error text mentions possible committed index when deadline fires post-mutation (if cheap with packet 02)

## Non-goals

- Replacing packets 01–03
- Invented CVEs or perf numbers
- Full doctor redesign

## Handoff

May remain open after ZERO-CHANGE audit seal. Do not block seal solely on this packet if 01–03 are dispositioned (design Accept or PENDING named).

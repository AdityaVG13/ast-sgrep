# CONVERGENCE.md — Pass 12 absolute convergence seal (rotational-code-analysis)

| Field | Value |
|-------|-------|
| Campaign | rotational-code-analysis |
| Pass | **12 / 12** |
| Mode | audit residual-only re-rotate · **ZERO product source change** |
| Observer | skeptic · residual-only (no new axes theater) |
| Representation | coverage-check vs pass 11 residual scorecard |
| Evidence | re-read scorecard + spot-check code anchors + cheap MCP tests |
| Freeze retained | `fb932aac852f5496c0a7035cc5a0b508e05111cb` |
| HEAD observed | `b2af241959461f4f71d37ee92e4a94779f59d8d7` |
| Pass 11 HEAD | `7cb1a28d53d5a5752ea62010b970e0b491d2dc75` (residual file digests unchanged) |
| Timestamp UTC | 2026-08-11T03:08:13Z |

## Verdict

# **CONVERGED** (audit)

**Definition used:** pass 11 dual-evidenced TOP highs; pass 12 residual-only re-rotate finds **no REFUTED** residual and **no NEW material R-***; product source under residual loci unchanged (content fingerprints match pass 11); skill-loop stop condition met for **audit books**.

**Not claimed:** product green / multi-writer safe / Err-path cache consistent / CM root jail parity.

```
coverage honest  ≠  product safe
```

Residual packets **remain open** for harden later (design ASK / fix candidate / optional hygiene).

## Seal criteria checklist

| Criterion | Status |
|-----------|--------|
| Re-read residual scorecard; spot-check one anchor per TOP high | **Met** — see `residual-stay-proof.md` N2–N6 |
| No new high GAP/CONTRADICTION with dual evidence | **Met** — zero new R-* |
| Residual queue bounded (≤5 packets) | **Met** — still 4; slot 5 unused |
| Product source unchanged by this pass (audit) | **Met** — ZERO under `crates/` |
| All high dual-evidence REFUTED | **N/A / No** — 3 high still CONFIRMED |
| Product fixes landed | **No** (audit-only policy retained) |
| Braid | **Seal** (audit campaign complete-with-residuals) |

## Residual inventory (final, open)

| # | Residual ID | Sev | Disposition | Pass 12 |
|---|-------------|-----|-------------|---------|
| 1 | **R-CM-ROOT-POLICY** | high | DESIGN ASK | STILL VALID |
| 2 | **R-INDEX-ERR-CACHE-SYNC** | high | FIX CANDIDATE | STILL VALID |
| 3 | **R-XPROC-MULTIWRITER** | high | DESIGN ASK | STILL VALID |
| 4 | **R-OPS-DOCS-FOOTGUNS** | med/low | OPTIONAL hygiene | STILL VALID |

Packets live at: `../pass11-independent/work-queue/01`…`04-*.md` (authoritative work items; pass 12 does not re-file).

## Closed / CONSISTENT (not reopened)

| Item | Status |
|------|--------|
| Embed SSRF allowlist + redirects(0) | CONSISTENT (pass 10/11; not re-expanded) |
| MCP Ok-path sandbox + Ok-path cache invalidate | CONSISTENT (tests re-green pass 12) |
| Durability fail-closed parse | CONSISTENT (prior) |

## Product vs books honesty

| Layer | State after pass 12 |
|-------|---------------------|
| **Audit campaign (passes 1–12)** | **CONVERGED / Seal** |
| **Product residual risk** | **OPEN** — packets 01–03 (and optional 04) |
| **Beads** | markdown queue only (flood gate; open beads were ≥50 at pass 11) |
| **Commit/push** | not authorized this turn |

## Evidence summary

- Fingerprints of residual loci: match pass 11 (`residual-stay-proof.md` N1).
- Cheap tests: `ast-sgrep-mcp --lib` 3 passed; `tool_roots_are_sandboxed` 1 passed.
- Control flow windows: MCP `?` before invalidate; CM free `root_arg`; commit→sidecar; watch stderr-only — all re-read.

## Skill-loop stop

**Stop.** No pass 13 required for audit completeness. Resume only when product authorizes harden work on residual packets (or residual anchors change under product commits).

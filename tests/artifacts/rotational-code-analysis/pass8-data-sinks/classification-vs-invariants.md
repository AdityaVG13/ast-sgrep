# Pass 8 — Classification vs prior invariants (CONSISTENT / GAP / CONTRADICTION)

Pass-5 ledger statuses re-checked on the **dataflow / source→sink** axis. New observations are **DF-*** / **BY-*** only (audit books; no new product R-* filings unless already residual).

## Status vocabulary

- **CONSISTENT** — enforcement path traced source→sink matches invariant statement.
- **GAP** — missing test, docs, or incomplete enforcement relative to a reasonable contract.
- **CONTRADICTION** — two authoritative claims disagree (docs vs code, or cross-surface).

---

## Re-validation of pass-5 invariants

| INV | Pass-5 | Pass-8 dataflow check | Notes |
|-----|--------|----------------------|-------|
| INV-MCP-SANDBOX | CONSISTENT | **CONSISTENT** | All MCP tools: wire → `resolve_root` → `sandbox_root` before sinks (search/index/read). `code_read` re-checks join+canon. |
| INV-CM-ROOT-FREE | GAP (vs MCP) / CONSISTENT w/ CM docs | **CONSISTENT** (docs) · **GAP** (host tests) | `root_arg` is pure `PathBuf::from`; sinks inherit free root. No negative CM test. |
| INV-SURFACE-ROOT-PARITY | CONTRADICTION | **CONTRADICTION (C2)** | Same data class `root` has opposite isolation policies by surface. |
| INV-INDEX-PATH-PREC | CONSISTENT | **CONSISTENT** | `try_index_db_path` order traced; testkit poison env. |
| INV-INDEX-PATH-PRIV | GAP | **GAP** | Absolute path → SQLite sink outside root; still unlabeled privilege. |
| INV-MCP-SEARCHER-INV | CONSISTENT | **CONSISTENT** (happy) | Invalidate + registry clear after successful index; **BY-REGISTRY-STALE** on Err (CL-INDEX-FAIL-REGISTRIES). |
| INV-CM-SEARCHER-INV | GAP | **GAP** | Cache clear present; poison no-op (pass 7); no parity test. |
| INV-BATCH-NO-MUT-PAR | CONSISTENT | **CONSISTENT** | Not re-opened as data path; still serializes mutators. |
| INV-RO-CATALOG | GAP | **GAP** | Catalog advisory; plan/`session.call` can still hit mutators. |
| INV-XOR-CM-MCP | GAP | **GAP** | Docs-only; dual load multiplies DF-ROOT policy confusion. |
| INV-EMBED-ALLOW | CONSISTENT | **CONSISTENT** | Allowlist + scheme + redirects(0) + re-check endpoint; unit tests. Query **content** not filtered (not claimed). |
| INV-DURABILITY-FC | CONSISTENT | **CONSISTENT** | Unknown env → Balanced; opens SQLite with profile. |
| INV-CASCADE-NO-WIDEN | CONSISTENT | **CONSISTENT** | Semantic file set ⊆ working_files; no path widen. |
| INV-CASCADE-STRUCT-EMPTY | CONTRADICTION | **CONTRADICTION (C1)** | Dataflow still implements lexical fallback (code truth). |
| INV-AST-GREP | CONSISTENT | **CONSISTENT** | Dual opt-in before Command sink. |
| INV-EDIT-ROOT | CONSISTENT | **CONSISTENT** | planEdit containment + device deny before write sink. |
| INV-LIMITS | CONSISTENT | **CONSISTENT** | Query/limit clamps on all major ingress paths. |
| INV-RANK-FUSION | CONSISTENT | **CONSISTENT** | Ranking uses query+hits in-process; no new sink claim. |

**Counts (unchanged set of 18):** CONSISTENT **11** · CONTRADICTION **2** · GAP **5** — reinforced, not flipped.

---

## New dataflow findings (audit observations)

| ID | Class | Statement | Evidence |
|----|-------|-----------|----------|
| DF-MCP-READ-CHAIN | CONSISTENT | MCP `code_read` full chain: sandbox → compact resolve → relative node → TOCTOU open | `lib.rs` parse_code_read / resolve_compact_id / read_node; protocol tests |
| DF-QUERY-EMBED | CONSISTENT | Query length validated before embed; URL allowlisted; body is free-form query | limits + embedder + embed pass |
| DF-INDEX-SINK | GAP | Index path resolution never re-attaches project-root authz | `try_index_db_path` |
| DF-PLAN-ROOT | GAP / C2 amplify | `$ref` can bind prior output into CM `root` without FS validation | `plan.rs` resolve_ref + `root_arg` |
| BY-REGISTRY-STALE | GAP | path_registry survives index **failure** path | tool_index_repo clears only after success; pass 7 CL-INDEX-FAIL-REGISTRIES |
| BY-QUERY-REGEX-CPU | residual | Regex length-capped only | `search/passes/regex.rs` |
| DF-KEY-REDACT | CONSISTENT | API key redacted in Debug; sent only as Bearer | embedder.rs |

No **new CONTRADICTION** beyond C1/C2. No product R-* filed (audit mode).

---

## Validation invalidation checklist

| Earlier validation | Later transform | Still holds? |
|--------------------|-----------------|--------------|
| MCP sandbox root | tool body uses canonical PathBuf | yes |
| parse_node_id relative | root.join + re-canon | yes |
| embed host allowlist | Ollama path join | yes (re-check) |
| embed host allowlist | HTTP redirect | yes (`redirects(0)`) |
| query length | lexicon expansions in response only | yes (not re-executed as queries) |
| CM no root check | `$ref` injects path string | N/A — never validated |
| index path open | durability pragma | orthogonal; both apply |

---

## Pass-7 residuals closed on this axis?

| Residual (pass 7 §3) | Pass-8 outcome |
|----------------------|----------------|
| Untrusted query/root/node/index/embed/file | Traced DF-* |
| Validation order wire→sandbox→parse→open | Mapped in validation-normalization-map |
| path_registry after failed index | BY-REGISTRY-STALE / GAP confirmed |
| Embed body + key + no-redirect | DF-EMBED CONSISTENT |
| Miss envelope agent-control | ownership ledger (soft) |
| Index write + durability | DF-INDEX + INV-DURABILITY-FC |
| Benchmark honesty | No numbers invented |

---

## Open residuals retained

C1, C2, GAP-CM-ROOT, GAP-CM-INV-TEST, GAP-RO-HOST, GAP-XOR-RUNTIME, GAP-INDEX-PATH-DOC, GAP-EMBED-REDIR-IT, B-ZS-ENGINES, B-DIRTY-FREEZE, B-SECURITY-NAPI-DOC, CL-* cleanup gaps (pass 7), BY-QUERY-REGEX-CPU.

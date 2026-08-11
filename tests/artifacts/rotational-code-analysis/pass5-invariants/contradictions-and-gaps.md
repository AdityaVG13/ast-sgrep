# Pass 5 — Contradictions, gaps, requirement conflicts

## Top contradictions

### C1 — INV-CASCADE-STRUCT-EMPTY (docs vs code/tests)

| Side | Claim |
|------|-------|
| **docs/cascade-query-planner.md** | Structural empty → cascade **stops**; "no hybrid hits when either the lexical or structural stage has no survivors." |
| **search_hybrid + cascade_planner test** | Structural empty → **keep lexical** as working set; semantic may still run (ht1h.3 / parity). |

**Impact:** Public retrieval contract is wrong for natural-language / plain-content queries. Callers reading the doc will mis-predict stop behavior.

**Resolution options (not applied — audit only):**
1. Fix docs to match ht1h.3 (lexical fallback + semantic on lexical files).
2. Or change code to hard-stop on empty structural (would break test and plain-content findability).

**Pass-6 probe:** happy-path hybrid on content-only files without structural symbols must show which side is product truth.

### C2 — INV-SURFACE-ROOT-PARITY (MCP jail vs Code Mode free root)

| Side | Claim |
|------|-------|
| **MCP / env-trust** | Tool roots sandboxed under workspace. |
| **Code Mode docs + `root_arg`** | No OS jail; host/process authority. |

**Impact:** Hosts that expose model-controlled `root` on Code Mode inherit full FS of the OS user for index/search. MCP users get jail. Dual-loading (XOR gap) compounds confusion.

**Not a code bug inside either surface alone** — each matches its local docs. Contradiction is **cross-surface product expectation**.

---

## Top gaps (falsifiable residual)

| Gap ID | Linked INV | Missing | Severity | Next pass |
|--------|------------|---------|----------|-----------|
| GAP-CM-ROOT | INV-CM-ROOT-FREE / SURFACE-ROOT | Negative test + optional jail | high (host-dependent) | pass 6 path; harden later |
| GAP-CM-INV-TEST | INV-CM-SEARCHER-INV | Codemode parity test + generation restore | medium | pass 6/9 |
| GAP-RO-HOST | INV-RO-CATALOG | Host approval for `index_repo` | medium | pass 6 Pi sticky path |
| GAP-XOR-RUNTIME | INV-XOR-CM-MCP | Runtime mutual exclusion | medium (policy) | pass 10 boundary |
| GAP-INDEX-PATH-DOC | INV-INDEX-PATH-PRIV | Document absolute path as privileged | medium | docs (not this audit) |
| GAP-EMBED-REDIR-IT | INV-EMBED-ALLOW | Live redirect integration test (policy in code) | low | pass 8/20 |

---

## Conflicts recorded (not silently chosen)

1. Cascade empty-structural: **code/tests win as runtime truth**; docs **stale** until fixed.
2. Root isolation: **no unified product invariant** — surfaces intentionally differ; parity claim rejected.
3. XOR: **docs-only** — runtime co-load is allowed by code.
4. `read_only`: **catalog truth** is metadata; **execution truth** is unrestricted `session.call`.

---

## Inherited open (not re-litigated)

- B-DIRTY-FREEZE, B-ZS-ENGINES, B-NO-COVERAGE-GATE, B-NO-MUTATION-GATE, B-SECURITY-NAPI-DOC
- GAP-WATCH-ADV (→ pass 9)
- U-LSP-MULTIROOT, U-SERVE-AUTH, U-BATCH-ROOT, U-SUPERVISOR-FORGE

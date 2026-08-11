# Residual handoff → Pass 11 (independent verification + beads)

## Mission for pass 11

Axes: independent observer, dual-evidence, loop-27 style promotion. **No seal.** Audit may remain book-only unless dual-evidence lands and product intent is clear.

## Must re-verify (do not trust pass 10 narrative alone)

| ID | Claim to re-prove | Suggested independent method |
|----|-------------------|------------------------------|
| C2 × shared index | Free CM `root` + fixed `index_path` can index foreign tree into pinned DB and prune workspace paths | Temp dirs fixture: index root A, CM index_repo root B with same index_path, assert file set / meta root |
| CL-MID-SIDECAR-CACHE | Sidecar Err after bulk commit leaves MCP Searcher un-invalidated | Inject/fail rebuild path or unit around invalidate placement vs `index_all` Result |
| GAP-WATCH-XPROC | External writer; MCP warm Searcher generation unchanged | Two processes or simulated external Indexer.update_paths while MCP cache held |
| FastUnsafe | write_pragma OFF only when named; steady NORMAL | Existing `store_pragmas` / sqlite tests — reaffirm; check doctor silence |
| Embed SSRF | allowlist + redirects(0) | Existing embed unit tests only — do not invent new CVE |

## Bead promotion gate (pass 11)

File `br` issues **only** when:

1. Severity high/critical **and**
2. Dual evidence (source locus + test or minimal repro) **and**
3. Not pure BY-DESIGN intentional policy without product decision

| Candidate | Likely product action | Promote? |
|-----------|----------------------|----------|
| Invalidate Searcher + registries on index Err (incl. mid-sidecar) | fix | **yes** if dual-evidence |
| path_registry clear on any index attempt end | fix | yes if bundled |
| xproc index lease / watch notify | design ASK | ask before bead |
| CM root sandbox parity with MCP | design ASK (breaks multi-root hosts) | ask |
| Doctor warn FastUnsafe | small fix | optional P3 |
| Structured MCP error codes | enhancement | optional |
| C1 cascade docs | docs | optional |

## Explicit non-goals for pass 11

- No invent CVEs or benchmark numbers
- No product edits unless user re-scopes off audit-only
- No commit/push without authority
- No hand-edit `.beads/issues.jsonl`

## Ledger IDs still open (carry)

C1, C2, GAP-CM-ROOT, GAP-CM-INV-TEST, GAP-RO-HOST, GAP-XOR-RUNTIME, GAP-INDEX-PATH-DOC, GAP-WATCH-XPROC, CL-MID-SIDECAR-CACHE, CL-PINNED-REINDEX, CL-CM-POISON-INV, CL-INDEX-FAIL-REGISTRIES, RW-NESTED-UNFENCED, B-ZS-ENGINES, B-DIRTY-FREEZE, B-SECURITY-NAPI-DOC, FastUnsafe-ops, ESC-3.

# R-CM-ROOT-POLICY

| Field | Value |
|-------|-------|
| Residual ID | **R-CM-ROOT-POLICY** |
| Aggregates | C2, BY-CM-ROOT, GAP-CM-ROOT, GAP-CM-INV-TEST, B-SECURITY-NAPI-DOC, INV-CM-ROOT-FREE |
| Severity | high (durable corpus rewrite if host co-locates free CM root with pinned index) |
| Status | **DESIGN ASK** — dual-evidence CONFIRMED asymmetry; product intent open |
| Pass | 11 independent verification |
| Tracker | markdown only (open beads ≥50; no `br create`) |

## Problem

MCP jails every tool `root` under the configured workspace (`sandbox_root` / `starts_with`). Code Mode (and NAPI inheriting Session) accepts any `args.root` string via `root_arg` with **no** under-workspace check. Session and MCP both honor `ASGREP_INDEX_PATH` / config `index_path`.

Composition risk: model or plan `$ref` supplies `root=/tmp/evil` while `index_path` points at the project DB → Indexer walks evil tree, stores **relative** paths from evil root, `set_meta("root", …)` overwrites, and `prune_missing_files` removes project paths absent from the evil walk. Concurrent search on the same Session sees mixed or emptied corpus. MCP would refuse the same root.

This is a **policy contradiction** between surfaces, not a remote CVE. Threat model remains local OS user / installing agent.

## Evidence (pass 11)

1. **Source MCP jail:** `crates/ast-sgrep-mcp/src/lib.rs` `sandbox_root` ~L547–573; `resolve_root` routes all tool roots.
2. **Source CM free:** `crates/ast-sgrep-codemode/src/session.rs` `root_arg` ~L105–111; `index_repo` ~L248–266 binds `root_arg` + `config.index_path`.
3. **Indexer composition:** `crates/ast-sgrep-core/src/index.rs` `set_meta("root")` on open; `collect_index_candidates` strip_prefix; `prune_missing_files`.
4. **Test (MCP):** `tool_roots_are_sandboxed_under_configured_workspace` **PASS** (pass 11).
5. **Test (CM foreign root):** **ABSENT**.
6. Full writeup: `tests/artifacts/rotational-code-analysis/pass11-independent/dual-evidence-high-findings.md` §H1.

## Product decision options (ASK before implement)

| Option | Effect |
|--------|--------|
| A. Jail CM/NAPI root like MCP | Parity; may break intentional multi-root hosts |
| B. Document host duty only | Keep free root; security docs + NAPI note; optional deny-list env |
| C. Soft warn when `root` not under config root and `index_path` pinned | Observability without hard break |
| D. Separate index_path per root (refuse shared pin when root overrides) | Prevents prune cross-contamination |

## Acceptance (when product chooses A or D)

- [ ] Written product decision recorded in packet / bead close reason
- [ ] If jail: CM/NAPI fail-closed with message compatible with MCP (`escapes configured workspace` or shared helper)
- [ ] Integration test: foreign `root` + shared `index_path` does **not** rewrite project DB (or is refused)
- [ ] Catalog/docs for free `root` updated; NAPI = same Session contract
- [ ] If B only: docs in `docs/` + codemode catalog host-duty; adversarial test marked N/A with link

## Non-goals

- Inventing multi-tenant auth
- Changing MCP sandbox (already CONSISTENT)
- Full workspace test suite

## Verify (sketch)

```bash
# After product decision A:
cargo test -p ast-sgrep-codemode -- foreign_root  # or named test
# Expect fail-closed or isolated index — not project DB prune
rg -n "fn root_arg|sandbox_root|starts_with" crates/ast-sgrep-codemode/src/session.rs
```

## Handoff

Pass 12: residual remains **PENDING design** unless product decides. Not a ZERO-CHANGE seal blocker if documented as intentional INV-CM-ROOT-FREE with host duty — but must stay on residual ledger until explicit Accept of B.

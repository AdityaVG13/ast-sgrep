# Pass 4 RESULT — Entry points trust/privilege map

| Field | Value |
|-------|-------|
| Loop | 4 / entry-points-trust-and-privilege-map |
| Status | **COMPLETE** |
| Mode | audit (no product edits under crates/ or packages/) |
| Freeze | `fb932aac852f5496c0a7035cc5a0b508e05111cb` (retained; HEAD may be ahead with books) |
| Axes | boundary→entrypoint · attack-surface · adversary · source+config |
| Axes vs pass 3 | all four changed (system→component/graph/architect → this set) |
| Braid | **Continue** → pass 5 contracts & invariants |

## Deliverables

| Artifact | Path |
|----------|------|
| Entry-point catalog (json) | `iterations/04-entrypoints/entry-point-catalog.json` |
| Entry-point catalog (md) | `iterations/04-entrypoints/entry-point-catalog.md` |
| Trust/privilege map | `iterations/04-entrypoints/trust-privilege-map.md` |
| Policy-enforcement map | `iterations/04-entrypoints/policy-enforcement-map.md` |
| Gaps / UNKNOWN | `iterations/04-entrypoints/gaps.md` |
| Slim mirror | `tests/artifacts/rotational-code-analysis/pass4-entrypoints/` |

## Headline findings (observations)

1. **Local OS-user trust model** across all production surfaces -- no network authn/tenant.
2. **MCP enforces workspace jail** (`sandbox_root` + `code_read` TOCTOU); **Code Mode / NAPI does not** on `root` arg (`session.rs` `root_arg`).
3. **Highest sinks:** Indexer mutations, arbitrary `ASGREP_INDEX_PATH`, Pi `asgrep_edit` source writes, embed egress, optional ast-grep exec.
4. **State-changing coverage:** MCP index + Pi edit well tested; codemode root-escape **untested**; watch adversarial **sparse**; agent-plugin packaging **UNKNOWN**.
5. **Code Mode XOR MCP** is documentation/skill only -- not a runtime mutual exclusion.
6. Pass 3 BND-* all linked to concrete entry IDs.

## Gate check

> Every state-changing or externally reachable entry point is classified; uncertain reachability is recorded as UNKNOWN.

**Met:** process bins, MCP tools, LSP methods/commands, NAPI exports, Pi tools/hooks/commands, env plane, fuzz/dev, supervisor. UNKNOWNs listed in `gaps.md`.

## Evidence commands

```
rg '\[\[bin\]\]|Commands::|tools/list|sandbox_root|root_arg|#\[napi' crates packages
rg -n 'sandbox_root|uri_to_rel_path|planEdit|embed_url_is_allowed' crates packages/pi docs
# catalog: crates/ast-sgrep-codemode/src/catalog.rs
# CLI: crates/ast-sgrep-cli/src/cli_args.rs
# MCP: crates/ast-sgrep-mcp/src/lib.rs
# LSP: crates/ast-sgrep-lsp/src/server.rs backend.rs
# NAPI: crates/ast-sgrep-codemode-napi/src/lib.rs
# Pi: packages/pi/extension/src/index.ts edit.ts
```

## Counts

- Catalog entries (process + major families): **22** EP-* IDs
- MCP tools: **7**
- Codemode tools: **12**
- Pi tools: **5** + **4** commands + **1** hook
- LSP executeCommands: **5**
- NAPI exports: **6**
- State-changing primaries: **8** clusters

## Residuals → pass 5

- Codemode vs MCP root policy contract
- Index path resolution invariant
- Searcher invalidation after every mutator
- `read_only` catalog semantics for hosts
- XOR surface non-enforcement
- Env allowlist invariants (partially evidenced)

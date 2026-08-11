# Pass 4 — Trust / privilege boundary map

| Field | Value |
|-------|-------|
| Loop | 4 / entry-points-trust-and-privilege-map |
| Freeze retained | `fb932aac852f5496c0a7035cc5a0b508e05111cb` |
| Mapped at | 2026-08-11T01:59:47Z |
| Axes | boundary→entrypoint · attack-surface · adversary · source+config |
| Mode | audit (no product edits) |
| Evidence path | native rg/read (zerostack fszero engines unavailable — B-ZS-ENGINES) |

## Trust model (baseline)

**Local filesystem / installing OS user.** Surfaces do not implement multi-user authn, tokens, or network ACLs. Any code that can spawn or call into a surface runs with that process's UID privileges.

| Layer | Trust assumption | Failure mode if violated |
|-------|------------------|--------------------------|
| OS user | Caller is the project owner or equally privileged agent | Full index + source read; index write; optional file edit (Pi) |
| Workspace root | `ASGREP_ROOT` / CLI `--root` / LSP initialize root bounds intended project | Path escape → read/index foreign trees |
| Index path | Default `.asgrep` under root; `ASGREP_INDEX_PATH` is explicit override | Arbitrary SQLite path write/read as user |
| Env | Ambient config; boolish fail-closed for unknown durability | Privilege via embed URL, external binary, durability |
| Agent model | Prompt/tool args untrusted; host must sandbox model | Model-driven `root` / `index_repo` / `asgrep_edit` |

Documented explicitly: `docs/env-trust.md`, agent-plugin skill ("trusted code with the installing OS user's full system access — not an OS jail").

## Entry-point table

| ID | Name | Surface | Trust | Authn/Authz | Side effects | Highest sinks |
|----|------|---------|-------|-------------|--------------|---------------|
| EP-CLI-ASGREP | `asgrep` / `ast-sgrep` | CLI bins | OS user | none / full FS via root+index_path | R/W index, search, watch, serve, embed | Indexer, SQLite, embed HTTP, optional ast-grep exec |
| EP-CLI-SUB-INDEX | `index`/`reindex` | CLI | OS user | none | **Write** index gens | `index_all` / `reindex_all` |
| EP-CLI-SUB-SEARCH | search family | CLI | OS user | none | Read + optional embed | Searcher, embed |
| EP-CLI-SUB-WATCH | `watch` | CLI long-run | OS user | none | Continuous index write | `update_paths`, notify |
| EP-CLI-SUPERVISOR | worker supervisor | process | OS user | nonce+marker+ppid | spawn/kill child | process control |
| EP-CLI-CODEMODE-BATCH | `codemode-batch` | CLI JSON | OS user | none on root | multi-tool incl index | Session tools |
| EP-CLI-CODEMODE-SERVE | `codemode-serve` | CLI NDJSON | OS user | stdio trust | sticky tools | Session tools |
| EP-MCP-SERVER | `asgrep-mcp` | MCP stdio | OS user + **workspace sandbox** | `sandbox_root` under `ASGREP_ROOT` | search/index/code_read | `index_repo`, `code_read` |
| EP-LSP-SERVER | `asgrep-lsp` | LSP stdio | editor workspace | URI must map under root | index ensure, symbol nav | `ensure_index`, reindex cmd |
| EP-VSCODE | vscode ext | editor host | editor trust | host | spawn LSP | child spawn |
| EP-NAPI | codemode-napi | Node FFI | same Node process | **no** MCP sandbox on `root` | Session.call / batch | index_repo, Searcher |
| EP-CODEMODE-SESSION | tool catalog | lib | inherits caller | **`root_arg` unsandboxed** | per-tool | index_repo (write) |
| EP-PI-EXTENSION | pi-ast-sgrep | Pi tools | Pi user | edit under root; device refuse | Code Mode + edit + index | `asgrep_edit`, native index |
| EP-PI-LAUNCHER | npm bins | spawn | OS user | binaryPath integrity | exec CLI | platform bin |
| EP-AGENT-PLUGIN | agent-plugin | MCP pack | host + OS user | host config | spawn mcp | BND-MCP |
| EP-ENV | `ASGREP_*` | env plane | ambient | process env | configures all sinks | INDEX_PATH, embed, AST_GREP |
| EP-FUZZ | fuzz targets | dev | local/CI | n/a | crash-only | n/a |

## Highest-privilege sinks (ranked)

1. **`Indexer::{index_all,reindex_all,update_paths}`** — durable SQLite mutations; reachable from CLI index/reindex/watch, MCP `index_repo`, codemode/NAPI/Pi `index_repo`, LSP `ensure_index` / `asgrep.reindex`.
2. **`ASGREP_INDEX_PATH` / `--index-path`** — redirect DB location **anywhere the user can write** (no under-root constraint in `try_index_db_path`).
3. **Pi `asgrep_edit` / `edit.ts` `writeFile`** — direct project source mutation (only product path that edits source files, not just index). Sandboxed under project root + device-path denylist; tests present.
4. **MCP `code_read`** — arbitrary file content under sandboxed root (info disclosure within workspace); TOCTOU-hardened open.
5. **Embed HTTP (`ASGREP_EMBED_*` / Ollama)** — network egress of query/chunk text; host allowlist + no redirects; SSRF-class residual mitigated hop-final.
6. **Optional external `ASGREP_AST_GREP`** — process exec only when dual env set (bench path); PATH names refused.
7. **Supervisor spawn/kill** — process privilege, not data plane.
8. **`Durability::FastUnsafe`** — power-loss corruption risk to index (opt-in by name).

## Privilege by surface (adversary view)

```
                    untrusted model / argv / JSON
                              │
        ┌─────────────────────┼─────────────────────┐
        ▼                     ▼                     ▼
   CLI (full FS)        MCP (ASGREP_ROOT)      LSP (workspace URI)
        │                     │                     │
        │              sandbox_root ✓          uri_to_rel_path ✓
        ▼                     ▼                     ▼
   CodemodeSession      McpServer tools       LspBackend
   root_arg ✗ sandbox         │                     │
        │                     │                     │
        └──────────► ast-sgrep-core Indexer/Searcher/store
                              │
              ┌───────────────┼───────────────┐
              ▼               ▼               ▼
           SQLite          embed HTTP      mmap IVF (read)
```

### Confirmed asymmetry

| Control | MCP | Code Mode / NAPI / CLI serve | CLI direct | Pi edit |
|---------|-----|------------------------------|------------|---------|
| Workspace root jail on `root` | **yes** (`sandbox_root`) | **no** (`root_arg` raw PathBuf) | n/a (root is the authority) | **yes** (project root) |
| Deny unknown wire fields | yes | schema `additionalProperties: false` on catalog | clap | zod-ish checks in edit |
| Index write single-flight | yes (lock+deadline) | session cache invalidate only | no cross-process lock | via backend |
| Source file write | no | no | no | **yes** (`asgrep_edit`) |

**Finding observation (audit, not product fix):** Code Mode / NAPI callers that supply `args.root` can target any path the OS user can read/index -- MCP explicitly refuses escape. If a host trusts model-supplied `root` into NAPI `Session.call`, privilege equals full-user indexer (R-CODEMODE-ROOT-UNSANDBOXED). Residual for pass 5/8/10.

## Policy-enforcement map (summary)

| Policy | Enforcement locus | Alternate / bypass path |
|--------|-------------------|-------------------------|
| Workspace containment | MCP `sandbox_root`; LSP `uri_to_rel_path`; Pi `planEdit`/`inside` | CLI any `--root`; codemode `root_arg`; `ASGREP_INDEX_PATH` absolute |
| Embed SSRF | `embed_url_is_allowed` + redirects(0) | Operator expands `ASGREP_EMBED_URL_ALLOWLIST` |
| External binary | dual env + absolute path + version probe | default off |
| Durability weak mode | clap/env parse must name `fast-unsafe` | env injection in same user |
| Output size | clap parsers + MCP limits + batch byte cap | UNKNOWN hostile huge trees still cost CPU/disk |
| Worker authenticity | supervisor nonce/marker | same-user env forge if parent checks fail (partial Unix) |
| Code Mode XOR MCP | docs/skill policy only | **host config** -- not enforced in process |

## State-changing entries: owner / contract / test

| Entry | Owner | Contract | Test evidence | Gap? |
|-------|-------|----------|---------------|------|
| CLI index/reindex | CLI | capabilities JSON | cli/core index tests | low |
| CLI watch | CLI | help/about | sparse adversarial | **GAP-WATCH-ADV** medium residual |
| MCP index_repo | MCP | docs/mcp.md | protocol + invalidate test | low |
| Codemode index_repo | Code Mode | catalog schema `read_only:false` | batch parallelization test | **GAP-CM-ROOT** no root-escape test |
| NAPI Session.call | Pi native | release-contract | native-inprocess | inherits GAP-CM-ROOT |
| LSP reindex/ensure | Editor | initialize_result | backend tests | medium -- executeCommand trust = editor |
| Pi asgrep_index | Pi | release-contract | tools.test + e2e scripts | low |
| Pi asgrep_edit | Pi | edit schema + skill | tools.test path/device | low |
| Codemode-batch/serve | Pi/CLI | machine envelope | batch + fuzz serve | medium IPC trust |
| agent-plugin pack | MCP pack | mcp.json + skill | packaging **UNKNOWN** | **GAP-PLUGIN-TEST** |

## Boundary IDs (from pass 3, entry-linked)

| BND | Entry IDs |
|-----|-----------|
| BND-CLI-JS | EP-CLI-*, EP-PI-LAUNCHER, Pi CLI fallback |
| BND-MCP | EP-MCP-*, EP-AGENT-PLUGIN |
| BND-LSP | EP-LSP-*, EP-VSCODE |
| BND-NAPI | EP-NAPI, EP-PI-EXTENSION (native path) |
| BND-TREE-SITTER | via index/search (not a process entry) |
| BND-MMAP | via semantic path (read-only unsafe island) |

## UNKNOWN / residual

| ID | Item |
|----|------|
| U-CM-ROOT | Codemode/NAPI `root` escape vs session config -- no fail-closed; intent may be multi-root host -- needs contract |
| U-INDEX-PATH | Absolute `ASGREP_INDEX_PATH` outside project -- by design? document as privileged |
| U-WATCH-RACE | Watcher path normalization vs symlink races -- pass 9 |
| U-VSCODE-TEST | Extension automated coverage sparse |
| U-PLUGIN-CI | agent-plugin packaging gate not inventory-verified this pass |
| U-FUZZ-PROD | fuzz excluded from workspace -- ok for prod entry catalog |
| B-ZS-ENGINES | still open |
| B-DIRTY-FREEZE | freeze rev retained; HEAD may differ with audit books |
| B-SECURITY-NAPI-DOC | pass 3 doc tension retained |

## Residuals → pass 5 (contracts & invariants)

1. Falsify **workspace containment** for codemode vs MCP (same tool name `index_repo`, different root policy).
2. Invariants for **index path resolution** (`try_index_db_path` precedence).
3. **Warm Searcher invalidation** after all mutation paths (MCP tested; LSP/codemode/watch parity).
4. **read_only catalog flag** honored by hosts (Pi/NAPI must not treat index_repo as auto-approved if host claims PTC read-only).
5. **Code Mode XOR MCP** is social/docs contract only -- record as non-enforced invariant.
6. Env plane boolish + allowlist invariants (already partially tested in embed).

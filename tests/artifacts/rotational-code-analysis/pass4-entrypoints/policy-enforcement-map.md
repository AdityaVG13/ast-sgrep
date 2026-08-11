# Pass 4 — Policy enforcement map

Maps intended policy → enforcement site → alternate path. Adversary observer.

## P1 Workspace / path containment

| Surface | Enforcement | File:symbol | Alternate path |
|---------|-------------|-------------|----------------|
| MCP tool `root` | canonicalize + `starts_with(ASGREP_ROOT)` | `ast-sgrep-mcp/src/lib.rs` `sandbox_root` | none if root set; mis-set `ASGREP_ROOT=/` weakens |
| MCP `code_read` | join+canonicalize under root; same-file recheck | `read_node` | symlink games if root wide |
| LSP URIs | `uri_to_rel_path` bail outside root | `lsp/support.rs` | multi-root folders only first? (check initialize resolve_root) |
| Pi edit | resolve under projectRoot; device path refuse | `packages/pi/extension/src/edit.ts` | host cwd wrong → wrong root |
| Codemode `root` arg | **none** beyond OS | `session.rs` `root_arg` | any path |
| CLI `--root` | is the authority | `cli_args` / `resolve_root_index` | intentional full access |
| Index path env | absolute path accepted | `store/mod.rs` `try_index_db_path` | write DB outside project |

## P2 Network / SSRF (embed)

| Control | Site | Notes |
|---------|------|-------|
| Host allowlist | `embed/embedder.rs` `embed_url_is_allowed` | openai/azure/loopback + env list |
| HTTP non-loopback | requires `ASGREP_EMBED_ALLOW_INSECURE_HTTP` | fail closed |
| Redirects | agent `redirects(0)` | allowlist is final hop |
| API key | env `ASGREP_EMBED_API_KEY` | secret in env plane |

## P3 External process

| Control | Site | Notes |
|---------|------|-------|
| ast-grep exec off by default | `core/pattern.rs` | needs ALLOW + absolute AST_GREP |
| Pi binary override | launcher/runtime | must be existing executable |
| Supervisor child | `supervisor.rs` | nonce+marker; spawn current_exe only |

## P4 Resource / DoS budgets

| Control | Site |
|---------|------|
| Output limit clamp | CLI clap + core `clamp_output_limit` |
| Excerpt/snippet token bounds | cli_args parsers |
| MCP index deadline + single-flight | `tool_index_repo` + `index_lock` |
| Codemode max_calls | Session default 64 |
| Batch payload cap | `MAX_BATCH_REQUEST_BYTES` |
| Edit max bytes/line | `edit.ts` constants |

## P5 Index integrity / durability

| Control | Site |
|---------|------|
| Durability named modes | `Durability::parse` fail unknown |
| `fast-unsafe` opt-in only | cli help + parse |
| Searcher invalidate on MCP index | `index_repo_invalidates_searcher_*` test |
| Generation pointer | store active manifest |

## P6 Agent surface policy (non-code)

| Control | Enforcement type | Gap |
|---------|------------------|-----|
| Code Mode XOR MCP | docs + skills only | host can load both |
| Prefer asgrep over one-shots | skill prose | model non-compliance |
| No telemetry claim | skill/README | not verified this pass |

## Validation pipeline (wire → trusted)

```
MCP:  JSON-RPC params → #[serde(deny_unknown_fields)] wire structs
      → parse_* → resolve_root/sandbox_root → trusted *Args → handlers

Codemode: tool name parse → call_tool match → session method
      → root_arg (PathBuf::from string) → Indexer/Searcher

CLI:  argv → clap → Commands → handlers → open_indexer/searcher

LSP:  framed JSON → method table → typed params → backend
      → uri_to_rel_path → store/search

Pi edit: tool params → repairEditPath → planEdit (root check)
      → applyEdit writeFile
```

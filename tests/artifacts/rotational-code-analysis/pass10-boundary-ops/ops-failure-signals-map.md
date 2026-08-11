# Pass 10 — Ops failure signals map

What production / operator-visible surfaces **reveal** (and hide) when boundary and multi-writer failures occur.

| Field | Value |
|-------|-------|
| Loop | 10 |
| Axes | observer:**operator** · evidence:**source+config** |
| Mode | audit |

## Signal channels inventory

| Channel | Surface | Format | Audience | Always on? |
|---------|---------|--------|----------|------------|
| MCP tool result | `asgrep-mcp` stdio | JSON-RPC result; `isError:true` + `content[].text = e.to_string()` | Agent host | yes |
| MCP parse/protocol | same | JSON-RPC `error` codes (−32700 parse, method errors) | Host | yes |
| CLI machine envelope | `asgrep --json` / doctor / status | `ok`, `exit_code`, structured fields | CI/robots | when `--json` |
| CLI human stderr | watch, index, lexicon skip | `eprintln!` lines | TTY operator | yes for those cmds |
| Index status | `status` tool / CLI | counts, `index_path`, `durability`, embed cache, `semantic_ivf_present` | Agent/ops | on demand |
| Doctor triage | `asgrep doctor --robot-triage` | `healthy`, `issues[]`, `suggested_commands`; **exit 2** if unhealthy | CI/agent | on demand |
| Perf profile | `ASGREP_PERF_PROFILE=1` | JSONL spans | perf eng | opt-in only |
| Capabilities catalog | `capabilities --json` | env allowlist incl. durability, embed allowlist | Agent bootstrap | on demand |

No Prometheus/OTel metrics, no structured severity codes on MCP tool errors, no multi-writer heartbeat.

---

## Failure → signal matrix

| Failure class | Durable effect | MCP agent sees | CLI operator sees | Doctor / status | Hidden / false-negative risk |
|---------------|----------------|----------------|-------------------|-----------------|------------------------------|
| **Jail escape attempt (MCP)** | none | `isError` text: root escapes workspace | n/a | n/a | low — fail closed |
| **CM free root index (C2)** | foreign walk + possible prune of shared DB | **success** stats if walk ok | status `root` meta may flip; file_count shift | doctor may still `healthy` if DB opens | **high** — success is the attack |
| **Watch updates while MCP warm** | SQLite new data | **stale hits** (success) | watch stderr progress | status counts ≠ agent answers | **high** — no cross-process invalidate signal |
| **Mid-sidecar after commit** | SQLite committed; sidecar partial/missing | `isError:true` string; **no** "committed" flag | eprintln on some rebuild paths; lexicon skip only | may flag `semantic_ivf_missing` if chunks without IVF | **high** — error implies "no mutate"; cache not invalidated |
| **Index soft deadline after work** | committed + invalidated (if Ok path reached invalidate before ensure) | deadline exceeded error **after** invalidate on Ok path; if Err before invalidate, worse | none | none | medium — ESC-3 |
| **Pinned reindex crash mid-clear** | empty or partial live DB | open/search errors | integrity quarantine / open fail | `index_open` / `status_read` issues | medium — crash window |
| **FastUnsafe power loss** | possible corrupt | later open Err | corrupt rename `index.db.corrupt` | `index_open` | medium — no proactive FastUnsafe issue in doctor |
| **ASGREP_INDEX_PATH wrong/writable share** | shared multi-tenant clobber | confusing hits | path in status | shows path only | medium — no "shared path" warning |
| **Embed URL not allowlisted** | no request | tool/search error or embed disabled path | config error | not always | low — fail closed |
| **Embed allowlisted host 30x** | blocked at agent | redirect/HTTP error | same | n/a | low — redirects(0) |
| **path_registry stale after index Err** | none on disk | compact ids may mis-resolve; `code_read` re-jails | n/a | none | low–med soft |
| **Walk errors during index** | prune **suppressed** | stats with `walk_errors` if exposed | index warning unpruned | partial | low — intentional safety |
| **Unknown durability string** | refuse / default path | open uses default if env invalid (`from_env` → Balanced); CLI clap hard error | CLI usage error | n/a | CLI **CONSISTENT**; bare `from_env` silently defaults unknown → Balanced (not FastUnsafe) |
| **Poisoned MCP mutex** | cache cleared via `lock_or_recover` | may rebuild Searcher | n/a | none | low — fail closed rebuild |
| **CM poison searcher cache** | invalidate may no-op (pass 9) | possible stale | n/a | none | medium in-process |

---

## Doctor coverage vs gaps

`doctor_triage_json` (`cli/src/agent.rs`) issues kinds observed in source:

| Issue kind | Detects | Does **not** detect |
|------------|---------|---------------------|
| `missing_root` | bad root path | free-root CM misuse |
| `index_open` | open/integrity fail | mid-sidecar with openable DB |
| `status_read` | status query fail | multi-writer staleness |
| `semantic_ivf_missing` | chunks without IVF file | stale warm Searcher in another process |
| (healthy iff issues empty) | open+sidecar heuristic | FastUnsafe, dual process writers, CM root parity |

**Ops residual:** doctor is a single-process open/health probe, not a fleet consistency probe.

---

## MCP error semantics (operator/agent)

```
handle_tools_call:
  Ok(text)  → content text, isError:false  (even if text is JSON stats)
  Err(e)    → content text = e.to_string(), isError:true
```

No machine `code` field for: jail, deadline, busy, sidecar, durability. Hosts cannot route retries by class without string match.

Watch failures: `eprintln!("[asgrep] watch error: {e}")` only — never reaches MCP bus.

---

## Config surface (ops attack + footgun)

| Knob | Privilege effect | Signal when mis-set |
|------|------------------|---------------------|
| `ASGREP_ROOT` | MCP jail origin | startup canonicalize fail |
| `ASGREP_INDEX_PATH` | absolute DB anywhere; disables gen reindex | status.index_path only |
| `ASGREP_DURABILITY` | FastUnsafe power risk | status.durability; no doctor warn |
| `ASGREP_EMBED_URL_ALLOWLIST` | expands SSRF surface | fail closed if host still not listed for request URL |
| `ASGREP_EMBED_ALLOW_INSECURE_HTTP` | non-loopback http | none beyond request |
| `ASGREP_USE_CACHE` | cache under XDG; refuse /tmp | open error if no HOME |
| CM/NAPI `args.root` | free FS | **success** |

---

## Production reveal summary

| Question | Answer |
|----------|--------|
| Does agent learn index was durable when tool errors? | **No** (mid-sidecar, deadline) |
| Does agent learn external watch mutated index? | **No** |
| Does operator learn MCP is serving stale Searcher? | **No** automatic signal |
| Can CI gate on FastUnsafe? | Only by parsing status/env; doctor will not fail |
| Are embed SSRF attempts noisy? | Fail closed with error string; no audit log sink |
| Multi-writer deploy safe by default? | **No** — WAL yes, app cache no |

---

## Recommended ops signals (audit only — not implemented)

1. Structured MCP error codes: `INDEX_COMMITTED_SIDECAR_FAILED`, `DEADLINE_AFTER_MUTATE`, `ROOT_ESCAPE`.
2. `status` / doctor: `multi_writer_hint` if lock file absent; warn `durability=fast-unsafe`.
3. Optional index lock file under db dir for watch+MCP single-writer.
4. Invalidate-on-any-index-result (Ok or Err after mutation attempt).
5. Log line with `index_data_version` after every mutator for host correlation.

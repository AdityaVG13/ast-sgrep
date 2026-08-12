# Independent verification table (crossing-record)

Axes: skeptic · independent-reproduction · crossing-record. Originator passes 2–8 are hypotheses under attack.

| Residual | Claim (narrow) | Source (reconstructed) | Pin (fresh RCH) | Verdict | Confidence |
|----------|----------------|------------------------|-----------------|---------|------------|
| **R-INDEX-ERR-CACHE-SYNC** | Mid-sidecar `index_repo` Err clears warm Searcher (+ MCP maps) | MCP `tool_index_repo` captures result then `invalidate_after_index_attempt()` before `?` (`mcp/src/lib.rs` ~911–921); CM `index_repo` same (`session.rs` ~337–344) | `index_repo_invalidates_searcher_on_index_err` MCP+CM **ok** | **TRUE** | high |
| **R-CM-ROOT-POLICY** | Tool `root` jailed under session workspace (Option A) | `sandbox_root`: canonicalize + `starts_with(workspace)` (`session.rs` ~141–179); NAPI inherits Session | `foreign_root_is_rejected_under_session_workspace` **ok** | **TRUE** | high |
| **R-XPROC-MULTIWRITER** | External writer stamp bump drops warm MCP/CM Searcher | `bump_writer_generation` / `sync_writer_generation` poll in `searcher_for` (MCP ~622–640; CM ~108–123); Indexer `advertise_writer_generation` | core `writer_generation` 3 + MCP/CM `external_writer_generation_*` **ok** | **TRUE** (scope: polling peers) | high |
| **R-OPS-DOCS-FOOTGUNS** | Doctor emits `durability_fast_unsafe` when FastUnsafe active | `doctor_fast_unsafe_issue` → `doctor_triage_json` (`cli/src/agent.rs` ~188–237) | `doctor_surfaces_*` 3 **ok** | **TRUE** | high |
| **missing generation fail-closed** | Missing `generations/<active>/` refuses legacy/empty fallthrough | `try_index_db_path` Err when manifest present but candidate missing (`store/mod.rs` ~222–235) | `missing_active_generation_refuses_stale_legacy_fallthrough` **ok** | **TRUE** | high |
| **newer schema refuse** | `user_version > SCHEMA_VERSION` fails open | `init_schema` (`sqlite.rs` ~172–177) | `newer_than_binary_schema_refuses_open` **ok** | **TRUE** | high |
| **watch symlink refuse** | Existing symlink under root not indexed via `update_paths` | `normalize_watch_path` `symlink_metadata` → None (`index.rs` ~1148–1163) | `update_paths_refuses_symlink_escape_into_index` **ok** | **TRUE** | high |

## Disconfirming probes (none bit)

| Probe | Result |
|-------|--------|
| Invalidate only on Ok? | FALSE — both surfaces call invalidate before `result?` |
| CM bypass of sandbox via NAPI? | No separate root path; NAPI uses Session (rustdoc + shared `root_arg`) |
| Stamp bump without poll reopen? | MCP/CM `searcher_for` calls `sync_writer_generation` first |
| Missing gen fallthrough to flat `index.db`? | Err path before legacy branch |
| Future schema silent open (`>=`)? | Strict `>` refuse; equality probes tables |
| Watch follows symlink via `metadata`? | Uses `symlink_metadata`; test asserts no `leaked_secret` |

## Queued (not expanded this pass)

| ID | Dual-evidence this pass? | Disposition |
|----|--------------------------|-------------|
| R-EMBED-HTTP-TIMEOUT-BODY | Not elevated to high/critical correctness | **queue** |
| R-PATTERN-UNBOUNDED-READ | Availability/OOM, not wrong hits | **queue** |
| R-CM-SOFT-TIMEOUT-ORPHAN | Capacity bleed; Mutex serializes correctness | **queue** |
| R-PI-EDIT-SYMLINK-LEXICAL | Pi out of scope | **Refuse / queue** |

**Product edits this pass:** 0  
**Patches WRONG:** 0  
**Outcome:** ZERO-CHANGE

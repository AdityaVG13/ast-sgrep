# Golden provenance

Goldens live under `tests/golden/` (workspace) or crate-local fixture paths
passed to `assert_golden_at` / `assert_golden_json_at`.

| Field | Rule |
|---|---|
| Command | The test that froze the file (crate + test name). |
| Date | ISO date of the freeze. |
| Scrub | `Scrubber` preset: `none`, `standard`, `machine_contract`, `search_dump(root)`, `doctor`, `status`. |
| Notes | Why this freeze is stable. |

Update with `ASGREP_UPDATE_GOLDENS=1` only. Never `UPDATE_GOLDENS` or `INSTA_UPDATE`.
Compare is the default (env unset). Mismatches write `{golden}.actual` (gitignored).

## Existing crate-local freezes

These predate this helper and stay next to `machine_contracts`:

| File | Command | Scrub | Notes |
|---|---|---|---|
| `tests/cli/fixtures/capabilities.json` | `ast-sgrep-cli` `capabilities_and_version_match_goldens` | test assigns `version` → `<version>` then `assert_golden_json_at` | Machine capabilities envelope. |
| `tests/cli/fixtures/envelopes.json` | same test, `version` sub-object | ad-hoc | Still `assert_eq!` until a later child. |
| `tests/cli/fixtures/machine_shapes.json` | `index_reindex_status_and_doctor_have_stable_shapes`, `native_github_gitlab_search_shapes_are_stable` | key-set only | Shape keys, not a full dump. native/github/gitlab added nz7i.2. |

## nz7i.2 CLI / plugin freezes

| File | Command | Date | Scrub | Notes |
|---|---|---|---|---|
| `tests/cli/fixtures/search_agent_hits.json` | `ast-sgrep-cli` `search_hit_dumps_match_goldens_for_agent_capsule_and_compact` | 2026-08-13 | `search_dump(sample_root)` then `machine_contract` | `NO_COLOR=1 asgrep --json --no-embed --index-path <tmp> --limit 2 --format agent process_request <sample>` |
| `tests/cli/fixtures/search_agent_capsule_hits.json` | same | 2026-08-13 | same | `--format agent-capsule` |
| `tests/cli/fixtures/search_compact_hits.json` | same | 2026-08-13 | same | `--format compact`; scores kept |
| `tests/cli/fixtures/teaching_indxx.json` | `path_free_usage_teaching_messages_match_goldens` | 2026-08-13 | none | `asgrep --json indxx`; full usage envelope including did-you-mean |
| `tests/cli/fixtures/teaching_format_agnt.json` | same | 2026-08-13 | none | `asgrep --json --format agnt query .` |
| `tests/plugins/fixtures/capsule_sample.json` | `ast-sgrep-plugins` `capsule_compact_github_gitlab_full_dumps_match_goldens` | 2026-08-13 | none | `format_response_with(sample(), AgentCapsule, 0)`; synthetic `src/*.rs` |
| `tests/plugins/fixtures/compact_sample.json` | same | 2026-08-13 | none | `format_response_with(sample(), Compact, 0)` |
| `tests/plugins/fixtures/github_sample.json` | same | 2026-08-13 | none | `to_github_json(&sample())` |
| `tests/plugins/fixtures/gitlab_sample.json` | same | 2026-08-13 | none | `to_gitlab_json(&sample())` |

## nz7i.3 agent / protocol freezes

| File | Command | Date | Scrub | Notes |
|---|---|---|---|---|
| `tests/cli/fixtures/robot_guide.md` | `ast-sgrep-cli` `robot_docs_guide_body_matches_golden` | 2026-08-13 | `none` + `canonicalize_text` | `asgrep robot-docs` stdout; JSON `body` must match |
| `tests/mcp/fixtures/initialize.json` | `ast-sgrep-mcp` `initialize_and_tools_list_match_goldens` | 2026-08-13 | `machine_contract` (`serverInfo.version` → `<version>`) | Keep `protocolVersion` and `serverInfo.name` |
| `tests/mcp/fixtures/tools_list.json` | same | 2026-08-13 | none | Full `result.tools[]` including `inputSchema` |
| `tests/codemode/fixtures/tool_catalog.json` | `ast-sgrep-codemode` `catalog_and_host_adapters_match_goldens` | 2026-08-13 | none | All `ToolDef` values |
| `tests/codemode/fixtures/anthropic_tools.json` | same | 2026-08-13 | none | `anthropic_tools()` |
| `tests/codemode/fixtures/openai_tools.json` | same | 2026-08-13 | none | `openai_tools()` |
| `tests/codemode/fixtures/cloudflare_connector.json` | same | 2026-08-13 | none | `cloudflare_connector()` |

## nz7i.4 extraction dumps + chain expand

Full extraction dumps live under `tests/lang/fixtures/extract_dumps/` (not next to
source fixtures in `extract/`). Presence/forbid tuples stay in
`assert_language_conformance`; extra symbols and kind/name drift fail the dump
compare. Spans freeze because the extract fixtures are immutable.

| File | Command | Date | Scrub | Notes |
|---|---|---|---|---|
| `tests/lang/fixtures/extract_dumps/{lang}.json` (13 langs) | `ast-sgrep-lang` `all_languages_satisfy_shared_parse_extract_and_pattern_contract` | 2026-08-13 | none (`canonicalize_extraction` sort only) | Symbols `(name, kind, byte_start)`, imports `(module_path, line)`, calls `(caller, callee, line, byte_start)`, pattern nodes `(signature, line_start, excerpt)` |
| `tests/cli/fixtures/chain_expand_process_request.json` | `ast-sgrep-cli` `chain_expand_sample_dump_matches_golden` | 2026-08-13 | `search_dump(sample_root)` then `machine_contract` | `NO_COLOR=1 asgrep --json --no-embed --index-path <tmp> chain process_request <sample>`; nodes/edges via `canonicalize_chain_response`; scores kept |

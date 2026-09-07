# Negative ledgers (`6lmt`)

This file is the product fail-closed case table: CLI/MCP must error, not
return empty hits. Do not treat an ignored or not-run test as a product
success.

Cases that must **not** succeed as silent empty hits:

| Case | Expected |
|------|----------|
| Missing project root | exit 2 / operational error containing `does not exist` |
| Empty index (0 files) | exit 2 / `index is empty` |
| Doctor on missing root | `healthy:false`, `ok:false`, triage `missing_root` |
| MCP root outside workspace | tool `isError`, message `escapes configured workspace` |
| Stored HTTP embed backend (`cloud`/`ollama`) | `embed_query` Err naming removed HTTP provider; reindex required |
| Empty native package binary | `ASGREP_EXECUTABLE_EMPTY` even if checksum is empty-SHA256 |
| Regex worker panic | `StoreError` (not empty hit list) |

Harness stubs live under `tests/fixtures/ranking/` and `docs/validation/`.

Clause IDs **NL-xxx** (ghiw.2). Score TBD; do not claim MUST ≥ 0.95 from this
table. Compact omitting provenance is **DISC-compact-drops-provenance**, not a
fail-closed bug.

## MUST-not matrix

| ID | MUST-not | Expected | Test / gap |
|---|---|---|---|
| NL-001 | Missing project root must not return hits | exit 2, operational, message contains `does not exist` | `format_aliases_typos_and_root_failures_are_unambiguous` |
| NL-002 | Empty index must not return hits | exit 2, `index is empty` (search and chain) | same |
| NL-003 | Doctor on missing root must not look healthy | `healthy:false`, `ok:false`, `issues[0].kind=missing_root` | `agent_discovery_defaults_and_boolish_envs_are_round_trip_free` |
| NL-004 | Usage vs operational exits stay distinct | usage=1 / operational=2; never swap | `bounded_arguments_are_json_usage_errors`, `operational_failures_are_json_and_exit_two` |
| NL-005 | MCP root outside workspace | `isError`, `escapes configured workspace` | **gap** for this bead (MCP suite is `DISC-mcp-not-full-suite`; do not invent CLI-envelope coverage) |
| NL-006 | Embed URL to metadata IP | HTTP embed backends removed (2026-08-14). Query of stored `cloud`/`ollama` meta fails closed (`embed_query` reindex error). | `stored_http_backends_hard_error_on_query` |
| NL-007 | Empty native package binary | `ASGREP_EXECUTABLE_EMPTY` | **gap** here (packaging path; not machine_contracts) |
| NL-008 | Compact must not be treated as native JSON equality | Compact key set is `h/p/q/v/…`; no native `hits` array / excerpt blobs | `compact_omits_native_hit_array_and_excerpt_blobs` + `DISC-compact-drops-provenance` |
| NL-009 | Regex worker panic must not become empty hits | `StoreError` | **gap** in CLI machine suite (core store path) |
| NL-010 | Inert in-memory store must not resolve `:memory:` to a filesystem path on the destruction path (r15 finding 1 / 64c F1: the legacy migration unlinked the CWD-relative `semantic.ivf` on every foreign-root search) | `invalidate_semantic_ivf` no-ops for in-memory db targets (`is_in_memory_db`, incl. `file:...mode=memory` URI forms); migration still stamps the stand-in schema; `NotFound` stays benign for real indexes | `tests/core/store_inert_memory.rs` both tests (foreign-root search sentinel + direct `open_in_memory` seam); FIXED pass 65/65b — detection-kill and migration-skip mutants both die. Residual read-only CWD-resolution sites (predicate-bound, owner user-WIP files): `search/mod.rs:412` (`semantic_manifest` can report a bogus `semantic: sidecar_unreadable` degraded channel), `store/sqlite/queries.rs:118` (`semantic_ivf_present` stat can read true) — gate `semantic_ivf_path` on in-memory targets if degraded-channel/status honesty is promoted |

NL-008 is the new machine-visible negative: compact is a different shape, and
dropping provenance is intentional (`DISC-compact-drops-provenance`).

## Pass-77 note (r27 remediation, 2026-09-06 — additive)

The pass-75 §4 remediation-report claim that the schema-15 migration probe
verified "both paths" was **materially incomplete**: it verified ANSWERS
(search results), not ROW STATE, and the answer it saw was produced by the
native fallback, not by the migration's rebuild — the migration had left
the `body:`/`struct:` fast-path fingerprints armed, so an incremental
refresh skipped genuine re-extraction while the answer channel still
looked correct. Lesson (mechanism class: *a passing answer can mask an
unperformed rebuild*): migration/freshness verification must assert row
state through a seam that first arms EVERY fast-path fingerprint (mtime,
content-hash, `body:`, `struct:` — writes.rs:24-45), not answers through a
channel with its own fallback. Closed by F76c-1 (pass 77a): migration now
wipes all four skip layers; `index_schema_rekey_freshness` 3/3 asserts
genuine row rebuild; CLI state-machine probe re-verified by pass 77E
(v14 db → codemod loud exit 2 → writable index → `call:Foo::bar` rows
rebuilt → idempotent second open). Retry predicate: re-run the CLI
state-machine probe after ANY `INDEX_SCHEMA_VERSION` bump.

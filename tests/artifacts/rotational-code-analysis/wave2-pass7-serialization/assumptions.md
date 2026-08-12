# Assumptions — Wave 2 Pass 7

1. On-disk `PRAGMA user_version` is the authoritative index schema id; `ActiveManifest.schema_version` is advisory metadata written at activation.
2. Machine JSON `schema_version: "1.0.0"` is a distinct namespace from store `SCHEMA_VERSION` (i64) and compact envelope `v` (integer).
3. Additive machine JSON fields (e.g. `writer_generation` on status) are forward-compatible for lenient peers; Pi rejects only `schema_version` / `machine_schema_version` mismatches.
4. MCP unknown `protocolVersion` answers with current server revision (spec); supported revisions negotiate exact match.
5. Product edits authorized on PR #27 harden campaign; Pi leftover remains out of scope.
6. zerostack unavailable this pass.

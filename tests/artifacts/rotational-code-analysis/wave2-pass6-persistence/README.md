# README — Wave 2 Pass 6 (Loop 9 persistence)

Attack: after harden patches 2–5, do SQLite bulk tx vs sidecars vs writer_generation vs Searcher cache still represent one coherent reality under crash/partial commit?

**Outcome: PRODUCTIVE** — new high data-integrity bug: corrupt `active.json` → silent stale legacy corpus. Fail-closed in `try_index_db_path`.

Other Loop 9 sites re-checked as CONSISTENT (degraded-but-correct or already compensated); see RESULT.md.

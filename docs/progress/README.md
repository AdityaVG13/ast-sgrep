# Campaign negative ledgers

These files are **campaign rejection / deferral ledgers** (gauntlet WP3). They are
not the product fail-closed table.

| File | Pillar | Use |
|---|---|---|
| [perf-negative-results.md](perf-negative-results.md) | Performance | Measured-and-rejected (or Open pointer) perf ideas |
| [conformance-negative-results.md](conformance-negative-results.md) | Conformance | Refuted or deferred conformance hypotheses |
| [surface-deferrals.md](surface-deferrals.md) | Surface | Intentional exclusions / deltas with retry predicates |

Product fail-closed cases (missing root, empty index, SSRF, …) stay in
[`docs/validation/negative-ledgers.md`](../validation/negative-ledgers.md).

## Entry template

Every **Closed** entry needs:

| Field | Required |
|---|---|
| `date` | ISO 8601 |
| `candidate_name` | kebab-case, unique in this file |
| `target_workload` | bench / fixture / surface |
| `files_touched` | status string (see skill seed) |
| `correctness_proof` | or `not-measured` for Open pointers |
| `evidence_artifact_paths` | real paths; never invent numbers |
| `baseline_configuration` | host / SHA / profile, or `pointer-only` |
| `candidate_configuration` | delta vs baseline, or `pointer-only` |
| `measured_result` | numbers + `cv_pct`, or **omit** (Open only) |
| `retry_condition_predicate` | **one of forms 1–8** |
| `bead_id` | optional |

**Zero invented measurement closes.** First seeds are Open / pointer imports.
Closed stays empty until a real artifact path exists.

## Predicate forms (1–8)

1. Retry only if a profiler attributes a clearly-above-noise share to `<COUNTER>` on `<WORKLOAD_SHAPE>`.
2. Reconsider only inside the broader `<X>` redesign (track as `<beads_id>`).
3. Worth reconsidering when `<GATE>` crosses `<THRESHOLD>`.
4. Not worth retrying as a standalone patch.
5. Do not retry from a cold read; use comprehensive-bench attribution instead.
6. Retry condition not applicable -- the gain is structural, not numerical.
7. Retry only if this workload class exhibits measurable `<PROPERTY>` below `<THRESHOLD>`.
8. Blocked until `<ARCHITECTURAL_DEPENDENCY>` lands; track as `<beads_id>`.

Forbidden: later, TBD, maybe, eventually, we should revisit, tracked elsewhere,
if it seems important, when we have time.

## Pre-flight mine

See root `AGENTS.md` **Negative-Evidence Discipline**. Grep these three files,
mine failure terms, check recent commits. If `cass` is unavailable, record a
blocker Open row rather than skipping.

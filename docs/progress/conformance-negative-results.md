# Conformance negative results

Campaign ledger for conformance hypotheses that were tested and refuted, or
that must not be reported as Pass when Not-run.

Skill headers: gauntlet WP3 / K-3. Predicate forms: `docs/progress/README.md`.
Verdict rules: `docs/validation/conformance-verdicts.md`.

**Closed:** empty on seed. Do not invent bake-off identity.

## Closed

_(none -- no in-tree measurement close on this seed)_

## Open (pointer imports)

### `jell-external-differential` (Form-2)

- **target_workload:** asgrep vs ripgrep vs ast-grep CLI hit-ID bake-off
- **files_touched:** `no-source-patch-attempted`
- **evidence_artifact_paths:** `docs/validation/jell-deferral.md`, `DISC-no-jell-harness`
- **retry_condition_predicate:** Reconsider only inside the broader jell / external-differential harness redesign (track as `ast-sgrep-conformance-harness-program-ghiw`).
- **bead_id:** `ast-sgrep-conformance-harness-program-ghiw`

### `lexical-not-rg`

- **target_workload:** keyword / FTS result identity vs ripgrep
- **evidence_artifact_paths:** `DISC-lexical-not-rg`, `docs/validation/jell-deferral.md`
- **retry_condition_predicate:** Reconsider only inside the broader jell / external-differential harness redesign (track as `ast-sgrep-conformance-harness-program-ghiw.3`).
- **bead_id:** `ast-sgrep-conformance-harness-program-ghiw.3`

### `pattern-native-subset-not-ast-grep-cli`

- **target_workload:** `pattern:` vs ast-grep CLI
- **evidence_artifact_paths:** `docs/structural-patterns.md`, `DISC-pattern-native-subset`
- **retry_condition_predicate:** Reconsider only inside the broader pattern vs ast-grep differential (track as `ast-sgrep-conformance-harness-program-ghiw.3`).
- **bead_id:** `ast-sgrep-conformance-harness-program-ghiw.3`

### `ranking-soft-oracle`

- **target_workload:** `tests/fixtures/ranking/cases.json`
- **evidence_artifact_paths:** `tests/core/ranking_oracle.rs`, `DISC-ranking-soft-oracle`
- **retry_condition_predicate:** Worth reconsidering when a gold rank vector (not must_include bag) lands with provenance under `tests/golden/`.
- **bead_id:** `ast-sgrep-golden-artifacts-program-nz7i`

### `query-grammar-must-matrix-unfilled`

- **target_workload:** QUERY_GRAMMAR MUST/SHOULD clauses
- **evidence_artifact_paths:** `docs/QUERY_GRAMMAR.md`, `docs/validation/COVERAGE.md`
- **retry_condition_predicate:** Blocked until QUERY_GRAMMAR + machine envelope MUST matrix lands; track as `ast-sgrep-conformance-harness-program-ghiw.2`.
- **bead_id:** `ast-sgrep-conformance-harness-program-ghiw.2`

## Retired

_(none)_

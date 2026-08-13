# Surface deferrals

Campaign ledger for surfaces explicitly excluded, partial, or intentionally
divergent. WP5 consumes this file for FeatureUniverse honesty.

Skill headers: gauntlet WP3 / K-3. Predicate forms: `docs/progress/README.md`.
Product parity table: `docs/validation/surface-parity.md`.
DISC register: `docs/validation/DISCREPANCIES.md`.

**Closed:** empty on seed.

## Closed

_(none -- no invented "we shipped parity" closes)_

## Open (pointer imports)

### `mcp-no-auto-fusion`

- **target_workload:** MCP vs CLI hybrid
- **evidence_artifact_paths:** `docs/validation/surface-parity.md`, `DISC-mcp-not-full-suite`
- **retry_condition_predicate:** Reconsider only inside the broader MCP hybrid-fusion redesign (track as `ast-sgrep-gauntlet-remediation-program-1vhy.5`).
- **bead_id:** `ast-sgrep-gauntlet-remediation-program-1vhy.5`

### `mcp-no-doctor`

- **target_workload:** MCP doctor/triage
- **evidence_artifact_paths:** `docs/validation/surface-parity.md` (doctor row `--`)
- **retry_condition_predicate:** Blocked until a product decision to expose doctor over MCP lands; track as a WP5 FeatureUniverse cell, not a silent CLI clone.
- **bead_id:** `ast-sgrep-gauntlet-remediation-program-1vhy.5`

### `lsp-navigation-not-full-cli`

- **target_workload:** LSP command set
- **evidence_artifact_paths:** `docs/validation/surface-parity.md`
- **retry_condition_predicate:** Retry condition not applicable -- the gain is structural, not numerical. LSP is an IDE navigation surface by contract.
- **bead_id:** (none)

### `compact-drops-provenance`

- **target_workload:** `--format compact`
- **evidence_artifact_paths:** `docs/validation/compact-output.md`, `DISC-compact-drops-provenance`
- **retry_condition_predicate:** Retry condition not applicable -- the gain is structural, not numerical. Compact is a token budget, not native JSON identity.
- **bead_id:** (none)

### `pattern-rewrites-not-in-product`

- **target_workload:** ast-grep YAML rules / rewrites
- **evidence_artifact_paths:** `docs/structural-patterns.md`, `docs/comparison.md`
- **retry_condition_predicate:** Reconsider only inside the broader rewrite/codemod product (not this indexer). Use standalone ast-grep; do not silently delegate.
- **bead_id:** (none)

### `dual-banner-process-cli-mcp`

- **target_workload:** one-shot CLI fusion vs MCP channel tools (two process models)
- **evidence_artifact_paths:** `docs/mcp.md`, `docs/validation/surface-parity.md`
- **retry_condition_predicate:** Reconsider only inside the broader Code Mode XOR MCP process redesign. Dual process is intentional; not a missing CLI clone.
- **bead_id:** (none)

### `ivf-ann-below-threshold`

- **target_workload:** semantic ANN on small corpora
- **evidence_artifact_paths:** `docs/validation/semantic-ivf-mmap.md`, `DISC-ivf-adaptive-threshold`
- **retry_condition_predicate:** Retry only if this workload class exhibits measurable `chunk_count` above the adaptive IVF threshold on the fixture under test.
- **bead_id:** `ast-sgrep-ho-ivf-residual-ho-20260807-hoy3.4`

### `extraction-presence-not-dump-golden`

- **target_workload:** lang extraction dumps
- **evidence_artifact_paths:** `DISC-extraction-presence-only`
- **retry_condition_predicate:** Blocked until extraction dump goldens land; track as `ast-sgrep-golden-artifacts-program-nz7i.4`.
- **bead_id:** `ast-sgrep-golden-artifacts-program-nz7i.4`

## Retired

_(none)_

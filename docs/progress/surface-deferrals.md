# Surface deferrals

Campaign ledger for surfaces explicitly excluded, partial, or intentionally
divergent. WP5 consumes this file for FeatureUniverse honesty.

Skill headers: gauntlet WP3 / K-3. Predicate forms: `docs/progress/README.md`.
Product parity table: `docs/validation/surface-parity.md`.
DISC register: `docs/validation/DISCREPANCIES.md`.

**Closed:** HTTP embed clients removed 2026-08-14 (product decision: native/in-process only).

## Closed

### `http-cloud-embed-removed`

- **date:** 2026-08-14
- **candidate_name:** `http-cloud-embed-removed`
- **target_workload:** OpenAI-compatible HTTP embed client (`--cloud-embed`, `ASGREP_EMBED_API_KEY`)
- **files_touched:** `crates/ast-sgrep-embed`, CLI/LSP/MCP flags, capabilities golden, semantic-search docs
- **correctness_proof:** not-measured (product removal, not a quality experiment)
- **evidence_artifact_paths:** `docs/semantic-search.md`, `docs/env-trust.md`, this ledger
- **baseline_configuration:** pointer-only
- **candidate_configuration:** pointer-only
- **retry_condition_predicate:** Not worth retrying as a standalone HTTP embed client. Reconsider only inside a broader hosted-model product that is explicitly not ast-sgrep's default path.
- **bead_id:** (none -- withdrawn with `lbx1.1`)

### `http-ollama-embed-removed`

- **date:** 2026-08-14
- **candidate_name:** `http-ollama-embed-removed`
- **target_workload:** Ollama HTTP embed client (`--ollama-embed`, `ASGREP_OLLAMA_URL`)
- **files_touched:** `crates/ast-sgrep-embed`, CLI/LSP/MCP flags, capabilities golden, semantic-search docs
- **correctness_proof:** not-measured (product removal, not a quality experiment)
- **evidence_artifact_paths:** `docs/semantic-search.md`, `docs/env-trust.md`, this ledger
- **baseline_configuration:** pointer-only
- **candidate_configuration:** pointer-only
- **retry_condition_predicate:** Not worth retrying as a standalone HTTP embed client. In-process ONNX neural is the only non-hashed vector path.
- **bead_id:** (none -- withdrawn with `lbx1.2`)

## Open (pointer imports)

### `cass-unavailable-http-embed-strip-2026-08-14`

- **target_workload:** 60-day cass failure-term mine before surface-affecting embed changes
- **evidence_artifact_paths:** this ledger
- **retry_condition_predicate:** Blocked until `cass` is on PATH; re-run the 60-day mine for `rejected|reverted|cloud-embed|ollama|keep gate` before resurrecting any HTTP embed client.
- **bead_id:** (none)

### `mcp-no-auto-fusion`

- **target_workload:** MCP vs CLI hybrid
- **evidence_artifact_paths:** `docs/validation/surface-parity.md`, `DISC-mcp-not-full-suite`
- **retry_condition_predicate:** Reconsider only inside the broader MCP hybrid-fusion redesign. Status in WP5 matrix is `excluded` (not missing). Track as `ast-sgrep-gauntlet-remediation-program-1vhy.5`.
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

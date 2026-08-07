# PASS8 — soft-skip / vacuous-green audit (mock-free e2e)

**Date:** 2026-08-07  
**Scope:** `crates/**/tests/**`, testkit helpers used by surface parity  
**Constraint:** real IndexStore / real process; no production network; hashed embed for parity

## Fixed this pass

| Item | Location | Fix |
|------|----------|-----|
| Vacuous green `assert!(is_empty() \|\| true)` | `crates/ast-sgrep-cli/tests/no_embed_hit_key_parity.rs` (both-error table) | Hard assert: whitespace query → zero hits on core **and** CLI |
| Embed-on multi-surface hit-key parity missing (only `--no-embed`) | same file + testkit helpers | New test `surface_equivalence_embed_on_hit_keys` (lbx1.13); helpers take `use_embed: bool` |
| Helpers forced `use_embed: false` / LSP `no_embed: true` | `crates/ast-sgrep-testkit/src/index.rs`, `lsp.rs` | Parameterized; callers must opt in for embed-on parity |

### Evidence

```text
cargo test -p ast-sgrep-cli --test no_embed_hit_key_parity -- --nocapture
# 3 passed: multi_mode (no-embed), embed_on, both_error
```

### Bead closed

- `ast-sgrep-mock-free-e2e-gaps-lbx1.13` — embed-kind hit-key parity across CLI/core/LSP with embed ON (hashed). Hard fail if any surface lacks `kind=embed`.

## Remaining soft-skip / zero-run / ignore offenders

These are **not** invented greens fixed this pass; inventory for next loops.

### Intentional `#[ignore]` (default CI never runs body)

| File:line | Test | Gate / note | Owning bead |
|-----------|------|-------------|-------------|
| `crates/ast-sgrep-core/tests/e2e_smoke.rs:157` | `archived_pi_fixture_graph_modes_match_indexed_keys` | `ASGREP_REAL_PI_FIXTURE` archive | lbx1.5 |
| `crates/ast-sgrep-core/tests/semantic_ivf_roundtrip.rs:321` | `adaptive_ivf_tradeoff_at_2048_and_10000_vectors` | release-mode ANN scale | lbx1.7 |
| `crates/ast-sgrep-core/tests/store_delete.rs:156` | `re_upsert_many_files_is_linear` | timing quarantine (not correctness) | none / keep ignore |

### Soft budget skip (debug green without latency gate)

| File:line | Behavior | Severity |
|-----------|----------|----------|
| `crates/ast-sgrep-core/tests/sub1ms.rs:41-43` | `if cfg!(debug_assertions) { eprintln!(…); return; }` — skips budget assert in debug | Medium: still asserts work_units > 0; full gate is release-only by design |

### Helper early-returns (not test soft-skips)

| File:line | Note |
|-----------|------|
| `crates/ast-sgrep-core/tests/metamorphic.rs:117-118,149-150` | `ensure_nonzero_rows` / `inject_near_query` guards on empty dim — test helpers, not skip-green |

### Widespread `use_embed: false` in unit/integration tests

~40 sites under `crates/**/tests/**` force `use_embed: false` for isolation of lexical/graph/cache paths. **Not automatically vacuous** if embed is covered elsewhere; residual risk where the test *claims* semantic/ranking coverage without embed:

| Area | Status after PASS8 |
|------|--------------------|
| Ranking oracle / vwga | **Hardened** (lbx1.6): `use_embed: true`, empty embed must_include fails |
| CLI agent/search embed default | **Hardened** (lbx1.4): machine_contracts embed-on |
| Surface hit-key parity embed-on | **Hardened** (lbx1.13, this pass) |
| Codemode session tests | Still `use_embed: false` → **lbx1.11** |
| MCP semantic non-hashed | **lbx1.10** |
| Ollama/cloud/neural HTTP/model | **lbx1.1 / lbx1.2 / lbx1.3** |

### Profile path only (not soft-skip)

| File:line | Note |
|-----------|------|
| `crates/ast-sgrep-mcp/tests/protocol.rs:10` | `cfg!(debug_assertions)` chooses debug vs release binary path for `asgrep-mcp` when `CARGO_BIN_EXE_*` missing — not a test skip |

### Previously fixed (do not re-flag as open)

| Bead | What was killed |
|------|-----------------|
| lbx1.4 | CLI embed default ON contracts |
| lbx1.6 | ranking soft-skip / empty embed must_include |
| c0d8141 | downstream_correctness soft-skip |
| lbx1.13 | embed-kind multi-surface parity with embed ON |

## Open mock-free children (epic `lbx1` stays open)

| ID | P | Title |
|----|---|-------|
| lbx1.1 | P1 | Ollama embed HTTP contract (loopback/live) |
| lbx1.2 | P1 | Cloud embed HTTP contract (loopback/live+SSRF) |
| lbx1.3 | P1 | Neural embed feature e2e (real model load) |
| lbx1.5 | P1 | Large-corpus / archived Pi graph e2e in CI |
| lbx1.7 | P2 | ANN IVF scale quality gate |
| lbx1.8 | P2 | CLI watch daemon e2e |
| lbx1.9 | P2 | external ast-grep opt-in spawn e2e |
| lbx1.10 | P2 | MCP semantic tool non-hashed backend |
| lbx1.11 | P2 | codemode session embed-on path |
| lbx1.12 | P2 | LSP stdio JSON-RPC protocol e2e |
| **lbx1** | P2 | **epic** — do not close until children done |

## Recommended next (easy Score≥8 residual)

1. **lbx1.11** — flip codemode session tests to hashed embed-on (mirror lbx1.13 pattern).  
2. **lbx1.1** — loopback HTTP + real `ureq` (`--features cloud`) using `testkit::safety` (`is_loopback_host`, `require_real_ready`); no production network.  
3. **lbx1.12** — real stdio JSON-RPC process to `asgrep-lsp` (protocol bytes, not only in-process `LspBackend`).

## Files touched this pass

- `crates/ast-sgrep-testkit/src/index.rs` — `core_search_hit_keys(..., use_embed)`
- `crates/ast-sgrep-testkit/src/lsp.rs` — `lsp_search_hit_keys(..., use_embed)`
- `crates/ast-sgrep-cli/tests/no_embed_hit_key_parity.rs` — embed-on parity + vacuous assert kill
- `tests/artifacts/mock-free-audit/PASS8_SOFT_SKIP_AUDIT.md` — this note

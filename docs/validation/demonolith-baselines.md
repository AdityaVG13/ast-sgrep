# Demonolith Phase 3 baselines — ast-sgrep-wt-demonolith

Run: `2026-08-13-ast-sgrep-wt-demonolith-1` · Phase 3 dynamic baseline only (no extractions, no seam probes).

**BASELINE_SHA:** `9815d62027ae40beb72e0d219e752d86169df286`  
Workspace artifacts: `../ast-sgrep-wt-demonolith__demonolith_workspace/phase3_baselines.md`, `baseline_tests.json`, `api_snapshot_before.txt`, `phase3_raw/`.

Honesty rule: numbers below were measured this pass on `rch` worker **spark-1672**, or marked **SKIPPED** with rationale. No invented metrics; no copies from `docs/benchmarks/results`.

## Test counts

Degradation vs full ×3: **1 full workspace suite + 2 targeted** monolith-adjacent reps (`ast-sgrep-core --lib --tests` + `ast-sgrep-cli --test machine_contracts`). Full ×3 deferred for wall-time (~297s/full rep measured).

| rep | scope | passed | failed | skipped | wall (s) | exit |
|---|---|---:|---:|---:|---:|---:|
| 1 | full `cargo test --workspace --no-fail-fast` | 488 | 0 | 4 | 297 | 0 |
| 2 | targeted | 314 | 0 | 3 | 307 | 0 |
| 3 | targeted | 314 | 0 | 3 | 289 | 0 |

- Gate modal (full suite): **488 pass / 0 fail / 4 skip**.
- No PRE-EXISTING FLAKY (no pass↔fail flips within a shared scope).
- Stable ignored: `archived_pi_fixture_graph_modes_match_indexed_keys`, `adaptive_ivf_tradeoff_at_2048_and_10000_vectors`, `re_upsert_many_files_is_linear`, plus full-suite-only ignored doctest `ast-sgrep-codemode` lib.rs:29.
- `--offline` failed on spark (missing crates.io cache); suite re-run without `--offline`.

## API snapshot

| path | content |
|---|---|
| workspace `api_snapshot_before.txt` | `cargo +nightly public-api --simplified` for `ast-sgrep-core` + `ast-sgrep-mcp` (1973 lines composed) |
| `phase3_raw/api_ast-sgrep-core.txt` | core-only snapshot |
| `phase3_raw/api_ast-sgrep-mcp.txt` | mcp-only snapshot (12 lines) |

## Goldens

Existing fixtures hashed (not regenerated) under workspace `phase3_raw/goldens/checksums.txt` (`tests/lang/fixtures/extract` ×13 + `tests/cli/fixtures` ×3).

## SKIPPED this pass

| item | rationale |
|---|---|
| Compile-resource profile (`compile-mem-profile.sh`) | Time after suite+API; no invented RSS |
| Criterion benches (`crates/ast-sgrep-core/benches`) | Host load ~3–5; no quiet SAME-MACHINE window; >15 min risk |
| Coverage / Appendix B | Deferred (pass 2 already skipped instrumented suite) |
| Lint counts (clippy / tsc) | Time; suite+API prioritized |

## Notes

- Product code untouched at analysis HEAD (docs-only commits on `9815d62`; lineage still `accb010`).
- Pass 1 census / pass 2 seams unchanged; this pass only establishes behavior+API ground truth.

# Proof pack (`c1i2`)

Minimal reproducible gates for ranking and fail-closed honesty. Runnable gate:

```bash
bash scripts/run-proof-pack.sh
```

That script always writes `tests/artifacts/compliance/COMPLIANCE_REPORT.md`
(gitignored). Exit non-zero if an **executed** proof-pack suite failed.
Registry-only (no cargo):

```bash
python3 scripts/generate-compliance-report.py --registry-only --tier proof-pack
```

Manual cargo filters (same suites as the registry `proof-pack` tier):

```bash
export PATH="/usr/local/cargo/bin:$PATH"
bash scripts/verify-forbid-soundness
cargo test -p ast-sgrep-core --test ranking_oracle -j1 -- --test-threads=1
cargo test -p ast-sgrep-core --test graph_oracle -j1 -- --test-threads=1
cargo test -p ast-sgrep-cli --test machine_contracts -j1 -- --test-threads=1
cargo test -p ast-sgrep-mcp --test protocol -j1 -- --test-threads=1
cargo test -p ast-sgrep-embed --lib math:: -j1 -- --test-threads=1
```

Registry: [`tests/conformance/registry.toml`](../../tests/conformance/registry.toml).
Non-claims: [`DISCREPANCIES.md`](DISCREPANCIES.md). Coverage skeleton:
[`COVERAGE.md`](COVERAGE.md). Verdicts: [`conformance-verdicts.md`](conformance-verdicts.md).
Golden SOP / PR CI: [`golden-files.md`](golden-files.md) (`nz7i.5`).

Score in the report is Pass / Fail / Not-run only. Not-run is not Pass. No MUST%.

## CI tiers (honesty)

| Tier | What | When |
|---|---|---|
| T0 | `verify-forbid-soundness` + `cargo check --workspace` | Local default bar |
| T1 | Proof-pack (`scripts/run-proof-pack.sh`) | Local / merge honesty |
| T2 | GitHub `pull_request` jobs already in `ci.yml` (ubuntu `test`, clippy, fmt, …) | PRs. **Does not** regenerate this report |
| T3 | `workflow_dispatch` release matrix (`build-and-test`, Windows, fuzz) | Actions tab |
| T4 | Human `scripts/local-release-gate.sh` (crates) and Pi `release-acceptance.mjs` (npm) | Release prep. Distinct tools |

Until a dedicated report job exists, compliance reports are **local or dispatch**,
not "on every PR". Golden compare-only PR triggers stay in
[`golden-files.md`](golden-files.md) (`nz7i.5`). Bounded fuzz stays the
`bounded-fuzz` `workflow_dispatch` job (`b8q3.1`). This emitter does not re-own
those.

Proof-pack `machine_contracts` skips
`bench_json_emits_cv_pct_and_skips_vacuous_ast_grep_speedup` (pre-existing
non-zero vs expected 0). That skip is not a Pass for the bench case.

## Artifacts

- `tests/fixtures/ranking/cases.json`
- `docs/validation/feature-universe.md`
- `docs/validation/engine-identity.md`
- `docs/validation/negative-ledgers.md`
- `docs/validation/DISCREPANCIES.md`
- `docs/validation/COVERAGE.md`
- `docs/validation/conformance-verdicts.md`
- `docs/progress/README.md`
- `docs/validation/oracle-dispatch.md`
- `docs/validation/residual-leaf-shares-post-T1R.md`
- `docs/validation/stage-timers-post-T1R.md`
- `docs/validation/ann-threshold-cliff-post-T1R.md`
- `docs/validation/t1r-sidecar-bit-identity.md`
- `docs/QUERY_GRAMMAR.md`
- `docs/contracts/oracle_dispatch.toml`
- `EPIC_EVIDENCE.md`

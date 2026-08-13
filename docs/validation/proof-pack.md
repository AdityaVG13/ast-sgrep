# Proof pack (`c1i2`)

Minimal reproducible gates for ranking and fail-closed honesty on this PR:

```bash
export PATH="/usr/local/cargo/bin:$PATH"
bash scripts/verify-forbid-soundness
cargo test -p ast-sgrep-core --test ranking_oracle -j1 -- --test-threads=1
cargo test -p ast-sgrep-core --test graph_oracle -j1 -- --test-threads=1
cargo test -p ast-sgrep-cli --test machine_contracts -j1 -- --test-threads=1
cargo test -p ast-sgrep-mcp --test protocol -j1 -- --test-threads=1
cargo test -p ast-sgrep-embed --lib math:: -j1 -- --test-threads=1
```

Artifacts:

- `tests/fixtures/ranking/cases.json`
- `docs/validation/feature-universe.md`
- `docs/validation/engine-identity.md`
- `docs/validation/negative-ledgers.md`
- `docs/validation/DISCREPANCIES.md`
- `docs/validation/COVERAGE.md`
- `docs/validation/conformance-verdicts.md`
- `docs/progress/README.md`
- `docs/validation/oracle-dispatch.md`
- `docs/contracts/oracle_dispatch.toml`
- `EPIC_EVIDENCE.md`

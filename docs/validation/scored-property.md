# Scored / NaN property notes (`g799`)

- Unit + property-style checks live in `ast-sgrep-embed` `math::contract_tests`
  and `math::property_tests`.
- Miri / TSim/TSan full-matrix runs are **skipped in CI** (cost); forbid-soundness
  and focused cargo tests are the merge bar. Optional local:

```bash
# Requires nightly + miri; not part of PR CI.
cargo +nightly miri test -p ast-sgrep-embed --lib math:: || true
```

NaN residuals must never enter `Scored` or poison ANN normalization.

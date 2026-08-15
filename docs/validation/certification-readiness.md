# Certification readiness (1vhy.6)

Greenfield hybrid search is **not** `strict-conformant-release.v1`.

- Checklist: [multi-ref-checklist.md](multi-ref-checklist.md)
- Score seed: [`tests/conformance/parity_score.json`](../../tests/conformance/parity_score.json)
- Weights: [`docs/contracts/parity_score_contract.toml`](../contracts/parity_score_contract.toml)
- Emitter: `python3 scripts/generate-parity-score.py`

## Forbidden-victory

No single pillar may be marked done or used as a release gate while another
pillar in the same evidence window is red. Keep-gate latency is never a
correctness oracle. Partial is not present. Excluded is not missing. Not-run
is not Pass.

## Lower bound vs point estimate

Quote **`lower_bound`**, not `optimistic_present_ratio`. The optimistic ratio
counts matrix `present` cells with partial truncated to 0; it is **not**
certified. Treat Not-run, Ignore, `UNREPRODUCIBLE` metrics, and `latency_only`
as 0 toward the lower bound.

Until an evidence window maps executed correctness Passes onto features,
**`lower_bound` stays 0** and **`certified` stays false**.

## `release_certificate.json`

Do **not** emit this file until `certified` is true and H1–H13 are non-red.
Audit markdown is not a certificate. A weaker ship tag, if ever needed, is
`provisional` **with a deviations list** -- still not `strict-conformant-release.v1`.

## Truncate policy

See `truncate_policy` in `parity_score.json` and `[forbidden_victory]` in the
weights contract.

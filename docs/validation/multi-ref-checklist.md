# Multi-ref certification checklist (1vhy.6)

Evidence window: docs in this tree at commit of `parity_score.json`.
Statuses are **red** or **yellow** only. Do not paint green from cargo-green,
audit markdown, or present-count in the surface matrix.

**Forbidden-victory:** no single pillar may be marked done or used as a release
gate while another pillar in the same evidence window is red.

## H1–H14

| ID | Pillar | Input owner | Band | Evidence |
|---|---|---|---|---|
| H1 | Keep-gate / history | WP1 | yellow | `.bench-history/`; `latency_only` never correctness ([oracle-dispatch.md](oracle-dispatch.md)) |
| H2 | Ledger unreproducible policy | WP2 | yellow | [baselines.md](../../benchmarks/results/baselines.md) mix; canonical MRR still `UNREPRODUCIBLE` |
| H3 | Oracle channel map | WP4 | yellow | [oracle-dispatch.md](oracle-dispatch.md); Pattern-1 / jell deferred |
| H4 | Feature × host matrix | WP5 | yellow | [supported_surface_matrix.toml](../contracts/supported_surface_matrix.toml); `min_verification_pct = unset` |
| H5 | Compliance point suites | ghiw.5 | yellow | [proof-pack.md](proof-pack.md); reports local/dispatch, Not-run is not Pass |
| H6 | Golden freeze | nz7i | yellow | [golden-files.md](golden-files.md); PR compare-only |
| H7 | Fuzz floor | b8q3 | yellow | `bounded-fuzz` is `workflow_dispatch`, not every PR |
| H8 | Conformal lower bound | WP6 (this) | red | [parity_score.json](../../tests/conformance/parity_score.json) `certified=false`, `lower_bound=0` |
| H9 | Multi-ref bundle (8 classes) | WP6 | red | this table; 0/8 green |
| H10 | Negative ledgers | WP2/WP3 | yellow | [negative-ledgers.md](negative-ledgers.md), [docs/progress/](../progress/README.md) |
| H11 | DISC registry | ghiw.1 | yellow | [DISCREPANCIES.md](DISCREPANCIES.md) |
| H12 | Live-embed / mock-free P1 | lbx1 | red | lbx1.1–.3,.5 not run here; do not fake |
| H13 | UNREPRODUCIBLE MRR not cert | WP2+WP6 | yellow | AGENTS.md + this file; fingerprints stay historical |
| H14 | `release_certificate.json` | WP6 | red | **refused** until H1–H13 are non-red and `certified=true` |

## Cert inputs (not re-implemented here)

| Program | What this WP consumes |
|---|---|
| WP1 | keep-gate history files |
| WP2 | unreproducible / negative ledger policy |
| WP4 | channel weights via oracles (`gate_class`) |
| WP5 | feature matrix + [parity_score_contract.toml](../contracts/parity_score_contract.toml) weights |
| ghiw.5 | Pass/Fail/Not-run matrix |
| nz7i | golden compare-only |
| b8q3 | fuzz floor |

lbx1 is a floor input: missing live-embed stays red, not excluded-as-pass.

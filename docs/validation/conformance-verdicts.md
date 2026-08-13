# Conformance verdicts

Default is **Fail** (panic / hard assert). Soft-skip is not a Pass.

| Verdict | When | How |
|---|---|---|
| **Fail** | Contract broken | `assert!` / `panic!`. Default. |
| **Pass** | Asserted invariant held | Test returned. |
| **Ignore** | Cannot run here | `#[ignore]` or env gate **with a reason string** and a DISC or COVERAGE link. |
| **ExpectedFailure / XFAIL** | Known intentional divergence | Only for a **registered** DISC id in `DISCREPANCIES.md`. v0 is documentation + comments; no enum required in every suite. |
| **Not-run** | Harness never executed the case | Must not be reported as Pass (future ghiw.5 emitter). |

Forbid silent green on empty optional channels (embed off, ANN below
threshold, missing ast-grep binary). Those are Not-run or DISC, not Pass.

## Pilot mapping

| Suite | Maps to |
|---|---|
| `tests/core/ranking_oracle.rs` | Fail = missing `must_include`. Soft oracle = `DISC-ranking-soft-oracle`. |
| `tests/cli/machine_contracts.rs` | Fail = envelope/shape mismatch. Capabilities dump uses `assert_golden_json_at`. |
| `ast_sgrep_testkit::TestVerdict` | Optional type for new table-driven rows (`disc_id` on Ignore / XFAIL). |

Do not rewrite existing suites into a megatrait in this bead.

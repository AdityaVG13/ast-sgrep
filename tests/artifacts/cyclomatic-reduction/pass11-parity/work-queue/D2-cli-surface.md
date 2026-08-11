# Work packet D2 — CLI surface residual

| Field | Value |
|---|---|
| id | D2 |
| priority | P2 |
| status | open / **Defer** |
| risk | medium |
| product_area | `crates/ast-sgrep-cli` |
| package_ΣCC_class | **~630** after pass 9 (−2); do not raise |

## Goal

Further accidental-structure cuts on CLI without raising package (or touched-file) ΣCC. Prefer shared collapse over ladder extracts.

## Exact targets (census pass 10)

| Function | CC | File |
|---|---:|---|
| `run_bench_suite` | 24 | `crates/ast-sgrep-cli/src/bench.rs` (~278) |
| `run_bench_batch` | 16 | `crates/ast-sgrep-cli/src/bench.rs` (~460) |
| `run_process` | 16 | `crates/ast-sgrep-cli/src/lib.rs` |
| `run_chain` | 14 | `crates/ast-sgrep-cli/src/search_cmd.rs` |
| `run_watch` | 14 | `crates/ast-sgrep-cli/src/watch.rs` |
| `clap_catalog` / agent helpers | 13 | `crates/ast-sgrep-cli/src/agent.rs` |
| `update_bench_history` | 13 | `bench.rs` |
| `run_eval` | 12 | `crates/ast-sgrep-cli/src/eval.rs` |
| `doctor_triage_json` / `supervise` | 11 | agent.rs / supervisor.rs |

## History

| Pass | Action | Bill |
|---|---|---|
| 5 | lookup / argv-related CLI touch (related surface) | see pass5 notes |
| 9 | shared collapse bench ratchet + human print; suite 29→24; run_bench 15→9; run_search 13→10 | package **−2** |
| 9 Refuse | `run_process` extract **+3**; `measure_suite_case` pure extract **+3** | not kept |
| 10 | re-measure | campaign Σ includes cli hotspots 10 |

## Classification

| Cluster | Class | Notes |
|---|---|---|
| bench suite/batch loops | extractable / case-loop identity | Much of CC is case enumeration — Ashby Keep-lean unless duplicate trees collapse |
| run_process error ladder | accidental + essential codes | Pure extract refused |
| chain/watch | accidental gates | Only if identical predicates across both |
| clap/agent catalog | domain-ish | Prefer Keep |

## Allowed techniques

1. Additional **shared collapse** inside bench family (suite/batch/history) if duplicate decision trees remain after pass 9.
2. Predicate consolidation for chain/watch **if** gates are truly identical.
3. Dead branch removal with test proof only.

## Forbidden

- Clap/agent error-ladder pure extract without decision elimination.
- Machine envelope / public flag / JSON field changes.
- Re-attempt `run_process` extract without a measured −ΣCC plan.
- Changing bench ratchet semantics (`ASGREP_BENCH_RATCHET`, history keys).

## Procedure

1. Diff `run_bench_suite` vs `run_bench_batch` for duplicated conditions (ratchet, skip reason, envelope fields).
2. If only case-loop identity remains → **Keep residual**, stop.
3. Else apply one shared helper that removes ≥1 decision from ≥2 sites.
4. Re-measure:
   ```bash
   python …/measure_complexity.py crates/ast-sgrep-cli/src --threshold 10
   ```
   Package total_cc must not increase vs post-pass-9 class (**630** class; if re-measure differs, gate on **non-increase vs your pre-edit measure** of same scope).

## Verify (acceptance)

```bash
cargo check -p ast-sgrep-cli

cargo test -p ast-sgrep-cli --test machine_contracts --test cli_smoke --lib
# expect: lib 10, smoke 2, machine_contracts 20 (or ≥ prior green set)

# Minimum bench pins (names from pass 9 / pass 11):
#   bench_suite_json_is_single_envelope_even_on_failure
#   bench_json_emits_cv_pct_and_skips_vacuous_ast_grep_speedup
#   chain_eval_and_bench_successes_use_machine_envelope
```

Characterization pins (pass 9):

- `enforce_bench_ratchet`: same conditions (history ∧ env ∧ ratchet_ok)
- `print_ast_grep_human`: field access order preserved
- Suite JSON skip path: `index_skipped` / `index_ms: 0` semantics

## Resolve default

Defer residual case-loop identity in `run_bench_suite`. Keep domain packing that is requisite for machine contracts.

## Stop / escalate

ΣCC up → Refuse. Public CLI change → Refuse without auth.

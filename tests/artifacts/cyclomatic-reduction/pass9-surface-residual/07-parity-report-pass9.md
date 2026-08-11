# 07 — Parity report (Pass 9)

## Scope

Product edits only:

- `crates/ast-sgrep-cli/src/bench.rs`
- `crates/ast-sgrep-cli/src/search_cmd.rs`

Launcher / lib / mcp / lang / lsp: **no final product edit** (refuse or Keep).

## Level 1 — Compile

```text
cargo check -p ast-sgrep-cli
Finished dev profile … (ok)
```

## Level 2–3 — Existing + targeted suite

```text
cargo test -p ast-sgrep-cli --test machine_contracts bench
  bench_suite_json_is_single_envelope_even_on_failure … ok
  bench_json_emits_cv_pct_and_skips_vacuous_ast_grep_speedup … ok
  chain_eval_and_bench_successes_use_machine_envelope … ok
  3 passed

cargo test -p ast-sgrep-cli --test cli_smoke
  2 passed

cargo test -p ast-sgrep-cli --lib
  10 passed
```

Launcher floor (unchanged):

```text
node --test test/npm-native-packages.test.mjs test/binary-env-alias.test.mjs
  13 passed
```

## Level 4 — Differential / characterization

| Transform | Characterization |
|---|---|
| `enforce_bench_ratchet` | Same conditions (`history` present ∧ `ASGREP_BENCH_RATCHET=1` ∧ `ratchet_ok == false`); messages still `bench ratchet failed for suite …` / `for query …` |
| `print_ast_grep_human` | Same field access order (`compared` → pattern + ms → optional speedup); suite indent vs single-query wording preserved; skip-reason only on non-suite path |
| Suite JSON skip path | `add_index_json(None, 0.0)` matches prior `index_skipped` / `index_ms: 0` / `files_indexed: null`; indexed path still only sets `files_indexed` |
| `uses_semantic_channel` | Algebraically identical to `semantic \|\| cli.active_tuning().semantic_only` at three former call sites |

No public CLI flag or machine-schema field changes.

## Level 5 — Analyzer re-run

| Scope | ΣCC before | ΣCC after | Δ |
|---|---:|---:|---:|
| `bench.rs` + `search_cmd.rs` | 151 | 149 | **-2** |
| `ast-sgrep-cli` package | 632 | 630 | **-2** |

**Displacement check: pass.**

## Verdict

**Differential parity: pass** (suite green + structure-preserving shared collapse).

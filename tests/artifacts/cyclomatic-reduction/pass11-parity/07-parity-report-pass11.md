# 07 — Parity report (Pass 11)

## Scope

**Measure / verify only.** Zero product edits. Re-run targeted floors covering waves 3–9.

Full matrix: `parity-matrix.md`.

## Level 1 — Compile

```text
cargo check -p ast-sgrep-core -p ast-sgrep-cli -p ast-sgrep-mcp
# Finished `dev` profile … (ok)  EXIT=0
```

## Level 2–3 — Existing + targeted suite

### Extension (passes 4–7)

```text
cd packages/pi/extension && npm test
# tests 88 · pass 88 · fail 0
```

Covers: code-mode, codemode dispatch/argvFor, commands, runtime (ensureFresh / index upgrade / classified errors), security, session-pool, skill-workflow, tools (parseSearchHit / edit dirty).

### Launcher floor (pass 3 / 9)

```text
cd packages/pi/launcher
node --test test/npm-native-packages.test.mjs test/binary-env-alias.test.mjs
# tests 13 · pass 13 · fail 0
```

Pins: host resolve, PATH fallback codes, checksum / empty executable, pack alias execution.

### CLI (pass 5 / 9)

```text
cargo test -p ast-sgrep-cli --test machine_contracts --test cli_smoke --lib
# lib 10 · cli_smoke 2 · machine_contracts 20 — all ok
```

Includes: `bench_suite_json_is_single_envelope_even_on_failure`, `bench_json_emits_cv_pct…`, `chain_eval_and_bench_successes_use_machine_envelope`.

### Core (pass 8)

```text
cargo test -p ast-sgrep-core \
  --test parity --test e2e_smoke --test regex_budget --test semantic_ivf_roundtrip \
  --test search_correctness_epics --test code_prose_fields
# parity 3 · e2e 5 (+1 ign) · regex 1 · ivf 8 (+1 ign) · epics 10 · prose 5
```

## Level 4 — Differential

No new refactors. Characterization remains structure-preserving extracts/collapses from waves 3–9 (see prior `07-parity-report-pass*.md` and pass notes). Living suite re-green is the campaign-level re-proof after multipass history.

## Level 5 — Analyzer

Not re-run (product frozen at pass-10 bill). Canonical:

| Metric | Baseline | Bill (pass 10) |
|---|---:|---:|
| ΣCC | 6022 | **5994** |
| Max CC | 31 | **26** |
| Hotspots >10 | 91 | **83** |

**Displacement check:** pass (from pass 10; unchanged).

## Pre-existing red (out of campaign fix scope)

| Test | Why not campaign |
|---|---|
| `extension-package.test.mjs` inventory | Pack allowlist vs dist (sandbox→runner + extra modules) |
| `asgrep-search-mode-matrix` `keyword` | Schema/docs mode name drift |

## Verdict

**Differential parity: pass** for all wave-touched joint-allowed floors.  
**Campaign-caused regressions: none.**

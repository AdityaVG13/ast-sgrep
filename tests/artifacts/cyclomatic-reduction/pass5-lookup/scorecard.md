# Complexity scorecard — Pass 5 (lookup_table)

Analyzer: lizard via `measure_complexity.py`
Scope files: `dispatch.ts` + `index.ts` + `literal.rs`

## Per-target CC

| Function | File | Before | After | Δ | Technique |
|---|---|---:|---:|---:|---|
| `argvFor` | dispatch.ts | 22 | **11** | −11 | lookup_table (+ `argStr` CC3) |
| `searchToolCall` | index.ts | 17 | **11** | −6 | lookup_table |
| `literal_sql` | literal.rs | 18 | **16** | −2 | lookup_table (`literal_sql_template` CC1) |

## File ΣCC (Bill)

| File | Before | After | Δ |
|---|---:|---:|---:|
| `packages/pi/extension/src/codemode/dispatch.ts` | 74 | **66** | **−8** |
| `packages/pi/extension/src/index.ts` | 146 | **140** | **−6** |
| `crates/ast-sgrep-core/src/search/passes/literal.rs` | 38 | **37** | **−1** |
| **Touched scope Σ** | **258** | **243** | **−15** |

**Displacement check:** justified — data tables are decision-free; interpreters hold remaining form variety. No helper dump that raised ΣCC.

## Residual (still > hard ceiling 10)

| Function | CC | Note |
|---|---:|---|
| argvFor | 11 | form interpreter residual; under-ish (1 over hard 10) |
| searchToolCall | 11 | form interpreter residual |
| literal_sql | 16 | word_mode/context loop (essential) dominates |
| summarizeCodemode | 15 | not in pass-5 batch |
| literal_trigram | 12 | not in pass-5 batch |
| resolveHost / resolveBinary / resolveCodemodeAddon | 26/17/18 | pass-3 residual launcher |

## CUT_BRANCHES_RESULT

`partial` — wave-5 planned trio transformed; ΣCC down; residuals deferred to pass 6+.

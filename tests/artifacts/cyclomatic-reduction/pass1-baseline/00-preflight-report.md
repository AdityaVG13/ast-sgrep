# Cyclomatic Reduction Preflight Report

Run ID: `2026-08-10T235424Z-baseline`
Target root: `/Users/aditya/Developer/ast-sgrep`
Mode: `baseline`
Date: 2026-08-10T23:54:30Z

## Joint policy

Read before any suite or transform: AGENTS.md  
Never invent whole-workspace test commands. Prefer package/crate-scoped commands documented by the joint.

## Toolchain Checks

| Tool | Status | Version |
|---|---|---|
| uv | ok | uv 0.12.3 (Homebrew 2026-08-07 aarch64-apple-darwin) |
| lizard | ok | 1.23.0 |

## Suite

| Status | Command | Note |
|---|---|---|
| skipped | `none` | --skip-tests set. |

## Conclusion

`PASS`

| Gate | Result |
|---|---|
| Target root exists | yes |
| lizard available | yes |
| uv available (optional bootstrap) | yes |
| Suite (only if --test-command) | skipped |
| Mode | baseline |

### Mode expectations

| Mode | Preflight suite | Next agent step |
|---|---|---|
| smoke / baseline | optional / skipped by default | measure + ledger; no product edit |
| mutate | optional here; required targeted tests during transform | analysis cards + one technique + parity |


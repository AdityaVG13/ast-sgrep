# Baseline / freeze evidence ledger — Wave 2 Pass 1

Frozen revision: `62ee4b4595ad2433bd16b0ac14747dada612b4d6` · recorded `2026-08-12T16:24:44Z`  
Policy: **freeze + authorize only**. No product edits. No census. No workspace tests.

## Toolchain (spot)

| Command | Exit | Concise output |
|---|---:|---|
| `cargo --version` | 0 | `cargo 1.97.1 (c980f4866 2026-06-30)` |
| `rustc --version` | 0 | `rustc 1.97.1 (8bab26f4f 2026-07-14)` |
| `node --version` | 0 | `v24.14.1` |
| `npm --version` | 0 | `11.11.0` |
| `python3 --version` | 0 | `Python 3.14.6` |
| `zs --version` | 0 | `zs 1.3.0` |
| `which fszero-codemode` | 1 | **not found** — zerostack unavailable; shell/`rg` |

## Freeze identity commands

| Command | Exit | Notes |
|---|---:|---|
| `git rev-parse HEAD` | 0 | `62ee4b4595ad2433bd16b0ac14747dada612b4d6` |
| `git rev-parse --abbrev-ref HEAD` | 0 | `perf/software-optimization` |
| `git status -sb` / `--porcelain=v1` | 0 | dirty; 38 short lines; beads + Pi leftover noted |
| `git log -1 --oneline` | 0 | `62ee4b4 skill-loop pass 12/12: RCA absolute convergence seal (audit)` |
| read `.rotational-code-analysis/state.json` | 0 | prior iteration/residuals/coverage summarized; **not** re-baselined |

## Explicitly NOT run (V-STATE-IGNORE / mission scope)

| Command / action | Why deferred |
|---|---|
| `spin.py init` (re-snapshot) | would re-census; forbidden this pass |
| census / architecture re-pass | V-STATE-IGNORE |
| product source edits under `crates/` / `packages/` | next harden passes only |
| touch Pi `runtime.ts` / `index.ts` rg work | unrelated dirty; out of scope |
| `cargo test --workspace` | not freeze |

## Evidence kind

`git` + prior `state.json` (+ wave-1 artifact mirrors). Independent verification **n/a** (no high finding claimed this pass).
